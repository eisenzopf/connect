//! Controlled Store facts only; these tests perform no Core IO or callback.
use super::*;
use connect_protocol::transitions::{OperationAction, TransitionCommand, TransitionIntent};

fn key(f: &Fixture) -> String {
    let owner = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "connection.replace".into(),
        "operation.cancel".into(),
        "operation.execute".into(),
    ]);
    f.store.issue(&owner, scope, crate::now() + 600).unwrap().id
}
fn endpoint(f: &Fixture, capabilities: Vec<String>, name: &str) -> String {
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("provision-{name}"),
                identity_id: identity,
                device: DeviceSpec {
                    id: format!("device-{name}"),
                    kind: "desktop".into(),
                    platform: "isolated-store-fixture".into(),
                },
                transport: EndpointTransport::Quic,
                address: format!(
                    "quic://localhost:{}",
                    if name == "replacement" { 4555 } else { 4556 }
                ),
                capabilities,
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store.connection().unwrap().execute("UPDATE endpoints SET verification_state='verified',verification_evidence='controlled-fixture',verified_at=?2 WHERE id=?1",params![endpoint_id,crate::now()]).unwrap();
    endpoint_id
}
fn fixture_with_sources() -> (
    Fixture,
    String,
    String,
    TransitionCommand,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
) {
    let f = Fixture::new();
    let old = consultation_reads::ready(&f, 0);
    let remote = consultation_reads::ready(&f, 1);
    let target = endpoint(&f, vec!["audio".into()], "replacement");
    let key = key(&f);
    let command = TransitionCommand {
        operation_id: "move-native-endpoint".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(old.connection_id().into()),
        expected_revision: 3,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(target.clone()),
            candidate_connid: None,
            retained_connid: remote.connection_id().into(),
        },
        payload_extensions: Default::default(),
    };
    (f, key, target, command, vec![old, remote])
}
fn fixture() -> (Fixture, String, String, TransitionCommand) {
    let (f, key, target, command, _) = fixture_with_sources();
    (f, key, target, command)
}
#[test]
fn replacement_source_pins_original_command_expiry_and_only_the_provisioned_destination_without_effects(
) {
    let (f, key, target, mut command) = fixture();
    command
        .payload_extensions
        .insert("destination".into(), json!("untrusted-caller-address"));
    let accepted = f
        .store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
        f.count("connections"),
    );
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let context = f.store.native_replacement_context(&origin).unwrap();
    assert_eq!(origin.record_id(), accepted.record_id);
    assert_eq!(Some(origin.request().expires_at), accepted.expires_at);
    assert_eq!(context.target_endpoint_id, target);
    assert_eq!(context.target_address, "quic://localhost:4555");
    assert_eq!(context.participant_id, f.pids[0]);
    assert_eq!(context.membership_epoch, 1);
    assert_eq!(context.target_transport, EndpointTransport::Quic);
    assert_ne!(
        context.source_runtime_connection_id,
        context.retained_runtime_connection_id
    );
    assert_eq!(format!("{origin:?}"), "NativeReplacementOrigin(redacted)");
    assert_eq!(format!("{context:?}"), "NativeReplacementContext(redacted)");
    let mut changed = command.clone();
    changed
        .payload_extensions
        .insert("destination".into(), json!("changed"));
    assert!(f.store.native_replacement_origin(&key, changed).is_err());
    assert_eq!(
        f.store.native_replacement_context(&origin).unwrap(),
        context
    );
    assert_eq!(
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal"),
            f.count("connections")
        ),
        before
    );
}
#[test]
fn replacement_source_requires_current_named_owner_permission_and_new_source_after_revocation() {
    let (f, credential, _, command) = fixture();
    f.store
        .admit_transition_command(&credential, command.clone())
        .unwrap();
    let original = f
        .store
        .native_replacement_origin(&credential, command.clone())
        .unwrap();
    for denied in [&f.keys[0], &f.keys[1]] {
        assert!(f
            .store
            .native_replacement_origin(denied, command.clone())
            .is_err());
    }
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &credential).unwrap();
    assert!(f.store.native_replacement_context(&original).is_err());
    let fresh = key(&f);
    let current = f.store.native_replacement_origin(&fresh, command).unwrap();
    assert_eq!(current.record_id(), original.record_id());
    assert_eq!(current.request().expires_at, original.request().expires_at);
    assert!(f.store.native_replacement_context(&original).is_err());
    assert!(f.store.native_replacement_context(&current).is_ok());
}
#[test]
fn replacement_source_rechecks_endpoint_device_membership_generation_topology_cancel_and_worker() {
    for change in [
        "endpoint-revision",
        "device-revoked",
        "source-generation",
        "remote-retired",
        "membership",
        "topology",
        "cancel",
        "worker",
    ] {
        let (f, key, target, command) = fixture();
        f.store
            .admit_transition_command(&key, command.clone())
            .unwrap();
        let origin = f
            .store
            .native_replacement_origin(&key, command.clone())
            .unwrap();
        match change {
            "endpoint-revision" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                        [&target],
                    )
                    .unwrap();
            }
            "device-revoked" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE devices SET revoked=1 WHERE id='device-replacement'",
                        [],
                    )
                    .unwrap();
            }
            "source-generation" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [command.connection_id.as_ref().unwrap()],
                    )
                    .unwrap();
            }
            "remote-retired" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
                        [command.intent.retained()],
                    )
                    .unwrap();
            }
            "membership" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE participants SET active=0 WHERE participant_id=?1",
                        [&f.pids[0]],
                    )
                    .unwrap();
            }
            "topology" => {
                f.store.connection().unwrap().execute("UPDATE sessions SET topology_revision=topology_revision+1 WHERE id='session'",[]).unwrap();
            }
            "cancel" => {
                f.store
                    .execute_operation_action(
                        &key,
                        OperationAction::Cancel {
                            operation_id: "cancel-native-move".into(),
                            target_operation_id: command.operation_id.clone(),
                            reason: None,
                        },
                    )
                    .unwrap();
            }
            "worker" => {
                assert!(f
                    .store
                    .claim_transition(&key, &command.operation_id, 30)
                    .unwrap()
                    .is_some());
            }
            _ => unreachable!(),
        }
        assert!(
            f.store.native_replacement_context(&origin).is_err(),
            "{change}"
        );
        assert!(
            f.store.native_replacement_origin(&key, command).is_err(),
            "{change}"
        );
    }
}
#[test]
fn replacement_source_has_no_idle_reader_and_cannot_cross_installation_or_restart() {
    let (f, key, _, command) = fixture();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_replacement_origin(&key, command).unwrap();
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &f.cid,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(foreign.native_replacement_context(&origin).is_err());
    drop(foreign);
    let path = f.store.path().to_path_buf();
    let Fixture { _dir, store, .. } = f;
    drop(store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert!(successor.native_replacement_context(&origin).is_err());
    drop(successor);
    drop(_dir);
}
#[test]
fn replacement_source_refuses_same_foreign_unverified_and_non_audio_endpoint_selection() {
    for change in ["same", "foreign", "unverified", "non-audio"] {
        let (f, key, target, mut command) = fixture();
        if let TransitionIntent::Replace { endpoint_id, .. } = &mut command.intent {
            match change {
                "same" => *endpoint_id = Some(f.endpoints[0].clone()),
                "foreign" => *endpoint_id = Some(f.endpoints[1].clone()),
                "unverified" => {
                    f.store
                        .connection()
                        .unwrap()
                        .execute(
                            "UPDATE endpoints SET verification_state='unverified' WHERE id=?1",
                            [&target],
                        )
                        .unwrap();
                }
                "non-audio" => *endpoint_id = Some(endpoint(&f, vec!["video".into()], "non-audio")),
                _ => unreachable!(),
            }
        }
        let before = f.count("operation_effects");
        let result = f
            .store
            .admit_transition_command(&key, command.clone())
            .and_then(|_| f.store.native_replacement_origin(&key, command));
        assert!(result.is_err(), "{change}");
        assert_eq!(f.count("operation_effects"), before);
    }
}
#[test]
fn replacement_candidate_metadata_is_pinned_to_its_original_endpoint_and_does_not_prove_media() {
    let (f, key, target, mut command) = fixture();
    let mut request = f.request(0, "replacement-candidate");
    request.endpoint_id = target.clone();
    f.apply(0, request.clone()).unwrap();
    // Controlled local observation for durable admission only. No native
    // candidate allocation, Core reservation, answer or duplex IO exists.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET observed_runtime_id=?2 WHERE id=?1",
            params![request.intent.connection_id, f.store.runtime_epoch()],
        )
        .unwrap();
    if let TransitionIntent::Replace {
        endpoint_id,
        candidate_connid,
        ..
    } = &mut command.intent
    {
        *endpoint_id = None;
        *candidate_connid = Some(request.intent.connection_id.clone());
    }
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_replacement_origin(&key, command).unwrap();
    let context = f.store.native_replacement_context(&origin).unwrap();
    assert_eq!(context.target_endpoint_id, target);
    assert_eq!(
        context.candidate_connection_id,
        Some(request.intent.connection_id.clone())
    );
    let other = endpoint(&f, vec!["audio".into()], "different-target");
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET endpoint_id=?2 WHERE id=?1",
            params![request.intent.connection_id, other],
        )
        .is_err());
    assert_eq!(
        f.store.native_replacement_context(&origin).unwrap(),
        context
    );
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke-candidate-endpoint".into(),
                endpoint_id: target,
                expected_revision: 0,
            },
        )
        .unwrap();
    assert!(f.store.native_replacement_context(&origin).is_err());
}

#[test]
fn replacement_metadata_preserves_an_observer_role_without_promoting_or_changing_defaults() {
    let (f, key, _, command) = fixture();
    let original_default: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT role FROM participants WHERE participant_id=?1",
            [&f.pids[0]],
            |r| r.get(0),
        )
        .unwrap();
    f.store.connection().unwrap().execute("UPDATE session_participants SET role='observer' WHERE session_id='session' AND participant_id=?1",[&f.pids[0]]).unwrap();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_replacement_origin(&key, command).unwrap();
    let context = f.store.native_replacement_context(&origin).unwrap();
    assert_eq!(context.role, "observer");
    let default_role: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT role FROM participants WHERE participant_id=?1",
            [&f.pids[0]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(default_role, original_default);
}

#[test]
fn replacement_preparation_read_holds_only_current_original_physical_authority_and_no_candidate_io()
{
    let (f, key, target, command, sources) = fixture_with_sources();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_replacement_origin(&key, command).unwrap();
    let unrelated = f
        .store
        .create_identity("outside-replacement-conversation")
        .unwrap()
        .id;
    let before = (
        f.count("operation_effects"),
        f.count("journal"),
        f.count("connections"),
    );
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    assert!(read.current());
    assert!(read.expires_at() <= crate::now() + 5);
    assert_eq!(read.context().target_endpoint_id, target);
    assert_eq!(read.caller().actor.participant_id, f.pids[0]);
    assert_eq!(
        read.source()
            .native()
            .unwrap()
            .target()
            .connection
            .connection_id,
        sources[0].connection_id()
    );
    assert_eq!(
        read.retained().target().connection.connection_id,
        sources[1].connection_id()
    );
    assert!(read.source().native().unwrap().source().is_none());
    assert!(read.retained().source().is_none());
    for i in [0, 1] {
        let identity = f.store.revalidate(&f.keys[i]).unwrap().identity_id;
        assert!(f
            .store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &identity,
                crate::AuthorityMode::Write
            )
            .unwrap()])
            .is_err());
    }
    let other = unrelated;
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &other,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    drop(read);
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    drop(
        f.store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                &identity,
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    assert_eq!(
        (
            f.count("operation_effects"),
            f.count("journal"),
            f.count("connections")
        ),
        before
    );
}
#[test]
fn replacement_preparation_refuses_swapped_foreign_and_revoked_physical_origins_despite_valid_control(
) {
    for denied in ["swapped", "foreign", "source-revoked", "remote-revoked"] {
        let (f, key, _, command, sources) = fixture_with_sources();
        f.store
            .admit_transition_command(&key, command.clone())
            .unwrap();
        let origin = f.store.native_replacement_origin(&key, command).unwrap();
        match denied {
            "swapped" => assert!(f
                .store
                .try_native_replacement_preparation_read(
                    &origin,
                    &crate::NativeReplacementSourceOrigin::Native(sources[1].clone()),
                    &sources[0]
                )
                .is_err()),
            "foreign" => {
                let foreign = Fixture::new();
                let peer = consultation_reads::ready(&foreign, 0);
                assert!(f
                    .store
                    .try_native_replacement_preparation_read(
                        &origin,
                        &crate::NativeReplacementSourceOrigin::Native(peer.clone()),
                        &sources[1]
                    )
                    .is_err());
            }
            other => {
                let i = if other == "source-revoked" { 0 } else { 1 };
                let admin = f.store.revalidate(&f.admin).unwrap();
                f.store.revoke(&admin, &f.keys[i]).unwrap();
                // Current controller metadata alone cannot replace either
                // physical peer's separately current signed media permission.
                assert!(f.store.native_replacement_context(&origin).is_ok());
                assert!(f
                    .store
                    .try_native_replacement_preparation_read(
                        &origin,
                        &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
                        &sources[1]
                    )
                    .is_err());
            }
        }
    }
}
#[test]
fn replacement_preparation_read_refuses_a_held_writer_without_waiting_or_reserving_work() {
    let (f, key, _, command, sources) = fixture_with_sources();
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f.store.native_replacement_origin(&key, command).unwrap();
    let identity = f.store.revalidate(&f.keys[1]).unwrap().identity_id;
    let writer = f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::identity(
            &identity,
            crate::AuthorityMode::Write,
        )
        .unwrap()])
        .unwrap();
    assert!(f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1]
        )
        .is_err());
    drop(writer);
    assert!(f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1]
        )
        .is_ok());
}

#[path = "replacement_sources/custody.rs"]
mod custody;

mod allocation;

mod start;

mod commit;

mod telephone;
