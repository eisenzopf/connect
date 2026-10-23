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
        service.drain_canonical_runtime().await;
    }
}
