//! Actual signed target negotiation, exact replacement resources and initial mute.
use super::media::{StartInvocation, TargetPolicy};
use super::*;
use crate::{canonical_native::Reply, canonical_runtime::Physical, StoreInvocationAuthority};
use rvoip_core::{ids::ConversationId, staged_streams::StreamController};
use rvoip_uctp::{staged_media::StagedRtpScope, v2::auth_dispatch::AuthorizedEnvelope};
use std::sync::OnceLock;

pub(super) struct StartedReplacement {
    pub receipt: Arc<connect_store::NativeReplacementStarted>,
    pub policy: Arc<TargetPolicy>,
    pub controller: Arc<StreamController>,
    intent: connect_protocol::connection_control::OfferIntent,
    pub commit: std::sync::Mutex<Option<super::movement::Capture>>,
    pub phases: std::sync::Mutex<Vec<Arc<connect_store::NativeReplacementMediaCapture>>>,
    pub preparation:
        OnceLock<Arc<rvoip_core::staged_streams::endpoint_preparation::AudioEndpointPreparation>>,
}
fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|a| a.peer_guard())
        .ok_or_else(Error::forbidden)
}
impl Runtime {
    pub(crate) async fn start_native_replacement(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Reply> {
        let _roles = self.role_projection.lock().await;
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(&a, authority)?;
        let native = self.native_adapter(physical.binding.transport())?;
        let intent = crate::canonical_native::intent(a.envelope())?;
        let record = intent.offer["replacement_preparation"]["operation_record_id"]
            .as_str()
            .ok_or_else(Error::forbidden)?;
        let saved = {
            self.replacements
                .lock()
                .map_err(|_| Error::storage())?
                .get(record)
                .cloned()
                .ok_or_else(|| {
                    Error::new("not-ready", "Owned replacement preparation is not live")
                })?
        };
        let allocated = {
            saved
                .allocated
                .lock()
                .map_err(|_| Error::storage())?
                .clone()
                .ok_or_else(|| {
                    Error::new(
                        "not-ready",
                        "Signed target must allocate the original reservation first",
                    )
                })?
        };
        if !allocated
            .physical
            .upgrade()
            .is_some_and(|p| Arc::ptr_eq(&p, &physical))
        {
            return Err(Error::forbidden());
        }
        let previous = {
            allocated
                .started
                .lock()
                .map_err(|_| Error::storage())?
                .clone()
        };
        if let Some(previous) = previous {
            if previous.intent != intent {
                return Err(Error::new(
                    "conflict",
                    "Replacement media start differs from its original intent",
                ));
            }
            self.record_started_replacement_with_peer(&previous, a.clone())
                .await?;
            let store = self.media.store.clone();
            let a = a.clone();
            let key = credential.to_owned();
            let receipt = previous.receipt.clone();
            let record = store_call(move || {
                store.replay_native_replacement_start_with_peer(&receipt, &key, proof(&a)?)
            })
            .await?;
            return Ok(Reply {
                record,
                answered: false,
            });
        }
        if !allocated.recorded.load(Ordering::Acquire) {
            return Err(Error::new(
                "not-ready",
                "Original native allocation capture is not yet recorded",
            ));
        }
        let read = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let a = a.clone();
            let receipt = allocated.receipt.clone();
            let key = credential.to_owned();
            store_call(move || {
                store.native_replacement_start_read_with_peer(&receipt, &key, proof(&a)?)
            })
        })
        .await?;
        if crate::canonical_runtime::core_transport(&read.target().endpoint_transport)?
            != physical.binding.transport()
        {
            return Err(Error::forbidden());
        }
        let base = crate::canonical_native::base_profile()?;
        let (profile, plan) =
            crate::canonical_media::offer_plan(read.target(), &base, a.principal()).map_err(
                |_| {
                    Error::new(
                        "unsupported-media-profile",
                        "Replacement target has no accepted audio codec",
                    )
                },
            )?;
        let (attempt, read) = {
            let store = self.media.store.clone();
            let a = a.clone();
            let receipt = allocated.receipt.clone();
            let key = credential.to_owned();
            store_call(move || {
                store.begin_native_replacement_start_with_read(receipt, &key, proof(&a)?, read)
            })
            .await?
        };
        let read = Arc::new(read);
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(
                (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64
            );
        let policy = Arc::new(TargetPolicy {
            base: self.media.clone(),
            pending: saved.handle.prepared_binding(),
            principal: a.principal().clone(),
            profile,
            plan,
            root: Arc::downgrade(&saved),
            activation: OnceLock::new(),
            origin: OnceLock::new(),
            preparation_operation: OnceLock::new(),
        });
        let invocation = Arc::new(StartInvocation {
            policy: policy.clone(),
            authorized: a.clone(),
            read,
            deadline,
        });
        let id = saved.handle.intent().replacement.clone();
        let scope = StagedRtpScope {
            session: SessionId::from_string(intent.session_id.clone()),
            connection: id.clone(),
            owner: a.principal().ownership_key(),
            wire_conversation: Some(intent.conversation_id.clone()),
            wire_session: intent.session_id.clone(),
            wire_connection: intent.connection_id.clone(),
        };
        let result=async {
            self.media.register_replacement_target(&policy).map_err(|_|Error::forbidden())?;
            tracing::debug!(target:"connect_service::canonical_replacement",stage="configure","Replacement resource start");
            let controller=native.configure_negotiated_media_with_held(&id,&a,ConversationId::from_string(intent.conversation_id.clone()),scope,self.media.clone(),policy.profile.clone(),None,8,invocation.as_ref()).map_err(|_|Error::forbidden())?;
            let activation=native.native_media_activation(&id,&a).map_err(|_|Error::forbidden())?;
            policy.activation.set(activation.clone()).map_err(|_|Error::forbidden())?;
            tracing::debug!(target:"connect_service::canonical_replacement",stage="negotiate","Replacement resource start");
            let outcome=native.negotiate_native_offer(&id,a.clone(),invocation.clone(),deadline).await.map_err(|e|{tracing::debug!(target:"connect_service::canonical_replacement",?e,"Replacement native negotiation refused");Error::forbidden()})?;
            tracing::debug!(target:"connect_service::canonical_replacement",stage="mute","Replacement resource start");
            native.mute_native_audio_for_private_admission(&id,&a,invocation.clone(),deadline).await.map_err(|_|Error::forbidden())?;
            if !invocation.read.current() || activation.confirmed_generation()!=Some(policy.pending.generation()) || controller.revision()!=2 || !native.is_connection_live(&id) {return Err(Error::forbidden());}
            let observation=serde_json::json!({"candidate":id.as_str(),"generation":policy.pending.generation(),"stream_revision":controller.revision(),"streams":outcome.accepted.iter().map(|s|s.as_str()).collect::<Vec<_>>(),"resources_prepared":true,"prepared_muted":true,"media_ready":false});
            Ok((controller,observation))
        }.await;
        drop(invocation);
        let (controller, observation) = match result {
            Ok(value) => value,
            Err(error) => {
                let uncertain = Arc::new(attempt.uncertain());
                *allocated
                    .failed_start
                    .lock()
                    .map_err(|_| Error::storage())? = Some(uncertain.clone());
                saved.handle.cancel();
                let store = self.media.store.clone();
                let a = a.clone();
                store_call(move || {
                    store.native_replacement_start_uncertain_with_peer(&uncertain, proof(&a)?)
                })
                .await?;
                return Err(error);
            }
        };
        tracing::debug!(target:"connect_service::canonical_replacement",stage="capture","Replacement resource start");
        let receipt = Arc::new(attempt.started(observation)?);
        let started = Arc::new(StartedReplacement {
            receipt,
            policy,
            controller,
            intent,
            commit: Default::default(),
            phases: Default::default(),
            preparation: OnceLock::new(),
        });
        *allocated.started.lock().map_err(|_| Error::storage())? = Some(started.clone());
        let record = self
            .record_started_replacement_with_peer(&started, a.clone())
            .await?;
        tracing::debug!(target:"connect_service::canonical_replacement",stage="origin","Replacement resource start");
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = started.receipt.clone();
            store_call(move || store.native_replacement_media_origin(receipt))
        })
        .await?;
        started
            .policy
            .origin
            .set(origin)
            .map_err(|_| Error::forbidden())?;
        self.queue_replacement_media(
            saved,
            started,
            a.peer_admission().ok_or_else(Error::forbidden)?.clone(),
        );
        tracing::debug!(target:"connect_service::canonical_replacement",stage="complete","Replacement resource start");
        Ok(Reply {
            record,
            answered: true,
        })
    }
    async fn record_started_replacement_with_peer(
        &self,
        started: &StartedReplacement,
        a: Arc<AuthorizedEnvelope>,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = started.receipt.clone();
            let a = a.clone();
            store_call(move || {
                store.record_native_replacement_started_with_peer(&receipt, proof(&a)?)
            })
        })
        .await
    }
    pub(super) async fn record_started_replacement(
        &self,
        started: &StartedReplacement,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = started.receipt.clone();
            store_call(move || store.record_native_replacement_started(&receipt))
        })
        .await
    }
}
