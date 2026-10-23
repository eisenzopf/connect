//! A new immutable media origin after an exact observed credential rotation.
use super::*;
use connect_store::NativeCredentialRefreshPlan;
use rvoip_uctp::v2::auth_dispatch::VerifiedCredentialRefresh;

pub(crate) struct Invocation {
    pub policy: Arc<Policy>,
    pub plan: Arc<NativeCredentialRefreshPlan>,
    pub grant: Arc<crate::peer_authority::Grant>,
    pub refresh: Arc<VerifiedCredentialRefresh>,
    pub index: usize,
    pub next: Arc<Connection>,
}
struct Lease {
    owner: MediaOwner,
    invocation: Arc<Invocation>,
}
impl MediaAuthorityLease for Lease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        self.invocation.current()?;
        Ok(Box::new(self.owner.clone()))
    }
}
impl Clone for Invocation {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            plan: self.plan.clone(),
            grant: self.grant.clone(),
            refresh: self.refresh.clone(),
            index: self.index,
            next: self.next.clone(),
        }
    }
}
impl Invocation {
    fn current(&self) -> ControlResult<()> {
        if !self.plan.current() || !self.grant.proof.current() || !self.refresh.current() {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    pub(super) fn control(
        &self,
        policy: &Policy,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<MediaControl> {
        self.current()?;
        if !std::ptr::eq(policy, Arc::as_ptr(&self.policy)) {
            return Err(StreamControlError::Authority);
        }
        policy.scope(scope, &self.next)?;
        let target = &self.plan.targets()[self.index];
        match intent {
            MediaControlIntent::RebindCredential {
                previous,
                expected_credential_revision,
                expected_stream_revision,
            } if *expected_credential_revision == target.expected_credential_revision()
                && *expected_stream_revision == target.stream_revision()
                && rvoip_core::staged_streams::reliable_data::same_credential(
                    previous,
                    &self.refresh.previous().principal,
                ) =>
            {
                ()
            }
            _ => return Err(StreamControlError::Authority),
        }
        let owner = self.next.owner(target.view(), scope.principal, false)?;
        let owner = self
            .next
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)?;
        MediaControl::new(
            Box::new(Lease {
                owner,
                invocation: Arc::new(self.clone()),
            }),
            self.refresh.cancellation(),
            self.refresh.deadline(),
        )
    }
}
impl Policy {
    pub(crate) fn connections(&self) -> ControlResult<Vec<Arc<Connection>>> {
        Ok(self
            .entries
            .lock()
            .map_err(denied)?
            .values()
            .cloned()
            .collect())
    }
    pub(crate) fn successor(
        self: &Arc<Self>,
        old: &Arc<Connection>,
        origin: Arc<NativeMediaOrigin>,
        principal: AuthenticatedPrincipal,
    ) -> ControlResult<Arc<Connection>> {
        let installed = self.entry(&ConnectionId::from_string(
            old.origin.runtime_connection_id(),
        ))?;
        if !Arc::ptr_eq(&installed, old)
            || !old.principal.has_same_owner(&principal)
            || origin.offer_record_id() != old.origin.offer_record_id()
            || origin.credential_epoch() != old.origin.credential_epoch() + 1
        {
            return Err(StreamControlError::Authority);
        }
        Ok(Arc::new(Connection {
            domain: Arc::downgrade(self),
            origin,
            principal,
            profile: old.profile.clone(),
            plan: old.plan.clone(),
            negotiated_remote: Mutex::new(old.negotiated_remote.lock().map_err(denied)?.clone()),
            activation: Mutex::new(old.activation.lock().map_err(denied)?.clone()),
            binding: Mutex::new(old.binding.lock().map_err(denied)?.clone()),
            preparation: Mutex::new(old.preparation.lock().map_err(denied)?.clone()),
        }))
    }
    /// Swap only the checked original incarnations. Old Arcs retain old origins;
    /// queued work never acquires successor credentials by registry lookup.
    pub(crate) fn install_successors(
        &self,
        pairs: &[(Arc<Connection>, Arc<Connection>)],
    ) -> ControlResult<()> {
        let mut entries = self.entries.lock().map_err(denied)?;
        for (old, _) in pairs {
            let id = ConnectionId::from_string(old.origin.runtime_connection_id());
            if !entries.get(&id).is_some_and(|e| Arc::ptr_eq(e, old)) {
                return Err(StreamControlError::Authority);
            }
        }
        for (_, next) in pairs {
            entries.insert(
                ConnectionId::from_string(next.origin.runtime_connection_id()),
                next.clone(),
            );
        }
        Ok(())
    }
}
