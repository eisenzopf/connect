//! Current domain authority during one live Core movement's journal projection.
//! A returned read is never proof that Core published a route. The Service must
//! additionally hold the actual managed binding/generation delivery capability.
use super::*;

fn pending_facts(
    store: &Store,
    c: &Connection,
    activation: &NativeReplacementActivation,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    let origin = activation.origin();
    current_owner(store, origin).map_err(|e| native_view_error("continuation owner", e))?;
    credential_refresh::require_epoch(store, c, origin)
        .map_err(|e| native_view_error("continuation credential epoch", e))?;
    let NativeMediaAdmission::Replacement {
        record,
        attempt,
        effect,
        runtime,
        connection,
        core,
        generation,
    } = &origin.attempt
    else {
        return Err(Error::forbidden());
    };
    // No mere receipt/Ready row is accepted. This is the same one started
    // original commit and local runtime, before any durable ordinary lifetime.
    // The installation epoch identifies native resources. The transition worker
    // also pins this database's filesystem identity; validate both exact owners.
    let exact: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements r JOIN operations o ON o.record_id=r.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=o.record_id WHERE o.record_id=?1 AND s.attempt_id=?2 AND s.effect_id=?3 AND r.runtime_epoch=?4 AND r.candidate_connection_id=?5 AND r.candidate_runtime_id=?6 AND r.generation=?7 AND s.started=1 AND e.provider='native-replacement-commit' AND e.attempt=1 AND e.submission_generation=1 AND e.state IN ('submitting','confirmed') AND o.state='preparing' AND o.cancel_requested=0 AND o.expires_at>?8 AND r.expires_at>?8 AND o.worker_runtime=?9 AND o.lease_owner=r.reservation_id AND o.lease_expires_at=r.expires_at AND json_extract(e.observation_json,'$.candidate')=?6 AND json_extract(e.observation_json,'$.generation')=?7 AND (s.observation_json IS NULL OR (json_extract(s.observation_json,'$.movement.candidate')=?6 AND json_extract(s.observation_json,'$.movement.generation')=?7)) AND NOT EXISTS(SELECT 1 FROM native_replacement_lifetimes l WHERE l.operation_record_id=o.record_id))",params![record,attempt,effect,runtime,connection,core,generation,crate::now(),store.transition_runtime],|r|r.get(0)).map_err(|_| Error::storage())?;
    if !exact {
        // An in-flight packet can straddle the exact final SQL publication.
        // Signal only this known phase edge as retryable. The caller starts a
        // new lookup; it must never fall back from a refused continuation read.
        let ordinary: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_lifetimes l JOIN operations o ON o.record_id=l.operation_record_id AND o.state='completed' WHERE l.operation_record_id=?1 AND l.attempt_runtime=?2 AND l.attempt_id=?3 AND l.effect_id=?4 AND l.generation=?5)",params![record,runtime,attempt,effect,generation],|r|r.get(0)).map_err(|_| Error::storage())?;
        return Err(if ordinary {
            Error::new(
                "authority-busy",
                "Exact replacement policy publication advanced",
            )
        } else {
            native_view_error("continuation original commit", Error::forbidden())
        });
    }
    let (meta, key, mut scopes, mut expiry) = checked(store, c, &origin.binding)
        .map_err(|e| native_view_error("continuation target binding", e))?;
    if key.public_key != origin.public_key {
        return Err(Error::forbidden());
    }
    meta.actor.require("connection.offer")?;
    let recipients = crate::connection_control::intent_resources(c, &meta.actor, &origin.intent)
        .map_err(|e| native_view_error("continuation recipients", e))?;
    if recipients
        .iter()
        .any(|id| !meta.actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    let (original_scopes, original_expiry) = activation
        .original
        .continuation_authority(store, c, &meta.actor.identity_id)
        .map_err(|e| native_view_error("continuation original authority", e))?;
    scopes.extend(original_scopes);
    expiry = expiry.min(original_expiry).min(origin.expires_at);
    let member =
        crate::audience::member(c, &meta.actor.identity_id, origin.conversation_id(), true)?;
    let session = crate::session_control::view(c, origin.conversation_id(), origin.session_id())?;
    if member.participant_id != origin.participant_id
        || member.membership_epoch != origin.membership_epoch
        || !matches!(session.state.as_str(), "accepted" | "ready" | "active")
    {
        return Err(Error::forbidden());
    }
    // This is a public continuation only. Replacing a private-floor primary
    // cannot discard that purpose merely because the new ID has no floor row.
    let old = activation
        .original
        .request()
        .connection_id
        .as_deref()
        .ok_or_else(Error::forbidden)?;
    if origin.intent.offer.get("private_consultation").is_some()
        || origin.intent.offer.get("handoff_preparation").is_some()
        || activation.target().private_consultation.is_some()
        || activation.target().handoff_preparation
    {
        return Err(Error::forbidden());
    }
    for id in [old, origin.connection_id()] {
        if crate::connection_control::private_consultation::for_connection(
            store,
            c,
            id,
            crate::connection_control::private_consultation::Purpose::Media,
        )?
        .is_some()
            || crate::connection_control::private_consultation::private_partner(c, id)?.is_some()
        {
            return Err(Error::forbidden());
        }
    }
    // Re-read current endpoint, current accepted resources and current role.
    // The captured preparation projection supplies immutable IDs only.
    let row: Option<(String,String,String,String,String)> = c.query_row("SELECT e.address,e.transport,e.capabilities_json,cap.capabilities_json,sp.role FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 JOIN session_participants sp ON sp.session_id=?1 AND sp.participant_id=?2 AND sp.membership_epoch=?3 AND sp.state='present' JOIN session_presence_capabilities cap ON cap.session_id=sp.session_id AND cap.participant_id=sp.participant_id AND cap.membership_epoch=sp.membership_epoch WHERE e.id=?4 AND e.device_id=?5 AND e.identity_id=?6 AND e.verification_state='verified' AND e.transport IN ('quic','webtransport','webrtc')",params![origin.session_id(),origin.participant_id,origin.membership_epoch,origin.endpoint_id,origin.binding.device.id,meta.actor.identity_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_| Error::storage())?;
    let (address, transport, endpoint_caps, accepted, role) = row.ok_or_else(Error::forbidden)?;
    if transport != origin.intent.endpoint_transport()?.as_str() {
        return Err(Error::forbidden());
    }
    let revision = c
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1 AND closed=0",
            [origin.conversation_id()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        origin.conversation_id(),
        AuthorityMode::Read,
    )?);
    let view = PeerConnectionProjection {
        session: PeerSessionProjection {
            session,
            actor: member,
            conversation_revision: revision,
            present: true,
        },
        connection: connect_protocol::connection_control::PreparedConnection {
            connection_id: origin.connection_id().into(),
            conversation_id: origin.conversation_id().into(),
            session_id: origin.session_id().into(),
            participant_id: origin.participant_id().into(),
            membership_epoch: origin.membership_epoch(),
            endpoint_id: origin.endpoint_id.clone(),
            generation: origin.generation(),
            revision: activation.target().connection.revision,
            // Internal eligibility facts, never a journal Ready notification.
            state: "ready".into(),
        },
        runtime_connection_id: origin.runtime_connection_id().into(),
        device_id: origin.binding.device.id.clone(),
        endpoint_address: address,
        endpoint_transport: origin.intent.endpoint_transport()?,
        endpoint_capabilities: serde_json::from_str(&endpoint_caps)
            .map_err(|_| Error::storage())?,
        accepted_capabilities: serde_json::from_str(&accepted).map_err(|_| Error::storage())?,
        presence_role: role,
        offer: origin.intent.offer.clone(),
        private_consultation: None,
        handoff_preparation: false,
        publication_suspended: false,
    };
    Ok((view, AuthorityScope::merged(&scopes)?, expiry))
}
fn current_facts(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
    activation: Option<&NativeReplacementActivation>,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    match activation {
        Some(activation) => {
            if !std::ptr::eq(origin, activation.origin().as_ref()) {
                return Err(native_view_error(
                    "continuation activation identity",
                    Error::forbidden(),
                ));
            }
            pending_facts(store, c, activation)
        }
        None => facts(store, c, origin),
    }
}
impl Store {
    /// Exact pending target/source pair only. This returns domain readers, not
    /// a Core route grant. Only immutable facts reuse the existing exact-version
    /// memo. Every call owns fresh authority or validates the actual held source
    /// interval; no retry, old-route fallback or retained grant is introduced.
    pub fn try_native_replacement_continuation_media_read(
        &self,
        target: &NativeMediaOrigin,
        target_activation: Option<&NativeReplacementActivation>,
        source: Option<(&NativeMediaOrigin, Option<&NativeReplacementActivation>)>,
        source_read: Option<&NativeMediaRead>,
    ) -> Result<NativeMediaRead> {
        if target_activation.is_none() && source.as_ref().is_none_or(|(_, a)| a.is_none()) {
            return Err(Error::forbidden());
        }
        current_owner(self, target)?;
        // These checks must precede memo lookup, including on a positive hit.
        // Equal logical IDs never let another origin borrow an activation.
        if target_activation.is_some_and(|a| !std::ptr::eq(target, a.origin().as_ref())) {
            return Err(Error::forbidden());
        }
        if let Some((origin, activation)) = source {
            current_owner(self, origin)?;
            if origin.conversation_id() != target.conversation_id()
                || origin.session_id() != target.session_id()
                || activation.is_some_and(|a| !std::ptr::eq(origin, a.origin().as_ref()))
            {
                return Err(Error::forbidden());
            }
        }
        if let Some(held) = source_read {
            let original = source.as_ref().map(|(o, _)| *o).unwrap_or(target);
            if !held.current()
                || !self.authority.owns(held.interval())
                || !Arc::ptr_eq(&self.require_runtime_owner()?, &held.owner)
                || held.target.runtime_connection_id != original.runtime_connection_id()
                || held.target.connection.participant_id != original.participant_id()
                || held.target.connection.membership_epoch != original.membership_epoch()
                || held.target.connection.generation != original.generation()
            {
                return Err(Error::forbidden());
            }
        }
        // A pending target has its own immutable phase token. A pending source
        // also keys by that token, so neither phase can borrow ordinary facts.
        // The existing memo bounds entries and retains only weak source tokens.
        let metadata = target_activation.map_or(&target.metadata, |a| &a.metadata);
        let memo = &metadata
            .downcast_ref::<snapshot::Metadata>()
            .ok_or_else(Error::storage)?
            .continuation;
        let derive = |c: &Connection| {
            let (target_view, mut scopes, mut expiry) =
                current_facts(self, c, target, target_activation)
                    .map_err(|e| native_view_error("continuation target facts", e))?;
            let source_view = if let Some((origin, activation)) = source {
                if origin.conversation_id() != target.conversation_id()
                    || origin.session_id() != target.session_id()
                {
                    return Err(Error::forbidden());
                }
                let (view, extra, until) = current_facts(self, c, origin, activation)
                    .map_err(|e| native_view_error("continuation source facts", e))?;
                require_current_pair(self, c, target, origin, &target_view, &view)
                    .map_err(|e| native_view_error("continuation current pair", e))?;
                scopes.extend(extra);
                expiry = expiry.min(until);
                Some(view)
            } else {
                None
            };
            Ok((
                (
                    target_view,
                    source_view,
                    expiry,
                    subscription_permission(c, target)?,
                ),
                AuthorityScope::merged(&scopes)?,
            ))
        };
        let source_token = source.map(|(o, a)| a.map_or(&o.metadata, |a| &a.metadata));
        let ((target_view, source_view, mut expires_at, may_subscribe), interval) =
            match source_read {
                Some(held) => snapshot::cached_under(
                    self,
                    memo,
                    source_token,
                    held.interval(),
                    derive,
                    |_| Ok(()),
                ),
                None => snapshot::cached(self, memo, source_token, derive, |_| Ok(())),
            }?;
        if let Some(held) = source_read {
            if !held.current() {
                return Err(Error::forbidden());
            }
            expires_at = expires_at.min(held.expires_at);
        }
        if let Some((origin, _)) = source {
            current_owner(self, origin)?;
        }
        let owner = current_owner(self, target)?;
        if crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        Ok(NativeMediaRead {
            target: target_view,
            source: source_view,
            _scopes: interval,
            owner,
            expires_at,
            may_subscribe,
        })
    }
}
