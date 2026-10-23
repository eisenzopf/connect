//! One admitted transport interval and its captured historical result.
use super::{VoiceAction, VoiceJob};
use crate::{authority_fences::Manager, AuthorityGuardSet, Store};
use connect_connector_api::{lifecycle::*, Failure, VoiceReceipt};
use connect_protocol::{Error, Result};
use std::sync::Arc;

/// Protected adapter custody, never a public command or a serializable grant.
/// Retain this interval until the bounded physical request has returned/dropped.
pub struct VoiceDispatch {
    job: VoiceJob,
    _authority: Arc<AuthorityGuardSet>,
    owner: Option<Arc<crate::InstallationOwnership>>,
    manager: Arc<Manager>,
    runtime: String,
    expires_at: i64,
}
pub struct CapturedVoiceResult {
    job: VoiceJob,
    manager: Arc<Manager>,
    runtime: String,
    _owner: Option<Arc<crate::InstallationOwnership>>,
    state: String,
    provider_id: Option<String>,
    control_url: Option<String>,
    reason: Option<String>,
    observation: Observation,
}
macro_rules! redacted {($($t:ty),*)=>{$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($t),"(redacted)"))}})*};}
redacted!(VoiceDispatch, CapturedVoiceResult);
impl VoiceDispatch {
    pub(super) fn new(
        store: &Store,
        job: &VoiceJob,
        authority: Arc<AuthorityGuardSet>,
        expires_at: i64,
        owner: Option<Arc<crate::InstallationOwnership>>,
    ) -> Self {
        Self {
            job: VoiceJob {
                call: job.call.clone(),
                route: job.route.clone(),
                action: job.action,
                control_url: job.control_url.clone(),
                end_intent_id: job.end_intent_id.clone(),
            },
            _authority: authority,
            owner,
            manager: store.authority.clone(),
            runtime: store.runtime_epoch().into(),
            expires_at,
        }
    }
    pub fn current(&self) -> bool {
        self.expires_at > crate::now() && self.owner.as_ref().is_none_or(|owner| owner.current())
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn job(&self) -> Result<&VoiceJob> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(&self.job)
    }
    /// The trusted adapter supplies the decoded outcome from this one request.
    /// Capturing releases transport authority before any result writer waits.
    /// A late result remains history; this value cannot authorize another IO.
    pub fn capture(
        self,
        result: std::result::Result<VoiceReceipt, Failure>,
    ) -> CapturedVoiceResult {
        let result = result.and_then(|receipt| {
            let normalized = match receipt.call.state.as_str() {
                "queued" | "ringing" => VoiceState::Starting,
                "in-progress" => VoiceState::Active,
                "ended" => VoiceState::Ended,
                "failed" => VoiceState::Failed,
                "uncertain" => VoiceState::Uncertain,
                _ => return Err(Failure::Uncertain),
            };
            let identifier =
                |id: &str| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control);
            if !identifier(&receipt.call.id)
                || !identifier(&receipt.observation.event_id)
                || receipt
                    .call
                    .ended_reason
                    .as_ref()
                    .is_some_and(|reason| reason.len() > 256)
                || receipt
                    .call
                    .control_url
                    .as_ref()
                    .is_some_and(|url| url.len() > 4096)
                || receipt.observation.mutation_id != self.job.call.id
                || receipt.observation.external_id.as_deref() != Some(receipt.call.id.as_str())
                || receipt.observation.state != State::Voice(normalized)
                || self
                    .job
                    .call
                    .provider_id
                    .as_deref()
                    .is_some_and(|id| id != receipt.call.id)
                || receipt.observation.verification == Verification::LocalAttempt
            {
                return Err(Failure::Uncertain);
            }
            Ok(receipt)
        });
        let (state, provider_id, control_url, reason, observation) = match result {
            Ok(receipt) => (
                receipt.call.state,
                Some(receipt.call.id),
                receipt.call.control_url,
                receipt.call.ended_reason,
                receipt.observation,
            ),
            Err(failure) => {
                let failed =
                    self.job.action == VoiceAction::Start && matches!(failure, Failure::Rejected);
                let state = if failed { "failed" } else { "uncertain" };
                let observation = Observation {
                    mutation_id: self.job.call.id.clone(),
                    external_id: None,
                    event_id: format!("attempt:{}:{:?}:{state}", self.job.call.id, self.job.action),
                    state: State::Voice(if failed {
                        VoiceState::Failed
                    } else {
                        VoiceState::Uncertain
                    }),
                    verification: Verification::LocalAttempt,
                };
                (state.into(), None, None, None, observation)
            }
        };
        CapturedVoiceResult {
            job: self.job,
            manager: self.manager,
            runtime: self.runtime,
            _owner: self.owner,
            state,
            provider_id,
            control_url,
            reason,
            observation,
        }
    }
}
impl Store {
    /// Durable once-only admission precedes a separately acquired current
    /// transport read. Withdrawal in that gap never grants physical IO.
    pub fn begin_voice_dispatch(&self, job: &VoiceJob) -> Result<Option<VoiceDispatch>> {
        let owner = self.runtime_ownership.permit(&self.path)?;
        if !self.admit_voice_dispatch(job)? {
            return Ok(None);
        }
        #[cfg(test)]
        admission_boundary()?;
        let authority = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                super::get(&c, &job.call.conversation_id, &job.call.id)?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    &job.call.conversation_id,
                    crate::AuthorityMode::Read,
                )
            },
        )?;
        let admitted = {
            let c = self.connection()?;
            self.voice_transport_deadline(&c, job)
        };
        match admitted {
            Ok(expires) => Ok(Some(VoiceDispatch::new(
                self, job, authority, expires, owner,
            ))),
            Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => {
                // This request is known not sent. No invalidated model grant is
                // reused to issue cleanup; only its admitted local facts settle.
                let captured = VoiceDispatch::new(self, job, authority, crate::now(), owner)
                    .capture(Err(Failure::Rejected));
                let current = {
                    let c = self.connection()?;
                    super::get(&c, &job.call.conversation_id, &job.call.id)?
                };
                if !matches!(
                    current.state.as_str(),
                    "ended" | "failed" | "cancelled" | "forbidden"
                ) {
                    self.record_voice_dispatch(&captured)?;
                }
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    fn voice_transport_deadline(&self, c: &rusqlite::Connection, job: &VoiceJob) -> Result<i64> {
        use crate::{conversations::authorized_conversation, identity::actor_from_connection};
        use connect_protocol::membership::ContentKind;
        super::dispatch::binding(c, job)?;
        let now = crate::now();
        let expires = match job.action {
            VoiceAction::Start => {
                if job.call.state != "starting"
                    || job.call.end_requested
                    || !super::route(c, &job.call.route_id)?.enabled
                {
                    return Err(Error::forbidden());
                }
                let credential: String = c
                    .query_row(
                        "SELECT credential_id FROM voice_calls WHERE id=?1",
                        [&job.call.id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                let actor = actor_from_connection(c, &credential)?;
                actor.require("call_start")?;
                crate::audience::require_original(
                    c,
                    &actor,
                    &job.call.conversation_id,
                    ContentKind::Call,
                    &job.call.id,
                )?;
                if !crate::audience::original_to_identity(
                    c,
                    &job.route.identity_id,
                    &job.call.conversation_id,
                    ContentKind::Call,
                    &job.call.id,
                )? || authorized_conversation(c, &actor, &job.call.conversation_id)?.closed
                {
                    return Err(Error::forbidden());
                }
                actor.expires_at.min(crate::tasks::effect_link_deadline(
                    c,
                    "call",
                    &job.call.id,
                    now,
                )?)
            }
            VoiceAction::End => {
                if job.call.state != "ending" {
                    return Err(Error::forbidden());
                }
                let (actor, expires) = crate::media_end::dispatched_authority(
                    c,
                    crate::media_end::Kind::Call,
                    &job.call.id,
                    job.end_intent_id.as_deref().ok_or_else(Error::forbidden)?,
                    now,
                )?;
                crate::audience::require_original(
                    c,
                    &actor,
                    &job.call.conversation_id,
                    ContentKind::Call,
                    &job.call.id,
                )?;
                authorized_conversation(c, &actor, &job.call.conversation_id)?;
                expires
            }
            VoiceAction::Poll => {
                if job.call.provider_id.is_none()
                    || matches!(
                        job.call.state.as_str(),
                        "pending"
                            | "starting"
                            | "ending"
                            | "ended"
                            | "failed"
                            | "cancelled"
                            | "forbidden"
                    )
                {
                    return Err(Error::forbidden());
                }
                i64::MAX
            }
        };
        let expires = expires.min(now.saturating_add(35));
        if expires <= now {
            return Err(Error::forbidden());
        }
        Ok(expires)
    }
    /// Record only the captured request's facts, even after principal withdrawal.
    /// A foreign installation/runtime cannot adopt this result as its own.
    pub fn record_voice_dispatch(&self, result: &CapturedVoiceResult) -> Result<()> {
        if !Arc::ptr_eq(&self.authority, &result.manager) || self.runtime_epoch() != result.runtime
        {
            return Err(Error::forbidden());
        }
        self.finish_voice_result(
            &result.job,
            &result.state,
            result.provider_id.as_deref(),
            result.control_url.as_deref(),
            result.reason.as_deref(),
            Some(&result.observation),
            true,
        )
    }
}

#[cfg(test)]
thread_local! { static BOUNDARY: std::cell::RefCell<Option<(std::sync::mpsc::Sender<()>,std::sync::mpsc::Receiver<()>)>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(crate) fn pause_after_admission(
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
) {
    BOUNDARY.with(|slot| *slot.borrow_mut() = Some((entered, release)));
}
#[cfg(test)]
fn admission_boundary() -> Result<()> {
    if let Some((entered, release)) = BOUNDARY.with(|slot| slot.borrow_mut().take()) {
        entered.send(()).map_err(|_| Error::storage())?;
        release
            .recv_timeout(std::time::Duration::from_secs(8))
            .map_err(|_| Error::storage())?;
    }
    Ok(())
}
