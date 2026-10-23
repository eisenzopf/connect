//! Controlled provider custody/content probes. These do not call Vapi or claim
//! actual provider initialization or native audio behavior.
use super::*;
use crate::{
    NativeConsultationOrigin, NativeMediaOrigin, PrivateProviderApplied, PrivateProviderContext,
};
use std::sync::Arc;
type ProviderFixture = (
    Fixture,
    Vec<Arc<NativeMediaOrigin>>,
    Arc<NativeMediaOrigin>,
    NativeConsultationOrigin,
    String,
    String,
    crate::NativeConsultationApplied,
);
fn fixture() -> ProviderFixture {
    let (f, sources, candidate, origin, execution, native_receipt) =
        consultation_audiences::established_fixture();
    let endpoint_id = ai_endpoint(&f, &candidate);

    (
        f,
        sources,
        candidate,
        origin,
        execution,
        endpoint_id,
        native_receipt,
    )
}
fn ai_endpoint(f: &Fixture, candidate: &Arc<NativeMediaOrigin>) -> String {
    ai_endpoint_profile(f, candidate, true)
}
fn ai_endpoint_profile(f: &Fixture, candidate: &Arc<NativeMediaOrigin>, private: bool) -> String {
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE identities SET kind='ai' WHERE id=(SELECT identity_id FROM connections WHERE id=?1)",[candidate.connection_id()]).unwrap();
        c.execute(
            "UPDATE participants SET kind='ai' WHERE participant_id=?1",
            [candidate.participant_id()],
        )
        .unwrap();
    }
    let identity = f.store.revalidate(&f.keys[2]).unwrap().identity_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: if private {
                    "private-provider-endpoint"
                } else {
                    "public-provider-endpoint"
                }
                .into(),
                identity_id: identity,
                device: DeviceSpec {
                    id: if private {
                        "private-provider-device"
                    } else {
                        "public-provider-device"
                    }
                    .into(),
                    kind: "service".into(),
                    platform: "isolated-provider-fixture".into(),
                },
                transport: EndpointTransport::VapiVoice,
                address: if private {
                    "fixture-assistant"
                } else {
                    "fixture-public-assistant"
                }
                .into(),
                capabilities: if private {
                    vec!["audio".into(), "private-consultation".into()]
                } else {
                    vec!["audio".into()]
                },
                secret_ref: Some("fixture_private_vapi_key".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store.connection().unwrap().execute("UPDATE endpoints SET verification_state='verified',verification_evidence='controlled-fixture',verified_at=?2 WHERE id=?1",params![endpoint_id,crate::now()]).unwrap();
    endpoint_id
}
fn fresh_fixture() -> (
    Fixture,
    Vec<Arc<NativeMediaOrigin>>,
    Arc<NativeMediaOrigin>,
    NativeConsultationOrigin,
    String,
) {
    let (f, sources, _, origin) = consultation_reads::fixture();
    let candidate = consultation_audiences::candidate(&f, &origin);
    let endpoint = ai_endpoint(&f, &candidate);
    (f, sources, candidate, origin, endpoint)
}
fn media_plan(
    origin: &NativeConsultationOrigin,
    sources: &[Arc<NativeMediaOrigin>],
    candidate: &Arc<NativeMediaOrigin>,
) -> Value {
    let (p, r, c, outside) = (
        sources[0].runtime_connection_id(),
        sources[1].runtime_connection_id(),
        candidate.runtime_connection_id(),
        sources[2].runtime_connection_id(),
    );
    json!({"operation_id":origin.record_id(),"audience_revision":0,"private_connections":[p,c],"receivers":{p:[c],c:[p],r:[]},"activate_muted":c,"expected_receiver_revisions":{p:1,r:1,c:2,outside:1}})
}
fn read(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    candidate: &Arc<NativeMediaOrigin>,
    origin: &NativeConsultationOrigin,
) -> crate::NativeConsultationRead {
    f.store
        .try_native_consultation_scene_read(
            origin,
            &sources[1],
            &[sources[0].clone(), candidate.clone()],
            &[sources[2].clone()],
        )
        .unwrap()
}
fn prepared(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    candidate: &Arc<NativeMediaOrigin>,
    origin: &NativeConsultationOrigin,
    endpoint: &str,
) -> PrivateProviderContext {
    f.store
        .prepare_private_provider_context(read(f, sources, candidate, origin), &f.keys[0], endpoint)
        .unwrap()
}
fn created(
    f: &Fixture,
    context: &PrivateProviderContext,
    reference: &str,
) -> crate::PrivateProviderCreated {
    let read = f.store.try_private_provider_read(context).unwrap();
    assert_eq!(read.assistant_id().unwrap(), "fixture-assistant");
    assert_eq!(read.secret_ref().unwrap(), "fixture_private_vapi_key");
    assert_eq!(read.json().unwrap_err().code, "not-ready");
    let (attempt, read) = f
        .store
        .begin_private_provider_context(context.clone(), read)
        .unwrap();
    assert!(read.json().is_err());
    drop(read);
    let receipt = attempt.created(reference.into()).unwrap();
    f.store.record_private_provider_created(&receipt).unwrap();
    receipt
}
fn export_attempt(f: &Fixture, context: &PrivateProviderContext) -> crate::PrivateProviderExport {
    let read = f.store.try_private_provider_read(context).unwrap();
    assert!(read.provider_ref().is_ok());
    assert!(read.json().is_ok());
    let (attempt, read) = f
        .store
        .begin_private_provider_export(context.clone(), read)
        .unwrap();
    drop(read);
    attempt
}
fn delivered(f: &Fixture, context: &PrivateProviderContext) -> PrivateProviderApplied {
    let _creation = created(f, context, "actual-controlled-fresh-private-context");
    let proof = export_attempt(f, context).delivered();
    f.store.record_private_provider_delivered(&proof).unwrap();
    proof
}

#[test]
fn protected_initializer_target_is_current_original_member_metadata_without_an_idle_grant() {
    let (f, _, candidate, origin, _) = fresh_fixture();
    let view = f.store.native_consultation_target(&origin).unwrap();
    assert_eq!(
        view.identity_id,
        f.store.revalidate(&f.keys[2]).unwrap().identity_id
    );
    assert_eq!(view.participant_id, candidate.participant_id());
    assert_eq!(view.membership_epoch, 1);
    assert_eq!(view.kind, "ai");
    assert!(view.present && view.registered);
    let (other, _, _, _, _) = fresh_fixture();
    assert!(other.store.native_consultation_target(&origin).is_err());
    f.store.connection().unwrap().execute("UPDATE session_participants SET state='invited' WHERE session_id='session' AND participant_id=?1",[candidate.participant_id()]).unwrap();
    assert!(!f.store.native_consultation_target(&origin).unwrap().present);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    assert!(f.store.native_consultation_target(&origin).is_err());
    assert_eq!(view.membership_epoch, 1); // The old facts cannot become a grant.
}

#[test]
fn captured_initialization_failure_records_only_an_exact_zero_io_intention() {
    use connect_protocol::transitions::{OperationAction, OperationState};
    for started in [false, true] {
        let (f, sources, candidate, origin, endpoint) = fresh_fixture();
        let _pending = if started {
            let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
            Some(created(&f, &context, "initializer-started-provider"))
        } else {
            None
        };
        let (other, _, _, _, _) = fresh_fixture();
        assert!(other
            .store
            .fail_native_consultation_initialization(&origin, "forbidden")
            .is_err());
        assert_eq!(
            f.store
                .fail_native_consultation_initialization(&origin, "forbidden")
                .unwrap(),
            !started
        );
        let get = f
            .store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap();
        if !started {
            assert_eq!(get.state, OperationState::Failed);
            assert!(!f
                .store
                .fail_native_consultation_initialization(&origin, "forbidden")
                .unwrap());
        } else {
            assert_ne!(get.state, OperationState::Failed);
        }
        assert!(f
            .store
            .try_native_media_read(&sources[0], Some(&sources[1]))
            .is_ok());
    }
}

#[test]
fn initialization_failure_receipt_survives_source_revocation_and_rolls_back_recording_failure() {
    let (f, _, _, origin, _) = fresh_fixture();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER refuse_initialization_failure BEFORE UPDATE OF state ON operations WHEN NEW.state='failed' BEGIN SELECT RAISE(ABORT,'controlled record failure'); END;").unwrap();
    assert!(f
        .store
        .fail_native_consultation_initialization(&origin, "forbidden")
        .is_err());
    {
        let c = f.store.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [origin.record_id()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "accepted"
        );
        c.execute_batch("DROP TRIGGER refuse_initialization_failure")
            .unwrap();
    }
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    assert!(f
        .store
        .fail_native_consultation_initialization(&origin, "forbidden")
        .unwrap());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [origin.record_id()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "failed"
    );
}

#[test]
fn initialization_cancellation_settles_only_after_captured_provider_close_without_native_io() {
    use connect_protocol::transitions::{OperationAction, OperationState};
    let (f, sources, candidate, origin, endpoint) = fresh_fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let capture = created(&f, &context, "cancelled-initialization-instance");
    let cancel = OperationAction::Cancel {
        operation_id: "cancel-before-native-initialization".into(),
        target_operation_id: origin.request().operation_id.clone(),
        reason: None,
    };
    f.store
        .execute_operation_action(&f.keys[0], cancel.clone())
        .unwrap();
    assert!(!f
        .store
        .fail_native_consultation_initialization(&origin, "forbidden")
        .unwrap());
    let get = || {
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap()
    };
    assert_eq!(get().state, OperationState::Unknown);
    f.store
        .record_private_provider_created_closed(&capture)
        .unwrap();
    assert!(f
        .store
        .fail_native_consultation_initialization(&origin, "forbidden")
        .unwrap());
    let result = get();
    assert_eq!(result.state, OperationState::Cancelled);
    assert!(result.cancel_requested);
    let problem: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT problem_json FROM operations WHERE record_id=?1",
            [origin.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&problem).unwrap()["code"],
        "cancelled"
    );
    assert!(!f
        .store
        .fail_native_consultation_initialization(&origin, "forbidden")
        .unwrap());
    f.store
        .execute_operation_action(&f.keys[0], cancel)
        .unwrap();
    assert_eq!(get().state, OperationState::Cancelled);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_consultation_attempts WHERE operation_record_id=?1",
                [origin.record_id()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn stopped_native_preparation_keeps_provider_uncertainty_until_actual_captured_termination() {
    use connect_protocol::transitions::{OperationAction, OperationState};
    for started in [false, true] {
        let (f, sources, candidate, origin, endpoint) = fresh_fixture();
        let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
        let receipt = delivered(&f, &context);
        let lease = f
            .store
            .prepare_native_consultation(
                read(&f, &sources, &candidate, &origin),
                media_plan(&origin, &sources, &candidate),
            )
            .unwrap()
            .unwrap();
        let proof = lease.preparation();
        if started {
            let read = f.store.native_consultation_attempt_read(&lease).unwrap();
            let (attempt, read) = f.store.begin_native_consultation(lease, read).unwrap();
            drop(read);
            f.store.native_consultation_uncertain(attempt).unwrap();
        }
        assert_eq!(
            f.store
                .settle_native_consultation_preparation(&proof)
                .unwrap(),
            !started
        );
        let get = || {
            f.store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Get {
                        operation_id: origin.request().operation_id.clone(),
                    },
                )
                .unwrap()
        };
        assert_eq!(get().state, OperationState::Unknown);
        if !started {
            assert!(f
                .store
                .try_native_media_read(&sources[0], Some(&sources[1]))
                .is_ok());
        }
        f.store.record_private_provider_closed(&receipt).unwrap();
        assert_eq!(
            get().state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        if started {
            assert!(f
                .store
                .try_native_media_read(&sources[0], Some(&sources[1]))
                .is_err());
        }
        assert!(!f
            .store
            .settle_native_consultation_preparation(&proof)
            .unwrap());
    }
}

#[test]
fn expired_unstarted_native_preparation_keeps_unconfirmed_provider_cleanup_unknown() {
    use connect_protocol::transitions::{OperationAction, OperationState};
    let (f, sources, candidate, origin, endpoint) = fresh_fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let receipt = delivered(&f, &context);
    let _lease = f
        .store
        .prepare_native_consultation(
            read(&f, &sources, &candidate, &origin),
            media_plan(&origin, &sources, &candidate),
        )
        .unwrap()
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET lease_expires_at=?2 WHERE record_id=?1",
            params![origin.record_id(), crate::now() - 1],
        )
        .unwrap();
    let get = || {
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap()
    };
    let pending = get();
    assert_eq!(pending.state, OperationState::Unknown);
    assert_eq!(
        pending.effects[0].state,
        connect_protocol::transitions::EffectState::Cancelled
    );
    assert!(f
        .store
        .try_native_media_read(&sources[0], Some(&sources[1]))
        .is_ok());
    f.store.record_private_provider_closed(&receipt).unwrap();
    assert_eq!(get().state, OperationState::Failed);
}

#[test]
fn private_provider_context_requires_current_private_source_and_verified_ai_destination_before_reservation(
) {
    for refusal in [
        "remote-owner",
        "wrong-endpoint",
        "unverified",
        "context-permission",
        "delegate-permission",
        "public-provider-profile",
    ] {
        let (f, sources, candidate, origin, _, endpoint, _) = fixture();
        let mut key = f.keys[0].clone();
        let mut target = endpoint.clone();
        match refusal {
            "remote-owner" => key = f.keys[1].clone(),
            "wrong-endpoint" => target = f.endpoints[1].clone(),
            "public-provider-profile" => {
                target = ai_endpoint_profile(&f, &candidate, false);
            }
            "unverified" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE endpoints SET verification_state='unverified' WHERE id=?1",
                        [&endpoint],
                    )
                    .unwrap();
            }
            "context-permission" => {
                let actor = f.store.revalidate(&key).unwrap();
                let mut scope = actor.scope;
                scope.operations.retain(|p| p != "context_view");
                key = f
                    .store
                    .issue(&actor.identity_id, scope, crate::now() + 600)
                    .unwrap()
                    .id;
            }
            "delegate-permission" => {
                let mut scope = f.store.revalidate(&f.keys[2]).unwrap().scope;
                let c = f.store.connection().unwrap();
                scope.operations.retain(|p| p != "context_view");
                c.execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    params![f.keys[2], serde_json::to_string(&scope.operations).unwrap()],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = f.count("context_deliveries");
        assert!(
            f.store
                .prepare_private_provider_context(
                    read(&f, &sources, &candidate, &origin),
                    &key,
                    &target
                )
                .is_err(),
            "{refusal}"
        );
        assert_eq!(f.count("context_deliveries"), before);
        assert_eq!(f.count("private_provider_contexts"), 0);
    }
}
#[test]
fn private_provider_exact_selection_and_one_attempt_cannot_complete_through_generic_context_api() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "shared-memory".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "authorized shared history",
                ),
            },
        )
        .unwrap();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    assert!(f
        .store
        .finish_context_delivery(
            context.delivery_id(),
            "delivered",
            Some("forged-public-memory")
        )
        .is_err());
    let read = f.store.try_private_provider_read(&context).unwrap();
    assert!(read.json().is_err());
    assert!(!format!("{read:?}").contains("authorized shared history"));
    let duplicate = f.store.try_private_provider_read(&context).unwrap();
    let (attempt, read) = f
        .store
        .begin_private_provider_context(context.clone(), read)
        .unwrap();
    drop(read);
    assert!(f
        .store
        .begin_private_provider_context(context.clone(), duplicate)
        .is_err());
    assert!(f.store.try_private_provider_read(&context).is_err());
    let creation = attempt.created("fresh-private-memory".into()).unwrap();
    f.store.record_private_provider_created(&creation).unwrap();
    f.store.record_private_provider_created(&creation).unwrap();
    let read = f.store.try_private_provider_read(&context).unwrap();
    assert!(read.json().unwrap().contains("authorized shared history"));
    assert_eq!(read.provider_ref().unwrap(), "fresh-private-memory");
    let duplicate = f.store.try_private_provider_read(&context).unwrap();
    let (export, read) = f
        .store
        .begin_private_provider_export(context.clone(), read)
        .unwrap();
    drop(read);
    assert!(f
        .store
        .begin_private_provider_export(context.clone(), duplicate)
        .is_err());
    assert!(f.store.try_private_provider_read(&context).is_err());
    let receipt = export.delivered();
    f.store.record_private_provider_delivered(&receipt).unwrap();
    f.store.record_private_provider_delivered(&receipt).unwrap();
    let c = f.store.connection().unwrap();
    let audience: Vec<String> = {
        let mut st=c.prepare("SELECT participant_id FROM resource_audiences WHERE kind='context_delivery' AND id=?1 ORDER BY participant_id").unwrap();
        st.query_map([context.delivery_id()], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(audience.len(), 2);
    assert!(audience.contains(&f.pids[0]));
    assert!(audience.contains(&f.pids[2]));
    assert!(!audience.contains(&f.pids[1]));
}

#[test]
fn private_provider_final_transcripts_keep_original_audience_and_never_enter_public_provider_memory(
) {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let receipt = delivered(&f, &context);
    assert!(f
        .store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "interim",
            "user",
            "partial",
            "not-final"
        )
        .unwrap()
        .is_none());
    let message = f
        .store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "transcript-1",
            "user",
            "final",
            "private user marker",
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .record_private_provider_transcript(
                &context,
                &receipt,
                "transcript-1",
                "user",
                "final",
                "private user marker"
            )
            .unwrap()
            .unwrap(),
        message
    );
    assert_eq!(
        f.store
            .record_private_provider_transcript(
                &context,
                &receipt,
                "transcript-1",
                "user",
                "final",
                "changed marker"
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let response = f
        .store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "transcript-2",
            "assistant",
            "final",
            "private assistant marker",
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        response.sender,
        f.store.revalidate(&f.keys[2]).unwrap().identity_id
    );
    assert_eq!(message.audience.len(), 2);
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    let ai = f.store.revalidate(&f.keys[2]).unwrap();
    let remote = f.store.revalidate(&f.keys[1]).unwrap();
    let c = f.store.connection().unwrap();
    assert!(crate::audience::visible(
        &c,
        &owner,
        &f.cid,
        connect_protocol::membership::ContentKind::Message,
        &message.id
    )
    .unwrap());
    assert!(!crate::audience::visible(
        &c,
        &remote,
        &f.cid,
        connect_protocol::membership::ContentKind::Message,
        &message.id
    )
    .unwrap());
    let public = crate::context::view_for_scope(
        &f.store,
        &c,
        &owner,
        &f.cid,
        1000,
        Some(&ai.identity_id),
        Some(&ai),
    )
    .unwrap();
    assert!(!serde_json::to_string(&public)
        .unwrap()
        .contains("private user marker"));
    assert!(!serde_json::to_string(&public)
        .unwrap()
        .contains("private assistant marker"));
    let private = crate::context::view_for_private_scope_at(
        &f.store,
        &c,
        &owner,
        &f.cid,
        1000,
        Some(&ai.identity_id),
        Some(&ai),
        None,
        Some(origin.record_id()),
    )
    .unwrap();
    assert!(serde_json::to_string(&private)
        .unwrap()
        .contains("private user marker"));
    let ordinary =
        crate::context::view_for_scope(&f.store, &c, &owner, &f.cid, 1000, None, None).unwrap();
    assert!(serde_json::to_string(&ordinary)
        .unwrap()
        .contains("private user marker"));
    for item in private.items.iter().filter(|i| i.source_id == message.id) {
        assert!(crate::context::authorize_export_event(
            &c,
            &owner,
            &ai.identity_id,
            Some(&ai),
            &f.cid,
            item.sequence
        )
        .is_err());
    }
}
#[test]
fn private_provider_captured_delivery_survives_revocation_but_cannot_authorize_new_transcripts() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let _creation = created(&f, &context, "captured-before-revocation");
    let receipt = export_attempt(&f, &context).delivered();
    let operator = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&operator, &f.keys[0]).unwrap();
    f.store.record_private_provider_delivered(&receipt).unwrap();
    assert!(f
        .store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "late",
            "user",
            "final",
            "must not publish"
        )
        .is_err());
    assert_eq!(f.count("private_content_sources"), 1);
}

#[test]
fn private_provider_uncertainty_reserves_the_original_context_and_never_reexports() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let read = f.store.try_private_provider_read(&context).unwrap();
    let (attempt, read) = f
        .store
        .begin_private_provider_context(context.clone(), read)
        .unwrap();
    drop(read);
    f.store.private_provider_uncertain(&attempt).unwrap();
    assert!(f.store.try_private_provider_read(&context).is_err());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM context_deliveries WHERE id=?1",
                [context.delivery_id()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "uncertain"
    );
}

#[test]
fn private_ai_completion_requires_independent_context_and_decoded_audio_then_follows_current_physical_grants(
) {
    let (f, sources, candidate, origin, execution, endpoint, native_receipt) = fixture();
    f.store
        .record_native_consultation_decoded(&native_receipt)
        .unwrap();
    assert!(f
        .store
        .complete_native_consultation(&native_receipt)
        .unwrap()
        .is_none());
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    assert!(f
        .store
        .complete_native_consultation(&native_receipt)
        .unwrap()
        .is_none());
    let proof = delivered(&f, &context);
    let completed = f
        .store
        .complete_native_consultation(&native_receipt)
        .unwrap()
        .unwrap();
    assert_eq!(
        completed.state,
        connect_protocol::transitions::OperationState::Completed
    );
    let operator = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&operator, &execution).unwrap();
    let transcript = f
        .store
        .record_private_provider_transcript(
            &context,
            &proof,
            "after-source-revocation",
            "assistant",
            "final",
            "private established provider",
        )
        .unwrap()
        .unwrap();
    assert_eq!(transcript.audience.len(), 2);
    f.store.revoke(&operator, &f.keys[2]).unwrap();
    assert!(f
        .store
        .record_private_provider_transcript(
            &context,
            &proof,
            "after-native-revocation",
            "assistant",
            "final",
            "must not publish"
        )
        .is_err());
}

#[test]
fn private_provider_context_precedes_native_ai_audience_attempt_and_cancellation_preserves_uncertainty(
) {
    for started in [false, true] {
        let (f, sources, candidate, origin, endpoint) = fresh_fixture();
        let before = f.count("operation_effects");
        assert_eq!(
            f.store
                .prepare_native_consultation(
                    read(&f, &sources, &candidate, &origin),
                    media_plan(&origin, &sources, &candidate)
                )
                .unwrap_err()
                .code,
            "not-ready"
        );
        assert_eq!(f.count("operation_effects"), before);
        let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
        let attempt = if started {
            let read = f.store.try_private_provider_read(&context).unwrap();
            let (attempt, read) = f
                .store
                .begin_private_provider_context(context.clone(), read)
                .unwrap();
            drop(read);
            Some(attempt)
        } else {
            None
        };
        assert!(f
            .store
            .claim_transition(&f.admin, &origin.request().operation_id, 60)
            .unwrap()
            .is_none());
        assert_eq!(
            f.store
                .claim_transition_cleanup(&f.admin, &origin.request().operation_id, 60)
                .unwrap_err()
                .code,
            "conflict"
        );
        f.store
            .execute_operation_action(
                &f.keys[0],
                connect_protocol::transitions::OperationAction::Cancel {
                    operation_id: "cancel-private-provider".into(),
                    target_operation_id: origin.request().operation_id.clone(),
                    reason: None,
                },
            )
            .unwrap();
        let state = f
            .store
            .execute_operation_action(
                &f.keys[0],
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            state.state,
            if started {
                connect_protocol::transitions::OperationState::Unknown
            } else {
                connect_protocol::transitions::OperationState::Cancelled
            }
        );
        assert!(f.store.try_private_provider_read(&context).is_err());
        if let Some(attempt) = attempt {
            let capture = attempt
                .created("late-controlled-initialization".into())
                .unwrap();
            f.store.record_private_provider_created(&capture).unwrap();
            assert!(f.store.try_private_provider_read(&context).is_err());
            f.store
                .record_private_provider_created_closed(&capture)
                .unwrap();
            f.store
                .record_private_provider_created_closed(&capture)
                .unwrap();
            f.store
                .execute_operation_action(
                    &f.keys[0],
                    connect_protocol::transitions::OperationAction::Cancel {
                        operation_id: "settle-closed-private-provider".into(),
                        target_operation_id: origin.request().operation_id.clone(),
                        reason: None,
                    },
                )
                .unwrap();
            let settled = f
                .store
                .execute_operation_action(
                    &f.keys[0],
                    connect_protocol::transitions::OperationAction::Get {
                        operation_id: origin.request().operation_id.clone(),
                    },
                )
                .unwrap();
            assert_eq!(
                settled.state,
                connect_protocol::transitions::OperationState::Cancelled
            );
        }
    }
}
#[test]
fn private_provider_confirmed_initialization_allows_one_native_ai_preparation() {
    let (f, sources, candidate, origin, endpoint) = fresh_fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let receipt = delivered(&f, &context);
    let lease = f
        .store
        .prepare_native_consultation(
            read(&f, &sources, &candidate, &origin),
            media_plan(&origin, &sources, &candidate),
        )
        .unwrap()
        .unwrap();
    assert_eq!(lease.record_id(), origin.record_id());
    f.store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "pre-media",
            "assistant",
            "final",
            "private configured provider",
        )
        .unwrap();
    let held = f.store.native_consultation_attempt_read(&lease).unwrap();
    f.store.record_private_provider_closed(&receipt).unwrap();
    assert!(f.store.begin_native_consultation(lease, held).is_err());
    assert!(f
        .store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "closed",
            "assistant",
            "final",
            "must not publish"
        )
        .is_err());
}

#[test]
fn private_provider_context_source_loss_recovery_distinguishes_unstarted_from_possible_creation() {
    for started in [false, true] {
        let (f, sources, candidate, origin, endpoint) = fresh_fixture();
        let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
        if started {
            let read = f.store.try_private_provider_read(&context).unwrap();
            let (attempt, read) = f
                .store
                .begin_private_provider_context(context, read)
                .unwrap();
            drop(read);
            drop(attempt);
        }
        let operator = f.store.revalidate(&f.admin).unwrap();
        f.store.revoke(&operator, &f.keys[2]).unwrap();
        let status = f
            .store
            .execute_operation_action(
                &f.keys[0],
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            status.state,
            if started {
                connect_protocol::transitions::OperationState::Unknown
            } else {
                connect_protocol::transitions::OperationState::Failed
            }
        );
        assert_eq!(f.count("native_consultation_attempts"), 0);
    }
}
#[test]
fn private_provider_capture_rolls_back_recording_failure_then_records_exact_result_once() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let _creation = created(&f, &context, "captured-controlled-context");
    let receipt = export_attempt(&f, &context).delivered();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_private_provider_capture BEFORE UPDATE OF state ON context_deliveries WHEN NEW.state='delivered' BEGIN SELECT RAISE(ABORT,'fixture context record unavailable'); END;").unwrap();
    assert!(f.store.record_private_provider_delivered(&receipt).is_err());
    {
        let c = f.store.connection().unwrap();
        assert!(c
            .query_row(
                "SELECT delivered_at FROM private_provider_contexts WHERE delivery_id=?1",
                [context.delivery_id()],
                |r| r.get::<_, Option<i64>>(0)
            )
            .unwrap()
            .is_none());
        c.execute_batch("DROP TRIGGER fixture_private_provider_capture")
            .unwrap();
    }
    f.store.record_private_provider_delivered(&receipt).unwrap();
    f.store.record_private_provider_delivered(&receipt).unwrap();
    assert!(f.store.try_private_provider_read(&context).is_err());
}

#[test]
fn private_provider_current_export_reader_blocks_revocation_then_releases_it_without_idle_grants() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let _creation = created(&f, &context, "before-export-writer-ordering");
    let read = f.store.try_private_provider_read(&context).unwrap();
    let f = Arc::new(f);
    let operator = f.store.revalidate(&f.admin).unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let (sender, result) = std::sync::mpsc::channel();
    let worker = f.clone();
    let thread = std::thread::spawn(move || {
        started.send(()).unwrap();
        sender
            .send(worker.store.revoke(&operator, &worker.keys[0]))
            .unwrap();
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(result
        .recv_timeout(std::time::Duration::from_millis(60))
        .is_err());
    assert!(read.current());
    drop(read);
    result
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap()
        .unwrap();
    thread.join().unwrap();
    assert!(f.store.try_private_provider_read(&context).is_err());
}

#[test]
fn private_provider_reused_public_memory_refuses_before_private_json_is_exportable() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    f.store.connection().unwrap().execute("INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,provider_ref,created_at,updated_at,sequence,owner_id) VALUES('old-public-context',?1,'session','vapi',0,'{}','delivered','public-memory',?2,?2,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries),?3)",params![f.cid,crate::now(),owner.identity_id]).unwrap();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let read = f.store.try_private_provider_read(&context).unwrap();
    assert!(read.json().is_err());
    let (attempt, read) = f
        .store
        .begin_private_provider_context(context.clone(), read)
        .unwrap();
    drop(read);
    let creation = attempt.created("public-memory".into()).unwrap();
    assert_eq!(
        f.store
            .record_private_provider_created(&creation)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(f.store.try_private_provider_read(&context).is_err());
    assert!(f
        .store
        .record_private_provider_created_closed(&creation)
        .is_err());
    f.store
        .private_provider_created_uncertain(&creation)
        .unwrap();
    f.store
        .private_provider_created_uncertain(&creation)
        .unwrap();
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM context_deliveries WHERE id='old-public-context'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "delivered"
    );
    assert_eq!(
        c.query_row(
            "SELECT export_started FROM private_provider_contexts WHERE delivery_id=?1",
            [context.delivery_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn private_provider_late_export_ack_records_only_the_original_reserved_instance_after_uncertainty()
{
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let _creation = created(&f, &context, "one-fresh-private-instance");
    let export = export_attempt(&f, &context);
    f.store.private_provider_export_uncertain(&export).unwrap();
    assert!(f.store.try_private_provider_read(&context).is_err());
    let receipt = export.delivered();
    f.store.record_private_provider_delivered(&receipt).unwrap();
    f.store.record_private_provider_delivered(&receipt).unwrap();
    assert!(f.store.try_private_provider_read(&context).is_err());
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT provider_ref FROM context_deliveries WHERE id=?1",
            [context.delivery_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "one-fresh-private-instance"
    );
}

#[test]
fn private_provider_lost_creation_worker_expires_as_uncertain_without_repeating_creation() {
    let (f, sources, candidate, origin, endpoint) = fresh_fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let read = f.store.try_private_provider_read(&context).unwrap();
    let expires = read.expires_at();
    let (attempt, read) = f
        .store
        .begin_private_provider_context(context.clone(), read)
        .unwrap();
    drop(read);
    drop(attempt);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT creation_expires_at FROM private_provider_contexts WHERE delivery_id=?1",
                [context.delivery_id()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        expires
    );
    while crate::now() <= expires {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let record = f
        .store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Get {
                operation_id: origin.request().operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        record.state,
        connect_protocol::transitions::OperationState::Unknown
    );
    assert!(f.store.try_private_provider_read(&context).is_err());
    assert_eq!(f.count("native_consultation_attempts"), 0);
}

#[test]
fn native_consultation_cannot_turn_an_existing_public_context_into_a_private_attempt() {
    let (f, sources, _, origin) = consultation_reads::fixture();
    let before = f.count("operation_effects");
    let read = f
        .store
        .try_native_consultation_read(
            &origin,
            &sources[1],
            &[sources[0].clone(), sources[2].clone()],
        )
        .unwrap();
    let (p, r, c) = (
        sources[0].runtime_connection_id(),
        sources[1].runtime_connection_id(),
        sources[2].runtime_connection_id(),
    );
    let plan = json!({"operation_id":origin.record_id(),"audience_revision":0,"private_connections":[p,c],"receivers":{p:[c],c:[p],r:[]},"activate_muted":c,"expected_receiver_revisions":{p:1,r:1,c:1}});
    assert_eq!(
        f.store
            .prepare_native_consultation(read, plan)
            .unwrap_err()
            .code,
        "not-ready"
    );
    assert_eq!(f.count("operation_effects"), before);
    assert_eq!(f.count("native_consultation_attempts"), 0);
}

#[test]
fn private_provider_changed_session_revision_settles_stopped_preparation_without_claiming_provider_cleanup(
) {
    for started in [false, true] {
        let (f, sources, candidate, origin, endpoint) = fresh_fixture();
        let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
        if started {
            let read = f.store.try_private_provider_read(&context).unwrap();
            let (attempt, read) = f
                .store
                .begin_private_provider_context(context.clone(), read)
                .unwrap();
            drop(read);
            drop(attempt);
        }
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE sessions SET topology_revision=topology_revision+1 WHERE id='session'",
                [],
            )
            .unwrap();
        let status = f
            .store
            .execute_operation_action(
                &f.keys[0],
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            status.state,
            if started {
                connect_protocol::transitions::OperationState::Unknown
            } else {
                connect_protocol::transitions::OperationState::Failed
            }
        );
        assert!(f.store.try_private_provider_read(&context).is_err());
        assert_eq!(f.count("native_consultation_attempts"), 0);
    }
}

fn ready_named(f: &Fixture, actor: usize, name: &str) -> Arc<NativeMediaOrigin> {
    let request = if actor < 3 {
        crate::PeerAuthorityRequest {
            credentials: vec![f.keys[actor].clone()],
            device: crate::PeerDeviceClaim {
                id: format!("device-{actor}"),
                kind: "desktop".into(),
                platform: "isolated-store-fixture".into(),
            },
            key_id: format!("native-key-{actor}"),
            algorithm: "EdDSA".into(),
            command: crate::PeerCommand::NativeConnectionOffer(f.request(actor, name).intent),
        }
    } else {
        f.native_request_for(actor, 600, name)
    };
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[actor], &proof, &lease)
        .unwrap();
    let mut c = f.store.connection().unwrap();
    let tx = c.transaction().unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [lease.effect_id()],
    )
    .unwrap();
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed' WHERE record_id=?1",
        [lease.record_id()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
    tx.commit().unwrap();
    origin
}
#[test]
fn a_fresh_private_context_recalls_authorized_private_history_without_exporting_it_to_public_memory(
) {
    let (f, sources, candidate, origin, _, endpoint, native_receipt) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let receipt = delivered(&f, &context);
    f.store
        .record_native_consultation_decoded(&native_receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&native_receipt)
        .unwrap()
        .unwrap();
    let transcript = f
        .store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "old-private-history",
            "user",
            "final",
            "earlier authorized private decision",
        )
        .unwrap()
        .unwrap();
    let primary = ready_named(&f, 0, "another-primary");
    let remote = ready_named(&f, 3, "another-remote");
    let command = connect_protocol::transitions::TransitionCommand {
        operation_id: "another-private-context".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(primary.connection_id().into()),
        expected_revision: 4,
        intent: connect_protocol::transitions::TransitionIntent::Consult {
            retained_connid: remote.connection_id().into(),
            private_participants: vec![f.pids[0].clone(), f.pids[2].clone()],
            hold_treatment: connect_protocol::transitions::HoldTreatment::Silence,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let next = f
        .store
        .native_consultation_origin(&f.keys[0], command)
        .unwrap();
    let next_candidate =
        consultation_audiences::candidate_named(&f, &next, true, "another-private-candidate");
    let outside = [
        sources[0].clone(),
        sources[1].clone(),
        sources[2].clone(),
        candidate.clone(),
    ];
    let read = f
        .store
        .try_native_consultation_scene_read(
            &next,
            &remote,
            &[primary.clone(), next_candidate.clone()],
            &outside,
        )
        .unwrap();
    assert!(read
        .media(candidate.connection_id(), Some(sources[0].connection_id()))
        .is_some());
    assert!(read
        .media(
            next_candidate.connection_id(),
            Some(sources[0].connection_id())
        )
        .is_none());
    assert!(read
        .media(sources[2].connection_id(), Some(sources[0].connection_id()))
        .is_none());
    let next_context = f
        .store
        .prepare_private_provider_context(read, &f.keys[0], &endpoint)
        .unwrap();
    let _creation = created(&f, &next_context, "another-actual-fresh-private-call");
    let read = f.store.try_private_provider_read(&next_context).unwrap();
    assert!(read
        .json()
        .unwrap()
        .contains("earlier authorized private decision"));
    drop(read);
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    let ai = f.store.revalidate(&f.keys[2]).unwrap();
    let c = f.store.connection().unwrap();
    let public = crate::context::view_for_scope(
        &f.store,
        &c,
        &owner,
        &f.cid,
        1000,
        Some(&ai.identity_id),
        Some(&ai),
    )
    .unwrap();
    assert!(!public.items.iter().any(|i| i.source_id == transcript.id));
    assert!(crate::context::view_for_private_scope_at(
        &f.store,
        &c,
        &owner,
        &f.cid,
        1000,
        Some(&ai.identity_id),
        Some(&ai),
        None,
        Some("unregistered-private-context"),
    )
    .is_err());
}

#[test]
fn private_provider_opaque_memory_and_new_transcripts_require_current_history_recipient_policy() {
    let (f, sources, candidate, origin, _, endpoint, native_receipt) = fixture();
    f.store
        .execute(
            &f.keys[1],
            &Operation::Send {
                request_id: "retained-history-before-private".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "history later outside the context grant",
                ),
            },
        )
        .unwrap();
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    let context_key = f
        .store
        .issue(&owner.identity_id, owner.scope, crate::now() + 600)
        .unwrap()
        .id;
    let context = f
        .store
        .prepare_private_provider_context(
            read(&f, &sources, &candidate, &origin),
            &context_key,
            &endpoint,
        )
        .unwrap();
    let receipt = delivered(&f, &context);
    f.store
        .record_native_consultation_decoded(&native_receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&native_receipt)
        .unwrap();
    assert!(f
        .store
        .try_native_media_read(&candidate, Some(&sources[0]))
        .is_ok());
    let held = f
        .store
        .try_private_provider_media_read(&context, &receipt)
        .unwrap();
    assert!(held.current());
    drop(held);
    let remote = f.store.revalidate(&f.keys[1]).unwrap().identity_id;
    let primary = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let ai = f.store.revalidate(&f.keys[2]).unwrap().identity_id;
    assert_ne!(remote, primary);
    assert_ne!(remote, ai);
    // Controlled current-scope change: both physical private recipients remain
    // permitted, while the owner of an already-exported history source does not.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients=?2 WHERE id=?1",
            params![
                context_key,
                serde_json::to_string(&vec![primary, ai]).unwrap()
            ],
        )
        .unwrap();
    assert!(f.store.try_native_media_read(&sources[0], None).is_ok());
    assert!(f.store.try_native_media_read(&candidate, None).is_err());
    assert!(f
        .store
        .try_native_media_read(&sources[0], Some(&candidate))
        .is_err());
    assert!(f
        .store
        .try_private_provider_media_read(&context, &receipt)
        .is_err());
    assert!(f
        .store
        .record_private_provider_transcript(
            &context,
            &receipt,
            "after-history-policy",
            "assistant",
            "final",
            "derived from stale private memory"
        )
        .is_err());
}

#[test]
fn private_provider_native_policy_memo_tracks_foreign_sqlite_writes_and_clock_expiry() {
    let (f, sources, candidate, origin, _, endpoint, native_receipt) = fixture();
    f.store
        .execute(
            &f.keys[1],
            &Operation::Send {
                request_id: "foreign-history-source".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "current foreign-writer history policy",
                ),
            },
        )
        .unwrap();
    let owner = f.store.revalidate(&f.keys[0]).unwrap();
    let ai = f.store.revalidate(&f.keys[2]).unwrap().identity_id;
    let key = f
        .store
        .issue(&owner.identity_id, owner.scope, crate::now() + 600)
        .unwrap()
        .id;
    let context = f
        .store
        .prepare_private_provider_context(read(&f, &sources, &candidate, &origin), &key, &endpoint)
        .unwrap();
    let _receipt = delivered(&f, &context);
    f.store
        .record_native_consultation_decoded(&native_receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&native_receipt)
        .unwrap();
    assert!(f.store.try_native_media_read(&candidate, None).is_ok());
    {
        let c = f.store.connection().unwrap();
        let (selection,audience):(String,String)=c.query_row("SELECT selection_json,audience_json FROM native_private_lifetimes WHERE operation_record_id=?1",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let mut different: Value = serde_json::from_str(&selection).unwrap();
        different["private"][0]["generation"] = json!(9999);
        assert!(crate::private_provider::ready_until(
            &c,
            &f.store,
            origin.record_id(),
            &different.to_string(),
            &audience
        )
        .unwrap()
        .is_none());
        assert!(crate::private_provider::ready_until(
            &c,
            &f.store,
            origin.record_id(),
            &selection,
            &audience
        )
        .unwrap()
        .is_some());
    }
    let version_before: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    let other = rusqlite::Connection::open(&f.store.path).unwrap();
    other
        .execute(
            "UPDATE credentials SET recipients=?2 WHERE id=?1",
            params![
                key,
                serde_json::to_string(&vec![owner.identity_id, ai]).unwrap()
            ],
        )
        .unwrap();
    let version_after: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    assert_ne!(version_before, version_after);
    assert!(f.store.try_native_media_read(&candidate, None).is_err());
    assert!(f.store.try_native_media_read(&sources[0], None).is_ok());
    let expires = crate::now() + 2;
    other
        .execute(
            "UPDATE credentials SET recipients=?3,expires_at=?2 WHERE id=?1",
            params![key, expires, serde_json::to_string(&vec!["*"]).unwrap()],
        )
        .unwrap();
    let current = f.store.try_native_media_read(&candidate, None).unwrap();
    assert!(current.current());
    assert!(current.expires_at() <= expires);
    drop(current);
    while crate::now() < expires {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(f.store.try_native_media_read(&candidate, None).is_err());
    assert!(f.store.try_native_media_read(&sources[0], None).is_ok());
    assert!(!f.store.connection().unwrap().query_row("SELECT closed_at IS NOT NULL FROM private_provider_contexts WHERE operation_record_id=?1",[origin.record_id()],|r|r.get::<_,bool>(0)).unwrap(),"Policy refusal is not provider termination evidence");
}

#[test]
fn ai_owned_private_context_requires_every_private_listener_for_all_sources() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    use connect_protocol::messages::*;
    for (operation, to, body) in [
        (
            "pair-source",
            vec![f.pids[0].clone(), f.pids[2].clone()],
            "PAIR-CONTEXT-PERMITTED",
        ),
        (
            "ai-only-source",
            vec![f.pids[2].clone()],
            "AI-ONLY-NOT-FOR-PRIMARY",
        ),
    ] {
        f.store
            .execute(
                &f.keys[2],
                &Operation::Send {
                    request_id: operation.into(),
                    conversation_id: f.cid.clone(),
                    message: MessageDraft::text(MessageAudience::Participants(to), body),
                },
            )
            .unwrap();
    }
    f.store
        .execute(
            &f.keys[2],
            &Operation::Send {
                request_id: "revocable-private-listener-source".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text(
                    MessageAudience::Participants(vec![f.pids[2].clone()]),
                    "REVOKABLE-SHARED-CONTEXT",
                ),
            },
        )
        .unwrap();
    let message: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM messages WHERE conversation_id=?1 AND body='REVOKABLE-SHARED-CONTEXT'",
            [&f.cid],
            |r| r.get(0),
        )
        .unwrap();
    use connect_protocol::membership::*;
    let MembershipOutcome::Sharing { sharing_id, .. } = f
        .store
        .execute_membership(
            &f.keys[2],
            MembershipRequest {
                operation_id: "allow-private-primary-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[2], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::ShareHistory {
                    resources: vec![ContentRef {
                        kind: ContentKind::Message,
                        id: message.clone(),
                    }],
                    participants: vec![f.pids[0].clone()],
                    provenance: "explicit controlled private-listener grant".into(),
                },
            },
        )
        .unwrap()
    else {
        panic!("shared source")
    };
    let context = f
        .store
        .prepare_private_provider_context(
            read(&f, &sources, &candidate, &origin),
            &f.keys[2],
            &endpoint,
        )
        .unwrap();
    let json: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT selection_json FROM context_deliveries WHERE id=?1",
            [context.delivery_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(json.contains("PAIR-CONTEXT-PERMITTED"));
    assert!(json.contains("REVOKABLE-SHARED-CONTEXT"));
    assert!(
        !json.contains("AI-ONLY-NOT-FOR-PRIMARY"),
        "the AI's own credential is not the human listener's permission"
    );
    let delivered = delivered(&f, &context);
    assert!(f
        .store
        .try_private_provider_media_read(&context, &delivered)
        .is_ok());
    f.store
        .execute_membership(
            &f.keys[2],
            MembershipRequest {
                operation_id: "revoke-private-primary-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[2], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::RevokeHistoryShare { sharing_id },
            },
        )
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        let ai = f.store.native_media_route_actor(&c, &candidate).unwrap();
        assert!(crate::audience::visible_to_identity(
            &c,
            &ai.identity_id,
            &f.cid,
            ContentKind::Message,
            &message
        )
        .unwrap());
    }
    assert!(
        f.store
            .try_private_provider_media_read(&context, &delivered)
            .is_err(),
        "opaque AI memory must stop when the human listener loses a source"
    );
}

#[test]
fn private_context_receipt_keeps_original_native_generation_and_private_audience() {
    use connect_protocol::{ContextDeliveryPurpose, Operation, Outcome};
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let applied = delivered(&f, &context);
    f.store.record_private_provider_closed(&applied).unwrap();
    let original_generation = candidate.generation();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET generation=generation+1,state='retired' WHERE id=?1",
            [candidate.connection_id()],
        )
        .unwrap();
    let page = |key: &str| {
        let Outcome::ContextDeliveries(page) = f
            .store
            .execute(
                key,
                &Operation::ContextDeliveries {
                    conversation_id: f.cid.clone(),
                    before: None,
                    limit: 100,
                },
            )
            .unwrap()
        else {
            panic!("receipts")
        };
        page
    };
    let own = page(&f.keys[0]);
    let receipt = own
        .deliveries
        .iter()
        .find(|d| d.id == context.delivery_id())
        .unwrap();
    let destination = receipt.destination.as_ref().unwrap();
    assert_eq!(destination.connection_id, candidate.connection_id());
    assert_eq!(destination.generation, original_generation);
    assert_eq!(
        destination.purpose,
        ContextDeliveryPurpose::PrivateConsultation
    );
    let lifecycle = receipt.lifecycle.as_ref().unwrap();
    assert!(lifecycle.creation_confirmed && lifecycle.delivery_confirmed && lifecycle.closed);
    assert_eq!(lifecycle.cleanup_admitted, None);
    assert_eq!(lifecycle.cleanup_uncertain, None);
    assert!(
        !page(&f.keys[1])
            .deliveries
            .iter()
            .any(|d| d.id == context.delivery_id()),
        "retained remote received private context receipt"
    );
    assert!(page(&f.keys[2])
        .deliveries
        .iter()
        .any(|d| d.id == context.delivery_id()));
}

mod observation_sources;

#[test]
fn private_provider_metadata_wait_rechecks_current_authority_after_sql_contention() {
    for revoked in [false, true] {
        let (f, sources, candidate, origin, _, endpoint, _) = fixture();
        let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
        let c = f.store.connection().unwrap();
        std::thread::scope(|threads| {
            let (started, waiting) = std::sync::mpsc::channel();
            let store = &f.store;
            let context = &context;
            let pending = threads.spawn(move || {
                started.send(()).unwrap();
                store.private_provider_read(
                    context,
                    &crate::AuthorityWait::new(std::time::Duration::from_secs(2)),
                )
            });
            waiting.recv().unwrap();
            assert_eq!(
                f.store.try_private_provider_read(context).unwrap_err().code,
                "authority-busy"
            );
            if revoked {
                c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
                    .unwrap();
            }
            drop(c);
            let result = pending.join().unwrap();
            if revoked {
                assert!(matches!(
                    result.unwrap_err().code.as_str(),
                    "unauthorized" | "forbidden"
                ));
            } else {
                let read = result.unwrap();
                assert!(read.current());
                assert_eq!(read.assistant_id().unwrap(), "fixture-assistant");
                assert_eq!(read.json().unwrap_err().code, "not-ready");
            }
        });
    }
}

#[test]
fn private_provider_metadata_cancelled_wait_cannot_start_or_export() {
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(2));
    wait.cancel();
    assert_eq!(
        f.store
            .private_provider_read(&context, &wait)
            .unwrap_err()
            .code,
        "cancelled"
    );
    let c = f.store.connection().unwrap();
    let starts: (bool, bool) = c
        .query_row(
            "SELECT started,export_started FROM private_provider_contexts WHERE delivery_id=?1",
            [context.delivery_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(starts, (false, false));
}
