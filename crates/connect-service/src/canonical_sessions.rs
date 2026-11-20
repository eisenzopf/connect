//! Canonical Session lifecycle commands share durable policy and receipts.
use crate::{Service, StoreInvocationAuthority};
use connect_protocol::{session_control::*, Error, Result};
use rvoip_uctp::v2::{Envelope, Payload, Scope};
pub(crate) fn terminal(state: &str) -> bool {
    matches!(state, "ended" | "rejected" | "cancelled" | "failed")
}

pub(crate) fn correlated_cancel(e: &Envelope) -> bool {
    matches!(e.payload(), Payload::SessionCancel(_))
        && e.scope(Scope::Reply).is_some()
        && e.scope(Scope::Operation).is_some()
}

pub(crate) fn command(e: &Envelope) -> Result<Option<Request>> {
    let (action, by) = match e.payload() {
        Payload::SessionInvite(v) => (
            Action::Invite {
                participants: v.to.iter().map(|id| id.as_str().into()).collect(),
                medium: v.medium.clone(),
                intent: v.intent.clone(),
                capabilities: v.capabilities_offer.clone(),
            },
            v.r#from.as_ref(),
        ),
        Payload::SessionAccept(v) => (
            Action::Accept {
                capabilities: v.capabilities_answer.clone(),
            },
            v.by.as_ref(),
        ),
        Payload::SessionReject(v) => (
            Action::Reject {
                reason_code: v.reason_code,
                reason: v.reason.clone(),
            },
            v.by.as_ref(),
        ),
        Payload::SessionCancel(v) => (
            Action::Cancel {
                reason_code: v.reason_code,
                reason: v.reason.clone(),
            },
            v.by.as_ref(),
        ),
        Payload::SessionEnd(v) => (
            Action::End {
                reason_code: v.reason_code,
                reason: v.reason.clone(),
            },
            v.by.as_ref(),
        ),
        Payload::SessionUpdate(v) => {
            use rvoip_uctp::v2::payloads::SessionUpdate;
            (
                match v {
                    SessionUpdate::ParticipantLeave { participant_id } => Action::Leave {
                        participant_id: participant_id.as_str().into(),
                    },
                    SessionUpdate::RoleChange {
                        participant_id,
                        role,
                    } => Action::Role {
                        participant_id: participant_id.as_str().into(),
                        role: role.clone(),
                    },
                    SessionUpdate::MediumUpgrade { media } => Action::Upgrade {
                        media: media
                            .iter()
                            .map(|kind| {
                                serde_json::to_value(kind)
                                    .ok()
                                    .and_then(|v| v.as_str().map(str::to_owned))
                                    .ok_or_else(Error::storage)
                            })
                            .collect::<Result<_>>()?,
                    },
                    SessionUpdate::CodecRenegotiate { codec_preferences } => Action::Renegotiate {
                        codec_preferences: codec_preferences.clone(),
                    },
                },
                None,
            )
        }
        _ => return Ok(None),
    };
    if (e.scope(Scope::Reply).is_some() && !correlated_cancel(e))
        || e.scope(Scope::Connection).is_some()
    {
        return Err(Error::new(
            "invalid-request",
            "Unexpected Session resource assertion",
        ));
    }
    let required = |scope| {
        e.scope(scope)
            .map(str::to_owned)
            .ok_or_else(|| Error::new("invalid-request", "Missing Session resource assertion"))
    };
    let mut request = Request {
        operation_id: required(Scope::Operation)?,
        conversation_id: required(Scope::Conversation)?,
        session_id: required(Scope::Session)?,
        by_participant: by.map(|id| id.as_str().into()),
        expected_revision: e.expected_revision(),
        action,
    };
    request.normalize()?;
    Ok(Some(request))
}
impl Service {
    pub(crate) async fn canonical_session_held(
        &self,
        credential: String,
        request: Request,
        authority: Option<StoreInvocationAuthority>,
        peer: Option<std::sync::Arc<rvoip_core::peer_admission::PeerAdmission>>,
        principal: Option<rvoip_core::AuthenticatedPrincipal>,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        if matches!(&request.action, Action::Renegotiate { .. }) {
            if let Some(runtime) = self.canonical_runtime.get() {
                runtime.ensure_session_codec_worker()?;
            }
        }
        let store = self.store.clone();
        let runtime = self.canonical_runtime.get().cloned();
        let result = tokio::task::spawn_blocking(move || match authority {
            Some(authority) => {
                let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
                let record =
                    store.execute_session_control_with_peer(&credential, request, proof)?;
                let mut retirement: Option<(String, Box<dyn std::any::Any + Send + Sync>)> = None;
                if let Some(runtime) = runtime {
                    if record.state == connect_protocol::transitions::OperationState::Completed {
                        let projection = store.project_session_with_peer(&credential, proof)?;
                        if terminal(&projection.session.state) {
                            retirement = Some((
                                projection.session.id.clone(),
                                Box::new(authority.retained()),
                            ));
                        }
                        if let (Some(peer), Some(principal)) = (peer, principal) {
                            runtime.attach(&projection, &principal, peer, authority)?;
                        }
                    }
                }
                Ok((record, retirement))
            }
            None => {
                let (record, projection, held) =
                    store.execute_session_control_projection(&credential, request)?;
                if let (Some(runtime), Some(projection)) = (runtime, projection.as_ref()) {
                    if projection.present && !terminal(&projection.session.state) {
                        let actor = store.revalidate(&credential)?;
                        if actor.identity_id != projection.actor.identity_id {
                            return Err(Error::forbidden());
                        }
                        let principal =
                            crate::authentication::StoreValidator::canonical(store.clone())
                                .principal(actor)
                                .map_err(|_| Error::forbidden())?;
                        runtime.project_session_container(
                            projection,
                            principal.tenant.as_deref().ok_or_else(Error::forbidden)?,
                        )?;
                    }
                }
                let retirement = projection.filter(|p| terminal(&p.session.state)).map(|p| {
                    (
                        p.session.id,
                        Box::new(held) as Box<dyn std::any::Any + Send + Sync>,
                    )
                });
                Ok((record, retirement))
            }
        })
        .await
        .map_err(|_| Error::storage())?;
        let result = match result {
            Ok((record, retirement)) => {
                if let (Some(runtime), Some((sid, _held))) =
                    (self.canonical_runtime.get(), retirement)
                {
                    runtime
                        .consultation_audiences
                        .lock()
                        .map_err(|_| Error::storage())?
                        .retain(|_, a| a.session != sid);
                    let sid = rvoip_core::ids::SessionId::from_string(sid);
                    if runtime.core.session(&sid).is_some() {
                        runtime
                            .core
                            .end_session(sid, rvoip_core::adapter::EndReason::Normal)
                            .await
                            .map_err(|_| Error::storage())?;
                    }
                }
                Ok(record)
            }
            Err(error) => Err(error),
        };
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}

#[cfg(test)]
mod http_projection_tests {
    use super::*;
    #[tokio::test]
    async fn ordinary_http_decision_projects_a_checked_empty_core_under_its_existing_fence() {
        let directory = tempfile::tempdir().expect("fixture");
        let store = std::sync::Arc::new(
            connect_store::Store::initialize(
                &directory.path().join("state.sqlite3"),
                Default::default(),
            )
            .expect("store"),
        );
        let owner = store.create_identity("owner").expect("owner").id;
        let other = store.create_identity("phone").expect("phone").id;
        let key = store
            .issue(
                &owner,
                connect_store::Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .expect("key");
        let connect_protocol::Outcome::Conversation(room) = store
            .execute(
                &key.id,
                &connect_protocol::Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        other.clone(),
                    ]),
                },
            )
            .expect("room")
        else {
            panic!("room")
        };
        let target = room
            .participants
            .iter()
            .find(|p| p.identity_id == other)
            .expect("target")
            .participant_id
            .clone();
        let service = Service::new(store);
        service.install_canonical_runtime().expect("runtime");
        let request = Request {
            operation_id: "http-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "http-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![target],
                medium: "voice".into(),
                intent: "phone-callback".into(),
                capabilities: serde_json::json!({"audio":["g.711-mu"]}),
            },
        };
        let receipt = service
            .canonical_session_held(key.id.clone(), request.clone(), None, None, None)
            .await
            .expect("HTTP admission");
        assert_eq!(
            receipt.state,
            connect_protocol::transitions::OperationState::Completed
        );
        let runtime = service.canonical_runtime.get().expect("runtime");
        let sid = rvoip_core::ids::SessionId::from_string("http-session".to_owned());
        assert_eq!(
            runtime
                .core
                .participant_revision(&sid)
                .expect("checked coordinator"),
            1
        );
        assert_eq!(
            runtime
                .core
                .participant_capacity(&sid)
                .expect("empty capacity"),
            (0, 0)
        );
        let actual = runtime.core.session(&sid).expect("same Session");
        {
            let actual = actual.read().expect("Session");
            assert!(actual.participants.is_empty());
            assert!(actual.connections.is_empty());
        }
        assert_eq!(
            service
                .canonical_runtime_snapshot()
                .expect("snapshot")
                .physical_peers,
            0
        );
        assert_eq!(
            service
                .canonical_session_held(key.id.clone(), request, None, None, None)
                .await
                .expect("replay"),
            receipt
        );
        assert_eq!(
            runtime
                .core
                .participant_revision(&sid)
                .expect("same coordinator"),
            1
        );
        service.drain_canonical_runtime().await.unwrap();
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn actual_model_busy_scalar_wait_yields_and_initializer_keeps_live_terminal_and_missing_distinct(
    ) {
        let directory = tempfile::tempdir().expect("fixture");
        let store = std::sync::Arc::new(
            connect_store::Store::initialize(
                &directory.path().join("state.sqlite3"),
                Default::default(),
            )
            .expect("store"),
        );
        let owner = store.create_identity("owner").expect("owner").id;
        let other = store.create_identity("phone").expect("phone").id;
        let key = store
            .issue(
                &owner,
                connect_store::Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .expect("key");
        let connect_protocol::Outcome::Conversation(room) = store
            .execute(
                &key.id,
                &connect_protocol::Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        other.clone(),
                    ]),
                },
            )
            .expect("room")
        else {
            panic!("room")
        };
        let target = room
            .participants
            .iter()
            .find(|p| p.identity_id == other)
            .expect("target")
            .participant_id
            .clone();
        let service = Service::new(store);
        service.install_canonical_runtime().expect("runtime");
        let request = Request {
            operation_id: "http-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "http-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![target],
                medium: "voice".into(),
                intent: "phone-callback".into(),
                capabilities: serde_json::json!({"audio":["g.711-mu"]}),
            },
        };
        let receipt = service
            .canonical_session_held(key.id.clone(), request.clone(), None, None, None)
            .await
            .expect("HTTP admission");
        assert_eq!(
            receipt.state,
            connect_protocol::transitions::OperationState::Completed
        );
        let runtime = service.canonical_runtime.get().unwrap().clone();
        let sid = rvoip_core::ids::SessionId::from_string("http-session");
        let missing = rvoip_core::ids::SessionId::from_string("never-projected-session");
        assert!(!runtime
            .core
            .try_participant_control_enabled(&missing)
            .unwrap());
        assert!(runtime.core.try_participant_control_enabled(&sid).unwrap());
        let (same, projection, held) = service
            .store
            .execute_session_control_projection(&key.id, request)
            .unwrap();
        assert_eq!(same, receipt);
        let projection = projection.unwrap();
        drop(held); // Never reacquire Store authority beneath the replay writer.
        let actor = service.store.revalidate(&key.id).unwrap();
        let principal = crate::authentication::StoreValidator::canonical(service.store.clone())
            .principal(actor)
            .unwrap();
        struct ModelHolder {
            release: Option<std::sync::mpsc::Sender<()>>,
            owner: Option<std::thread::JoinHandle<bool>>,
        }
        impl ModelHolder {
            fn release(&mut self) {
                self.release.take().unwrap().send(()).unwrap();
            }
            fn close(&mut self) -> bool {
                self.owner.take().is_some_and(|owner| owner.join().unwrap())
            }
        }
        impl Drop for ModelHolder {
            fn drop(&mut self) {
                if let Some(release) = self.release.take() {
                    let _ = release.send(());
                }
                if let Some(owner) = self.owner.take() {
                    let _ = owner.join();
                }
            }
        }
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let owner = runtime.core.clone();
        let held_sid = sid.clone();
        let owner = std::thread::spawn(move || {
            owner
                .with_participant_model_state_exclusion_for_test(&held_sid, || {
                    entered.send(()).unwrap();
                    released
                        .recv_timeout(std::time::Duration::from_secs(3))
                        .is_ok()
                })
                .unwrap()
        });
        let mut holder = ModelHolder {
            release: Some(release),
            owner: Some(owner),
        };
        ready
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert!(matches!(
            runtime.core.try_participant_revision(&sid),
            Err(
                rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                    rvoip_core::staged_streams::StreamControlError::MetadataBusy
                )
            )
        ));
        assert!(
            runtime.core.try_participant_control_enabled(&sid).unwrap(),
            "existence must not read the held live model or initialize another coordinator"
        );
        runtime
            .project_session_container(&projection, principal.tenant.as_deref().unwrap())
            .unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        let stopped = tokio_util::sync::CancellationToken::new();
        stopped.cancel();
        assert_eq!(
            runtime
                .participant_revision_until(&sid, until, &stopped)
                .await
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(
            runtime
                .participant_revision_until(&sid, tokio::time::Instant::now(), &cancel)
                .await
                .unwrap_err()
                .code,
            "forbidden"
        );
        let heartbeat = std::sync::atomic::AtomicUsize::new(0);
        let scalar = runtime.participant_revision_until(&sid, until, &cancel);
        let healthy = async {
            tokio::task::yield_now().await;
            assert!(matches!(
                runtime.core.try_participant_revision(&sid),
                Err(
                    rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                        rvoip_core::staged_streams::StreamControlError::MetadataBusy
                    )
                )
            ));
            heartbeat.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            holder.release();
        };
        let (revision, ()) = tokio::join!(scalar, healthy);
        assert_eq!(revision.unwrap(), 1);
        assert_eq!(heartbeat.load(std::sync::atomic::Ordering::Acquire), 1);
        assert!(
            holder.close(),
            "original actual holder must be released by concurrent async work, never by its fuse"
        );
        assert_eq!(
            runtime.core.try_participant_revision(&sid).unwrap(),
            1,
            "same original coordinator, no enable/admit replay"
        );
        runtime
            .core
            .end_session(sid.clone(), rvoip_core::adapter::EndReason::Normal)
            .await
            .unwrap();
        let terminal = runtime.core.participant_revision(&sid).unwrap();
        assert!(
            runtime.core.try_participant_control_enabled(&sid).unwrap(),
            "actual terminal scalar is not missing and cannot reinitialize"
        );
        assert_eq!(
            runtime.core.try_participant_revision(&sid).unwrap(),
            terminal
        );
        assert!(!runtime
            .core
            .try_participant_control_enabled(&missing)
            .unwrap());
        service.drain_canonical_runtime().await.unwrap();
    }
}
