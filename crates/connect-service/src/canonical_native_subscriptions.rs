//! Current server reoffers and outcome-only custody for retained selectors.
use crate::{
    canonical_media::{self, reconciliation::Grant, Connection},
    canonical_runtime::Runtime,
};
use async_trait::async_trait;
use connect_store::{NativeStreamCommitted, NativeStreamLease, NativeStreamNoEffect, Store};
use rvoip_auth_core::{
    bearer::ValidatedBearer,
    sig9421::{PrincipalVerificationKey, ResolvedVerificationKey},
};
use rvoip_core::ids::ConnectionId;
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, AuthorizedEnvelope, DispatchError, ExchangeReplyAuthority},
    native_negotiation::NativeOfferError,
    native_requests::{NativeRequestPolicy, NativeRequestScope},
    native_subscriptions::{
        NativeSubscriptionAuthority, NativeSubscriptionNoEffect, NativeSubscriptionOutcome,
        NativeSubscriptionPlan, NativeSubscriptionPolicy,
    },
    payloads::Device,
    Envelope, Identifier, Scope,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
struct Policy {
    runtime: Weak<Runtime>,
    entry: Arc<Connection>,
    domain: Arc<()>,
    attempt: Mutex<Option<Attempt>>,
}
struct Attempt {
    operation: String,
    lease: Option<NativeStreamLease>,
    captured: Option<Arc<Recording>>,
}
enum Captured {
    Success(Arc<NativeStreamCommitted>, u64, bool),
    Unknown(Arc<NativeStreamLease>, NativeOfferError),
    NoEffect(Arc<NativeStreamNoEffect>, NativeOfferError),
}
struct Recording {
    captured: Captured,
    done: AtomicBool,
}
struct Observation {
    domain: Arc<()>,
    operation: String,
    recording: Arc<Recording>,
}
impl Runtime {
    pub(crate) fn refresh_native_reconciliation(
        self: &Arc<Self>,
        id: &ConnectionId,
        result: &rvoip_uctp::v2::native::RefreshedNativeMedia,
    ) -> Result<(), DispatchError> {
        let entry = self.media.entry(id).map_err(|_| DispatchError::Authority)?;
        let native = self
            .connection_native_adapter(id)
            .map_err(|_| DispatchError::Authority)?;
        let origin = result.request_origin(&native)?;
        self.codec_origins
            .lock()
            .map_err(|_| DispatchError::Capacity)?
            .insert(entry.origin.connection_id().into(), origin);
        result.enable_subscriptions(
            &native,
            &self.core,
            Arc::new(Policy {
                runtime: Arc::downgrade(self),
                entry,
                domain: Arc::new(()),
                attempt: Mutex::new(None),
            }),
        )
    }
    pub(crate) fn enable_native_reconciliation(
        self: &Arc<Self>,
        id: &ConnectionId,
        a: &AuthorizedEnvelope,
    ) -> connect_protocol::Result<()> {
        let entry = self
            .media
            .entry(id)
            .map_err(|error| crate::canonical_native::registration_error(error.into()))?;
        let origin = self
            .connection_native_adapter(id)?
            .native_request_origin(id, a)
            .map_err(crate::canonical_native::registration_error)?;
        self.codec_origins
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .insert(entry.origin.connection_id().into(), origin);
        self.ensure_session_codec_worker()?;
        let policy = Arc::new(Policy {
            runtime: Arc::downgrade(self),
            entry,
            domain: Arc::new(()),
            attempt: Mutex::new(None),
        });
        self.connection_native_adapter(id)?
            .enable_native_subscriptions(&self.core, id, a, policy)
            .map_err(crate::canonical_native::registration_error)
    }
}
#[async_trait]
impl NativeSubscriptionPolicy for Policy {
    async fn reconcile(
        &self,
        scope: &NativeRequestScope,
        plan: &NativeSubscriptionPlan,
        pending: Option<&str>,
    ) -> Result<NativeSubscriptionAuthority, DispatchError> {
        tracing::debug!(target:"connect_service::canonical_native_streams", revision=scope.stream_revision, "Native reconciliation requested");
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        if runtime.stop.is_cancelled() {
            return Err(DispatchError::Closed);
        }
        {
            let state = self
                .attempt
                .try_lock()
                .map_err(|_| DispatchError::Capacity)?;
            if let Some(attempt) = state.as_ref() {
                if pending != Some(attempt.operation.as_str()) {
                    return Err(DispatchError::Capacity);
                }
                let recording = attempt
                    .captured
                    .as_ref()
                    .ok_or(DispatchError::Authority)?
                    .clone();
                // This token authorizes historical observation only. It cannot
                // enter the request, reply or MediaPolicy grant paths below.
                return Ok(NativeSubscriptionAuthority {
                    operation: attempt.operation.clone(),
                    lease: AuthorityLease::holding(Observation {
                        domain: self.domain.clone(),
                        operation: attempt.operation.clone(),
                        recording,
                    }),
                });
            }
        }
        if pending.is_some() {
            return Err(DispatchError::Authority);
        }
        let operation = format!("reconcile_{}", uuid::Uuid::new_v4().simple());
        let grant = runtime.media.reconciliation_grant(
            self.domain.clone(),
            self.entry.clone(),
            scope,
            plan.clone(),
            operation.clone(),
            runtime.stop.child_token(),
        ).map_err(|error|{tracing::debug!(target:"connect_service::canonical_native_streams",?error,"Native reconciliation source grant deferred");error})?;
        let lease=runtime.media.store.begin_native_reconciliation(grant.source.subscription()?,&operation,grant.descriptor()?).map_err(|error|{
            tracing::debug!(target:"connect_service::canonical_native_streams",code=%error.code,"Native reconciliation admission deferred");DispatchError::Authority
        })?;
        if let Err(error) = runtime
            .media
            .store
            .validate_native_reconciliation(grant.source.subscription()?, &lease)
        {
            // No native call has begun. Record this definite preflight refusal
            // rather than abandoning a reserved attempt as possible IO.
            let _ = runtime
                .media
                .store
                .fail_native_stream_without_effect(lease, error);
            return Err(DispatchError::Authority);
        }
        let mut state = self
            .attempt
            .try_lock()
            .map_err(|_| DispatchError::Capacity)?;
        if state.is_some() {
            return Err(DispatchError::Capacity);
        }
        *state = Some(Attempt {
            operation: operation.clone(),
            lease: Some(lease),
            captured: None,
        });
        Ok(NativeSubscriptionAuthority {
            operation,
            lease: canonical_media::reconciliation::lease(grant),
        })
    }
    async fn observe(
        &self,
        scope: &NativeRequestScope,
        operation: &str,
        result: &Result<NativeSubscriptionOutcome, NativeOfferError>,
        held: Option<&AuthorityLease>,
        no_effect: Option<&NativeSubscriptionNoEffect>,
    ) -> Result<(), DispatchError> {
        if no_effect.is_some_and(|p| {
            result
                .as_ref()
                .err()
                .is_none_or(|error| !p.matches(scope, operation, error))
        }) {
            return Err(DispatchError::Authority);
        }
        let held = held.ok_or(DispatchError::Authority)?;
        let historical = held.authority::<Observation>();
        let grant = held.authority::<Arc<Grant>>();
        if let Some(p) = historical {
            if !Arc::ptr_eq(&p.domain, &self.domain) || p.operation != operation {
                return Err(DispatchError::Authority);
            }
        } else {
            let p = grant.ok_or(DispatchError::Authority)?;
            if !Arc::ptr_eq(&p.domain, &self.domain)
                || p.operation != operation
                || !Arc::ptr_eq(&p.scope.origin, &scope.origin)
            {
                return Err(DispatchError::Authority);
            }
            // Historical result custody may outlive the permission/deadline that
            // allowed the possible effect. It confers no new media authority.
        }
        let mut state = self
            .attempt
            .try_lock()
            .map_err(|_| DispatchError::Capacity)?;
        let attempt = state
            .as_mut()
            .filter(|a| a.operation == operation)
            .ok_or(DispatchError::Authority)?;
        if let Some(recording) = &attempt.captured {
            let same = match (&recording.captured, result) {
                (Captured::Success(_, revision, resources), Ok(out)) => {
                    *revision == out.revision && *resources == out.resources_changed
                }
                (Captured::Unknown(_, old), Err(error)) => old == error && no_effect.is_none(),
                (Captured::NoEffect(_, old), Err(error)) => old == error && no_effect.is_some(),
                _ => false,
            };
            if !same || historical.is_some_and(|p| !Arc::ptr_eq(&p.recording, recording)) {
                return Err(DispatchError::Authority);
            }
            if recording.done.load(Ordering::Acquire) {
                // Recording uncertainty is not resolution. Keep the original
                // captured error/operation and permit historical observation only.
                if matches!(recording.captured, Captured::Unknown(..)) {
                    return Err(DispatchError::SubmissionUnknown);
                }
                *state = None;
                return Ok(());
            }
            return Err(DispatchError::SubmissionUnknown);
        }
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        let lease = attempt.lease.take().ok_or(DispatchError::Authority)?;
        let captured = match result {
            Ok(out) => Captured::Success(
                Arc::new(
                    lease
                        .committed(
                            out.revision,
                            out.resources_changed,
                            crate::canonical_native_streams::captured_projection(
                                &runtime,
                                &ConnectionId::from_string(
                                    self.entry.origin.runtime_connection_id(),
                                ),
                                out.revision,
                            ),
                        )
                        .map_err(|_| DispatchError::Authority)?,
                ),
                out.revision,
                out.resources_changed,
            ),
            Err(error) => {
                if no_effect.is_some() {
                    tracing::debug!(target:"connect_service::canonical_native_streams",?error,"Native routing reconciliation failed before graph commit");
                    Captured::NoEffect(
                        Arc::new(lease.without_effect(connect_protocol::Error::new(
                            "native-reconciliation-no-effect",
                            &error.to_string(),
                        ))),
                        *error,
                    )
                } else {
                    tracing::debug!(target:"connect_service::canonical_native_streams",?error,"Native reconciliation engine outcome remains uncertain");
                    Captured::Unknown(Arc::new(lease), *error)
                }
            }
        };
        let recording = Arc::new(Recording {
            captured,
            done: AtomicBool::new(false),
        });
        attempt.captured = Some(recording.clone());
        let store = runtime.media.store.clone();
        let stop = runtime.stop.clone();
        let mut work = runtime.work.lock().map_err(|_| DispatchError::Authority)?;
        while work.try_join_next().is_some() {}
        // Retain only the captured outcome and kernel-bound Store. Permission,
        // signed exchange, proposal and original readers are not in this job.
        work.spawn(async move {
            loop {
                let saved=match &recording.captured {
                    Captured::Success(receipt,_,_)=>store.complete_native_stream(receipt),
                    Captured::Unknown(lease,_)=>store.native_stream_attempt_uncertain(lease),
                    Captured::NoEffect(receipt,_)=>store.record_native_stream_without_effect(receipt),
                };
                if saved.is_ok(){recording.done.store(true,Ordering::Release);return;}
                tokio::select!{biased;_=stop.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
            }
        });
        Err(DispatchError::SubmissionUnknown)
    }
}
#[async_trait]
impl NativeRequestPolicy for Policy {
    async fn authorize(
        &self,
        scope: &NativeRequestScope,
        proposal: &Envelope,
        held: Option<&AuthorityLease>,
    ) -> Result<AuthorityLease, DispatchError> {
        let held = held.ok_or(DispatchError::Authority)?;
        let grant = held
            .authority::<Arc<Grant>>()
            .ok_or(DispatchError::Authority)?;
        if !held.current() || !Arc::ptr_eq(&grant.domain, &self.domain) {
            return Err(DispatchError::Authority);
        }
        grant.authorize_proposal(scope, proposal)?;
        Ok(held.retained())
    }
}
pub(crate) fn reply(
    store: &Arc<Store>,
    stop: &CancellationToken,
    c: &ValidatedBearer,
    d: &Device,
    i: &Identifier,
    request: &Envelope,
    response: &Envelope,
    held: &AuthorityLease,
) -> Option<Result<ExchangeReplyAuthority, DispatchError>> {
    let grant = held.authority::<Arc<Grant>>()?;
    Some((|| {
        let source = &grant.source;
        let meta = source.credential_metadata();
        let device = source.peer_device();
        let issued = std::time::UNIX_EPOCH.checked_add(Duration::from_secs(meta.issued_at as u64));
        let proposal = grant
            .proposal
            .try_lock()
            .map_err(|_| DispatchError::Authority)?;
        if !held.current()
            || !grant.current()
            || stop.is_cancelled()
            || !Arc::ptr_eq(store, &grant.policy.store)
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &c.principal,
                &grant.connection.principal,
            )
            || c.token_id.as_deref() != Some(meta.actor.credential_id.as_str())
            || c.issued_at != issued
            || i.as_str() != meta.actor.identity_id
            || d.id.as_str() != device.id
            || d.kind != device.kind
            || d.platform != device.platform
            || proposal.as_ref().is_none_or(|p| !same_proposal(p, request))
            || response.kind() != "connection.answer"
            || response.scope(Scope::Reply) != Some(request.id())
            || [
                Scope::Conversation,
                Scope::Session,
                Scope::Connection,
                Scope::Operation,
            ]
            .into_iter()
            .any(|s| request.scope(s) != response.scope(s))
            || response.original()["signature"]["keyid"] != source.signing_key_id()
            || response.original()["signature"]["alg"] != source.signing_key().algorithm.as_str()
        {
            return Err(DispatchError::Authority);
        }
        let p = &response.original()["payload"];
        let setup = &request.original()["payload"]["substrate_setup"];
        if p["substrate"].as_str() != Some(source.media().target().endpoint_transport.as_str())
            || p["substrate"] != request.original()["payload"]["substrate"]
            || p["by_participant"] != grant.connection.origin.participant_id()
            || p["substrate_setup"] != *setup
        {
            return Err(DispatchError::ReplyMismatch);
        }
        let answered=setup["streams"].as_array().ok_or(DispatchError::Authority)?.iter().map(|s|serde_json::json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str(){Some("sendonly")=>"recvonly",Some("recvonly")=>"sendonly",Some("sendrecv")=>"sendrecv",_=>"inactive"},"codec":s["codec"]})).collect::<Vec<_>>();
        if p["streams_answered"] != serde_json::json!(answered) {
            return Err(DispatchError::ReplyMismatch);
        }
        let current = source.signing_key();
        let key = PrincipalVerificationKey::from_current(
            &c.principal,
            source.signing_key_id().into(),
            ResolvedVerificationKey {
                public_key: current.public_key.clone(),
                algorithm: current.algorithm.as_str().into(),
                not_before: chrono::DateTime::from_timestamp(current.not_before, 0),
                expires_at: chrono::DateTime::from_timestamp(current.expires_at, 0),
                revoked: false,
            },
        )
        .map_err(|_| DispatchError::Authority)?;
        Ok(ExchangeReplyAuthority {
            lease: held.retained(),
            key,
        })
    })())
}
fn same_proposal(a: &Envelope, b: &Envelope) -> bool {
    a.kind() == b.kind()
        && a.expected_revision() == b.expected_revision()
        && a.original()["payload"] == b.original()["payload"]
        && [
            Scope::Conversation,
            Scope::Session,
            Scope::Connection,
            Scope::Operation,
        ]
        .into_iter()
        .all(|s| a.scope(s) == b.scope(s))
}
