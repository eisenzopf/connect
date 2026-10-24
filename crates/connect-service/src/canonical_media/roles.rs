//! Project already authorized durable roles into the disposable coordinator.
//! Physical sources retain their own principals and one complete current read.
use super::*;
use crate::canonical_runtime::Runtime;
use connect_store::{SessionRoleMediaRead, TelephoneMediaRead};
use rvoip_core::{
    ids::ParticipantId,
    orchestrator::participant_control::{
        ControlResult as ParticipantResult, ParticipantAuthority, ParticipantControl,
        ParticipantControlError, ParticipantControlLease, ParticipantScope,
    },
    participant_transitions::{ParticipantChange, ParticipantMutation},
};
use sha2::{Digest, Sha256};
#[derive(Clone)]
enum Source {
    Native(Arc<Connection>),
    Telephone(Arc<crate::telephone_replacement::media::Policy>),
}
#[derive(PartialEq, Eq)]
struct Facts {
    conversation: String,
    session: String,
    participant: String,
    epoch: u64,
    topology: u64,
    role: String,
}
impl Source {
    fn id(&self) -> ParticipantResult<ConnectionId> {
        match self {
            Self::Native(n) => Ok(ConnectionId::from_string(
                n.origin.runtime_connection_id().to_owned(),
            )),
            Self::Telephone(p) => p
                .connection()
                .map_err(|_| ParticipantControlError::Authority),
        }
    }
    fn principal(&self) -> ParticipantResult<AuthenticatedPrincipal> {
        match self {
            Self::Native(n) => Ok(n.principal.clone()),
            Self::Telephone(p) => p
                .principal()
                .map_err(|_| ParticipantControlError::Authority),
        }
    }
    fn facts(&self, read: &SessionRoleMediaRead) -> ParticipantResult<Facts> {
        let id = self.id()?;
        match self {
            Self::Native(_) => read.native(id.as_str()).map(|r| native_facts(r.target())),
            Self::Telephone(_) => read.telephone(id.as_str()).map(phone_facts),
        }
        .ok_or(ParticipantControlError::Authority)
    }
    fn owner(&self, read: &SessionRoleMediaRead) -> ParticipantResult<MediaOwner> {
        let id = self.id()?;
        let deny = || ParticipantControlError::Authority;
        match self {
            Self::Native(entry) => {
                let read = read.native(id.as_str()).ok_or_else(deny)?;
                let owner = entry
                    .owner(read.target(), &entry.principal, false)
                    .map_err(|_| deny())?;
                entry
                    .activation()
                    .map_err(|_| deny())?
                    .ok_or_else(deny)?
                    .current(owner)
                    .map_err(|_| deny())
            }
            Self::Telephone(entry) => entry
                .established_owner(read.telephone(id.as_str()).ok_or_else(deny)?)
                .map_err(|_| deny()),
        }
    }
}
fn native_facts(v: &PeerConnectionProjection) -> Facts {
    Facts {
        conversation: v.connection.conversation_id.clone(),
        session: v.connection.session_id.clone(),
        participant: v.connection.participant_id.clone(),
        epoch: v.connection.membership_epoch,
        topology: v.session.session.topology_revision,
        role: v.presence_role.clone(),
    }
}
fn phone_facts(v: &TelephoneMediaRead) -> Facts {
    let s = v.snapshot();
    Facts {
        conversation: s.conversation_id.clone(),
        session: s.session_id.clone(),
        participant: s.participant_id.clone(),
        epoch: s.membership_epoch,
        topology: v.session().topology_revision,
        role: v.role().into(),
    }
}
#[derive(Clone)]
pub(super) struct Projection {
    policy: Arc<Policy>,
    sources: Vec<Source>,
    read: Arc<SessionRoleMediaRead>,
    mutation: ParticipantMutation,
    deadline: tokio::time::Instant,
    cancellation: tokio_util::sync::CancellationToken,
}
impl Projection {
    pub(super) fn control(
        &self,
        policy: &Policy,
        scope: &ParticipantScope<'_>,
    ) -> ParticipantResult<ParticipantControl> {
        if !std::ptr::eq(policy, Arc::as_ptr(&self.policy)) {
            return Err(ParticipantControlError::Authority);
        }
        let _ = self.current(scope)?;
        ParticipantControl::new(
            Box::new(self.clone()),
            self.cancellation.clone(),
            self.deadline,
        )
    }
}
impl ParticipantControlLease for Projection {
    fn current(&self, scope: &ParticipantScope<'_>) -> ParticipantResult<ParticipantAuthority> {
        let deny = || ParticipantControlError::Authority;
        if !self.read.current()
            || self.cancellation.is_cancelled()
            || tokio::time::Instant::now() >= self.deadline
            || scope.mutation.operation_id != self.mutation.operation_id
            || scope.mutation.correlation_id != self.mutation.correlation_id
            || scope.mutation.expected_revision != self.mutation.expected_revision
            || scope.mutation.participant != self.mutation.participant
        {
            return Err(deny());
        }
        let ParticipantChange::SetSessionRole { role } = &scope.mutation.change else {
            return Err(ParticipantControlError::Unsupported);
        };
        let first = self.sources.first().ok_or_else(deny)?;
        let view = first.facts(&self.read)?;
        if !rvoip_core::staged_streams::reliable_data::same_credential(
            &first.principal()?,
            scope.principal,
        ) || scope.connection_generation.is_some()
            || scope.session.as_str() != view.session
            || scope.conversation.as_str() != view.conversation
            || scope.mutation.participant.as_str() != view.participant
            || *role != crate::canonical_runtime::participant_role(&view.role)
        {
            return Err(deny());
        }
        let mut authority: Option<ParticipantAuthority> = None;
        for source in &self.sources {
            if source.facts(&self.read)? != view {
                return Err(deny());
            }
            let owner = source.owner(&self.read)?;
            owner
                .member
                .revalidate_principal(&source.principal()?, chrono::Utc::now())
                .map_err(|_| deny())?;
            let send = owner.receive_media.contains(&Medium::Audio);
            let receive = owner.send_media.contains(&Medium::Audio);
            if let Some(a) = &mut authority {
                a.may_send_audio |= send;
                a.may_receive_audio |= receive;
            } else {
                authority = Some(ParticipantAuthority {
                    actor: owner.member.clone(),
                    target: owner.member,
                    endpoint: None,
                    may_send_audio: send,
                    may_receive_audio: receive,
                    confirmation_required: false,
                    confirmed_generation: None,
                });
            }
        }
        authority.ok_or_else(deny)
    }
}
impl Policy {
    fn unchanged_native_role_hint(
        &self,
        id: &ConnectionId,
        core: &rvoip_core::Orchestrator,
    ) -> bool {
        let Ok(entry) = self.entry(id) else {
            return false;
        };
        let Ok(Some(role)) = self.store.try_native_session_role_hint(&entry.origin) else {
            return false;
        };
        core.participant_session_role(
            &SessionId::from_string(entry.origin.session_id()),
            &ParticipantId::from_string(entry.origin.participant_id()),
        )
        .ok()
        .flatten()
            == Some(crate::canonical_runtime::participant_role(&role))
    }
    fn role_sources(
        &self,
        facts: &Facts,
    ) -> connect_protocol::Result<(
        Vec<Source>,
        Vec<Arc<NativeMediaOrigin>>,
        Vec<Arc<connect_store::TelephoneMediaOrigin>>,
    )> {
        let mut sources = Vec::new();
        let mut native = Vec::new();
        let mut phone = Vec::new();
        {
            let entries = self
                .entries
                .lock()
                .map_err(|_| connect_protocol::Error::storage())?;
            for n in entries.values().filter(|n| {
                n.origin.conversation_id() == facts.conversation
                    && n.origin.session_id() == facts.session
                    && n.origin.participant_id() == facts.participant
                    && n.origin.membership_epoch() == facts.epoch
            }) {
                native.push(n.origin.clone());
                sources.push(Source::Native(n.clone()));
            }
        }
        {
            let entries = self
                .telephone_targets
                .lock()
                .map_err(|_| connect_protocol::Error::storage())?;
            for p in entries.values().filter_map(Weak::upgrade) {
                if let Some(origin) = p.established_origin().filter(|o| {
                    let s = o.snapshot();
                    s.conversation_id == facts.conversation
                        && s.session_id == facts.session
                        && s.participant_id == facts.participant
                        && s.membership_epoch == facts.epoch
                }) {
                    phone.push(origin);
                    sources.push(Source::Telephone(p));
                }
            }
        }
        Ok((sources, native, phone))
    }
}
impl Runtime {
    pub(crate) async fn reconcile_connection_role(&self, id: &ConnectionId) {
        let Ok(_serial) = self.role_projection.try_lock() else {
            return;
        };
        if self.media.unchanged_native_role_hint(id, &self.core) {
            return;
        }
        // The unchanged-role poll needs only the actual current source's read.
        let probe = if let Ok(entry) = self.media.entry(id) {
            let Ok(read) = self.media.store.try_native_media_read(&entry.origin, None) else {
                return;
            };
            if !read.current() {
                return;
            }
            native_facts(read.target())
        } else {
            let Ok(Some(entry)) = self.media.telephone_target(id) else {
                return;
            };
            let Some(origin) = entry.established_origin() else {
                return;
            };
            let Ok(read) = self.media.store.try_telephone_media_read(origin) else {
                return;
            };
            if !read.current() {
                return;
            }
            phone_facts(&read)
        };
        let sid = SessionId::from_string(probe.session.clone());
        let pid = ParticipantId::from_string(probe.participant.clone());
        if self
            .core
            .participant_session_role(&sid, &pid)
            .ok()
            .flatten()
            == Some(crate::canonical_runtime::participant_role(&probe.role))
        {
            return;
        }
        let Ok((sources, native, phone)) = self.media.role_sources(&probe) else {
            return;
        };
        let Ok(read) = self.media.store.try_session_role_media_read(
            &native,
            &phone,
            &probe.conversation,
            &probe.session,
            &probe.participant,
            probe.epoch,
        ) else {
            return;
        };
        let read = Arc::new(read);
        let Some(first) = sources.first() else {
            return;
        };
        let Ok(view) = first.facts(&read) else {
            return;
        };
        let desired = crate::canonical_runtime::participant_role(&view.role);
        if self
            .core
            .participant_session_role(&sid, &pid)
            .ok()
            .flatten()
            == Some(desired.clone())
        {
            return;
        }
        let Ok(revision) = self.core.participant_revision(&sid) else {
            return;
        };
        let operation = format!(
            "connection-role-{}",
            hex::encode(Sha256::digest(
                serde_json::to_vec(&serde_json::json!([
                    view.conversation,
                    view.session,
                    view.participant,
                    view.epoch,
                    view.topology,
                    view.role
                ]))
                .unwrap_or_default()
            ))
        );
        let Ok(principal) = first.principal() else {
            return;
        };
        let mutation = ParticipantMutation {
            operation_id: operation.clone(),
            correlation_id: operation,
            expected_revision: revision,
            participant: pid.clone(),
            change: ParticipantChange::SetSessionRole {
                role: desired.clone(),
            },
        };
        let projection = Projection {
            policy: self.media.clone(),
            sources,
            read,
            mutation: mutation.clone(),
            deadline: tokio::time::Instant::now() + Duration::from_millis(250),
            cancellation: self.stop.child_token(),
        };
        let outcome = self
            .core
            .change_session_participant_with_held(
                &sid,
                principal,
                mutation,
                projection.deadline,
                projection.cancellation.clone(),
                &projection,
            )
            .await;
        if let Err(error) = &outcome {
            tracing::debug!(target:"connect_service::canonical_roles",?error,"Durable physical role projection deferred");
        }
        if outcome.is_ok()
            && self
                .core
                .participant_session_role(&sid, &pid)
                .ok()
                .flatten()
                == Some(desired)
        {
            self.role_projections
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
