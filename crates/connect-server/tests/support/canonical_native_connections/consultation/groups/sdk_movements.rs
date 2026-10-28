//! Actual SDK controller transitions retain independent native media owners.
use super::*;
use connect_client::sdk::{
    self, Client, HandoffInput, HandoffRole, MoveInput, MoveTarget, MutationOptions, Outgoing,
    RequestOptions,
};
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(35)).unwrap()
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn session(client: &Client, command: &TransitionCommand) -> sdk::Session {
    client
        .conversation_by_id(command.conversation_id.parse().unwrap())
        .session_by_id(command.session_id.parse().unwrap())
}
enum Owner<T> {
    Internal {
        client: Client,
        operation: sdk::Operation<T>,
        config: CanonicalClientConfig,
    },
    External(crate::external_sdk_consumer::Consumer),
    Node(super::super::sdk_controls::NodeMovementProbe),
}
pub(super) struct HandoffProbe(Owner<sdk::HandoffResult>);
pub(super) struct ReplacementProbe {
    owner: Owner<sdk::MoveResult>,
    pub baseline: (usize, usize, usize),
}
pub(super) async fn handoff(
    f: &Fixture,
    command: &TransitionCommand,
    token: &str,
    group: &Group,
) -> HandoffProbe {
    let config = super::super::sdk_controls::controller_config(f, token, 911);
    if super::super::sdk_controls::NodeMovementProbe::enabled() {
        let parent = json!({"schema":"connect-operation-reference/1","operation_id":group.command.operation_id,"command":"session.consult","scope":{"conversation_id":group.command.conversation_id,"session_id":group.command.session_id,"connection_id":group.command.connection_id},"continuation":{"kind":"consultation","retained_connection_id":group.command.intent.retained(),"record_id":group.record}});
        let mut probe =
            super::super::sdk_controls::NodeMovementProbe::register(f, config, command, parent)
                .await;
        probe.begin().await;
        return HandoffProbe(Owner::Node(probe));
    }
    if crate::external_sdk_consumer::enabled() {
        let parent = json!({"schema":"connect-operation-reference/1","operation_id":group.command.operation_id,"command":"session.consult","scope":{"conversation_id":group.command.conversation_id,"session_id":group.command.session_id,"connection_id":group.command.connection_id},"continuation":{"kind":"consultation","retained_connection_id":group.command.intent.retained(),"record_id":group.record}});
        let mut consumer=crate::external_sdk_consumer::Consumer::start(f,json!({"mode":"transition","config":crate::external_sdk_consumer::configured(config),"command":command,"consultation":parent})).await;
        consumer.registered().await;
        consumer.begin().await;
        return HandoffProbe(Owner::External(consumer));
    }
    let client = Client::connect(config.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let TransitionIntent::Handoff {
        source_participant,
        target_participant,
        retained_connid,
        candidate_connid,
        outgoing,
        role_scope,
        expected_conversation_revision,
    } = &command.intent
    else {
        panic!("handoff");
    };
    let parent=client.restore(json!({"schema":"connect-operation-reference/1","operation_id":group.command.operation_id,"command":"session.consult","scope":{"conversation_id":group.command.conversation_id,"session_id":group.command.session_id,"connection_id":group.command.connection_id},"continuation":{"kind":"consultation","retained_connection_id":group.command.intent.retained(),"record_id":group.record}}),client.operation_kinds().session_consult).unwrap();
    let input = HandoffInput {
        via: command.connection_id.as_ref().unwrap().parse().unwrap(),
        from_participant: source_participant.parse().unwrap(),
        to: target_participant.parse().unwrap(),
        retain: retained_connid.parse().unwrap(),
        outgoing: if *outgoing == connect_protocol::transitions::OutgoingDisposition::Leave {
            Outgoing::Leave
        } else {
            Outgoing::Observe
        },
        role: if *role_scope == connect_protocol::transitions::RoleScope::Session {
            HandoffRole::Session
        } else {
            HandoffRole::Conversation {
                expected_conversation_revision: sdk::ConversationRevision::try_from(
                    expected_conversation_revision.unwrap(),
                )
                .unwrap(),
            }
        },
        expected_topology_revision: sdk::TopologyRevision::try_from(command.expected_revision)
            .unwrap(),
        candidate: candidate_connid.as_ref().map(|id| id.parse().unwrap()),
        consultation: Some(parent.reference()),
    };
    let mut opts = mutation(&command.operation_id);
    let mut ext = command.payload_extensions.clone();
    assert_eq!(
        ext.remove("consultation_record_id"),
        Some(json!(group.record))
    );
    opts.extensions = Some(ext);
    let operation = session(&client, command)
        .handoff(input, opts)
        .await
        .unwrap();
    HandoffProbe(Owner::Internal {
        client,
        operation,
        config,
    })
}
pub(super) async fn handoff_completed(
    f: &Fixture,
    probe: HandoffProbe,
    command: &TransitionCommand,
    recorded: &Value,
) {
    let (client, operation, config) = match probe.0 {
        Owner::Internal {
            client,
            operation,
            config,
        } => (client, operation, config),
        Owner::Node(probe) => {
            let summary = probe.completed().await;
            compare_topology(&summary, recorded);
            assert_eq!(summary["role_change"], recorded["native_handoff"]);
            return;
        }
        Owner::External(consumer) => {
            let result = consumer.finish().await;
            assert_eq!(result["phase"], "completed");
            assert_eq!(result["recovered"], true);
            let summary = &result["summary"];
            assert_eq!(
                summary["topology"]["topology_revision"],
                recorded["session_revision"]
            );
            assert_eq!(
                summary["topology"]["retained"],
                recorded["retained_connection_ids"]
            );
            assert_eq!(
                summary["topology"]["admitted"],
                recorded["admitted_connection_ids"]
            );
            assert_eq!(
                summary["topology"]["retired"],
                recorded["retired_connection_ids"]
            );
            assert_eq!(summary["role_change"], recorded["native_handoff"]);
            return;
        }
    };
    let value = operation.wait(options()).await.unwrap().value;
    assert_eq!(
        value.topology.topology_revision.get(),
        recorded["session_revision"].as_u64().unwrap()
    );
    assert_eq!(
        value
            .topology
            .retained
            .iter()
            .map(|c| c.id().as_str())
            .collect::<Vec<_>>(),
        recorded["retained_connection_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
    );
    let role = value.role_change.unwrap();
    assert_eq!(
        serde_json::to_value(role.outgoing).unwrap(),
        recorded["native_handoff"]["outgoing"]
    );
    assert_eq!(
        role.participant_revision,
        recorded["native_handoff"]["participant_revision"]
            .as_u64()
            .unwrap()
    );
    assert!(value.topology.admitted[0]
        .native_negotiation()
        .unwrap()
        .is_none());
    let fresh = Client::connect(config, "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let restored = fresh
        .restore(
            operation.reference_json(),
            fresh.operation_kinds().session_handoff,
        )
        .unwrap();
    assert_eq!(
        restored
            .wait(options())
            .await
            .unwrap()
            .value
            .topology
            .topology_revision
            .get(),
        value.topology.topology_revision.get()
    );
    fresh.close(options()).await.unwrap();
    client.close(options()).await.unwrap();
    println!(
        "SDK handoff {}:{} scope={},outgoing={},one_effect,fresh_observation_no_custody",
        f.client.signing_algorithm,
        command.operation_id,
        recorded["native_handoff"]["role_scope"],
        recorded["native_handoff"]["outgoing"]
    );
}
pub(super) async fn replacement(
    f: &Fixture,
    command: &TransitionCommand,
    token: &str,
) -> ReplacementProbe {
    let config = super::super::sdk_controls::controller_config(f, token, 921);
    let original = f.server.service.canonical_runtime_snapshot().unwrap();
    if super::super::sdk_controls::NodeMovementProbe::enabled() {
        let mut probe = super::super::sdk_controls::NodeMovementProbe::register(
            f,
            config,
            command,
            Value::Null,
        )
        .await;
        let before = registered_controller(f, &original).await;
        probe.begin().await;
        return ReplacementProbe {
            owner: Owner::Node(probe),
            baseline: before,
        };
    }
    if crate::external_sdk_consumer::enabled() {
        let mut consumer=crate::external_sdk_consumer::Consumer::start(f,json!({"mode":"transition","config":crate::external_sdk_consumer::configured(config),"command":command,"consultation":null})).await;
        consumer.registered().await;
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        let before = loop {
            let current = f.server.service.canonical_runtime_snapshot().unwrap();
            if current.physical_peers == original.physical_peers + 1 {
                assert_eq!(current.connections, original.connections);
                assert_eq!(current.live_sessions, original.live_sessions);
                break current;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "external controller registration did not settle"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        consumer.begin().await;
        return ReplacementProbe {
            owner: Owner::External(consumer),
            baseline: (
                before.connections,
                before.physical_peers,
                before.live_sessions,
            ),
        };
    }
    let client = Client::connect(config.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    // The runtime binds a control peer on its first authorized command, not
    // authentication. Use a read to establish it before the allocation baseline.
    client
        .conversation_by_id(command.conversation_id.parse().unwrap())
        .refresh(options())
        .await
        .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    let before = loop {
        let current = f.server.service.canonical_runtime_snapshot().unwrap();
        if current.physical_peers == original.physical_peers + 1 {
            assert_eq!(current.connections, original.connections);
            assert_eq!(current.live_sessions, original.live_sessions);
            break current;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "SDK controller registration did not settle"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let TransitionIntent::Replace {
        retained_connid,
        endpoint_id,
        candidate_connid,
    } = &command.intent
    else {
        panic!("replacement");
    };
    let target = if let Some(id) = endpoint_id {
        MoveTarget::Endpoint(id.parse().unwrap())
    } else {
        MoveTarget::Candidate(candidate_connid.as_ref().unwrap().parse().unwrap())
    };
    let mut opts = mutation(&command.operation_id);
    opts.extensions = Some(command.payload_extensions.clone());
    let operation = session(&client, command)
        .connection_by_id(command.connection_id.as_ref().unwrap().parse().unwrap())
        .replace_device(
            MoveInput {
                retain: retained_connid.parse().unwrap(),
                target,
                expected_topology_revision: sdk::TopologyRevision::try_from(
                    command.expected_revision,
                )
                .unwrap(),
            },
            opts,
        )
        .await
        .unwrap();
    assert_ne!(
        operation.status(options()).await.unwrap().state,
        sdk::OperationState::Completed,
        "preparation is not committed movement"
    );
    ReplacementProbe {
        owner: Owner::Internal {
            client,
            operation,
            config,
        },
        baseline: (
            before.connections,
            before.physical_peers,
            before.live_sessions,
        ),
    }
}
pub(super) async fn replacement_completed(
    f: &Fixture,
    probe: ReplacementProbe,
    command: &TransitionCommand,
    recorded: &Value,
) {
    let (client, operation, config) = match probe.owner {
        Owner::Internal {
            client,
            operation,
            config,
        } => (client, operation, config),
        Owner::Node(probe) => {
            let summary = probe.completed().await;
            compare_topology(&summary, recorded);
            assert_eq!(summary["connection_id"], recorded["connection_id"]);
            assert_eq!(summary["native_owned"], false);
            for field in [
                "generation",
                "participant_id",
                "membership_epoch",
                "endpoint_id",
                "device_id",
            ] {
                assert_eq!(
                    summary["binding"][field], recorded[field],
                    "captured move binding: {field}"
                );
            }
            return;
        }
        Owner::External(consumer) => {
            let result = consumer.finish().await;
            assert_eq!(result["phase"], "completed");
            assert_eq!(result["recovered"], true);
            let summary = &result["summary"];
            assert_eq!(summary["connection_id"], recorded["connection_id"]);
            assert_eq!(summary["generation"], 2);
            assert_eq!(summary["native_owned"], false);
            assert_eq!(
                summary["topology"]["topology_revision"],
                recorded["session_revision"]
            );
            assert_eq!(
                summary["topology"]["retained"],
                recorded["retained_connection_ids"]
            );
            assert_eq!(
                summary["topology"]["retired"],
                recorded["retired_connection_ids"]
            );
            return;
        }
    };
    let value = operation.wait(options()).await.unwrap().value;
    assert_eq!(
        value.connection.id().as_str(),
        recorded["connection_id"].as_str().unwrap()
    );
    assert_eq!(value.binding.generation, 2);
    assert_eq!(
        value.topology.topology_revision.get(),
        recorded["session_revision"].as_u64().unwrap()
    );
    assert_eq!(
        value.topology.retained[0].id().as_str(),
        command.intent.retained()
    );
    assert_eq!(
        value.topology.retired[0].as_str(),
        command.connection_id.as_ref().unwrap()
    );
    assert!(value.connection.native_negotiation().unwrap().is_none());
    let fresh = Client::connect(config, "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let restored = fresh
        .restore(
            operation.reference_json(),
            fresh.operation_kinds().connection_replace,
        )
        .unwrap();
    let replay = restored.wait(options()).await.unwrap().value;
    assert_eq!(replay.connection.id(), value.connection.id());
    assert_eq!(replay.binding.generation, value.binding.generation);
    assert!(replay.connection.native_negotiation().unwrap().is_none());
    fresh.close(options()).await.unwrap();
    client.close(options()).await.unwrap();
    println!("SDK replacement {}:{} generation={},retained={},retired={},captured_topology,fresh_observation_no_custody",f.client.signing_algorithm,command.operation_id,value.binding.generation,command.intent.retained(),command.connection_id.as_ref().unwrap());
}
pub(super) async fn replacement_stopped(
    f: &Fixture,
    probe: ReplacementProbe,
    state: OperationState,
    token: &str,
    original_revoked: bool,
) {
    let (client, operation) = match probe.owner {
        Owner::Internal {
            client, operation, ..
        } => (client, operation),
        Owner::Node(probe) => {
            let reference = probe.reference.clone();
            let result = probe.stopped(state, original_revoked).await;
            assert_eq!(result["state"], serde_json::to_value(state).unwrap());
            let config = super::super::sdk_controls::controller_config(f, token, 922);
            super::super::sdk_controls::NodeMovementProbe::recover_stopped(
                f, config, reference, state,
            )
            .await;
            return;
        }
        Owner::External(consumer) => {
            let reference = consumer.reference.clone().unwrap();
            let command = consumer.command.clone();
            let result = consumer.finish().await;
            if original_revoked {
                assert_eq!(result["phase"], "error");
                assert_eq!(result["certainty"], "ReadOnly");
            } else {
                assert_eq!(result["phase"], "stopped");
                assert_eq!(result["state"], serde_json::to_value(state).unwrap());
                assert_eq!(result["certainty"], "Recorded");
            }
            let config = super::super::sdk_controls::controller_config(f, token, 922);
            let recovery=crate::external_sdk_consumer::Consumer::start(f,json!({"mode":"recover","config":crate::external_sdk_consumer::configured(config),"command":command,"reference":reference})).await;
            let recovered = recovery.finish().await;
            assert_eq!(recovered["phase"], "stopped");
            assert_eq!(recovered["state"], serde_json::to_value(state).unwrap());
            assert_eq!(recovered["certainty"], "Recorded");
            return;
        }
    };
    if original_revoked {
        let error = operation
            .status(options())
            .await
            .err()
            .expect("revoked controller cannot recover");
        assert_eq!(error.certainty, sdk::Certainty::ReadOnly);
        assert!(matches!(
            error.kind,
            sdk::ErrorKind::Cancelled
                | sdk::ErrorKind::Authentication
                | sdk::ErrorKind::TransportUncertain
                | sdk::ErrorKind::Application
        ));
    } else {
        assert_eq!(operation.status(options()).await.unwrap().state, state);
        assert!(
            matches!(operation.wait(options()).await,Err(e) if e.certainty==sdk::Certainty::Recorded)
        );
    }
    let config = super::super::sdk_controls::controller_config(f, token, 922);
    let fresh = Client::connect(config, "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let restored = fresh
        .restore(
            operation.reference_json(),
            fresh.operation_kinds().connection_replace,
        )
        .unwrap();
    assert_eq!(restored.status(options()).await.unwrap().state, state);
    let error = restored
        .wait(options())
        .await
        .err()
        .expect("terminal unsuccessful movement remains recorded");
    assert_eq!(error.certainty, sdk::Certainty::Recorded);
    assert_eq!(error.observation.unwrap().state, state);
    fresh.close(options()).await.unwrap();
    client.close(options()).await.unwrap();
    println!("SDK stopped replacement {}:state={state:?},fresh_authority_original_id,original_revoked={original_revoked}",f.client.signing_algorithm);
}
#[tokio::test]
async fn production_public_rust_sdk_handoff_scopes_and_dispositions_keep_actual_remote_audio() {
    for algorithm in ["EdDSA", "ES256"] {
        for (outgoing, scope) in [
            (
                connect_protocol::transitions::OutgoingDisposition::Observe,
                connect_protocol::transitions::RoleScope::Session,
            ),
            (
                connect_protocol::transitions::OutgoingDisposition::Leave,
                connect_protocol::transitions::RoleScope::Session,
            ),
            (
                connect_protocol::transitions::OutgoingDisposition::Observe,
                connect_protocol::transitions::RoleScope::Conversation,
            ),
            (
                connect_protocol::transitions::OutgoingDisposition::Leave,
                connect_protocol::transitions::RoleScope::Conversation,
            ),
        ] {
            super::handoff::handoff_case_with_sdk(outgoing, scope, false, Some(algorithm)).await;
        }
    }
}
#[tokio::test]
async fn production_public_rust_sdk_device_movement_retains_generation_two_audio_and_recovers() {
    for algorithm in ["EdDSA", "ES256"] {
        super::replacement::exercise_replacement_with_sdk(
            true,
            true,
            false,
            true,
            false,
            Some(algorithm),
        )
        .await;
    }
}
#[tokio::test]
async fn production_public_rust_sdk_device_move_preparation_cancellation_never_invents_completion()
{
    for algorithm in ["EdDSA", "ES256"] {
        super::replacement::exercise_replacement_with_sdk(
            false,
            false,
            false,
            false,
            false,
            Some(algorithm),
        )
        .await;
    }
}
#[tokio::test]
async fn production_public_rust_sdk_device_move_recording_failure_preserves_one_known_commit() {
    for algorithm in ["EdDSA", "ES256"] {
        super::replacement::exercise_replacement_with_sdk(
            true,
            true,
            false,
            true,
            true,
            Some(algorithm),
        )
        .await;
    }
}

#[tokio::test]
#[ignore = "requires independent public consumer via CONNECT_GATE6_EXTERNAL_SDK"]
async fn external_public_crate_handoff_preserves_roles_audio_and_original_recovery() {
    crate::external_sdk_consumer::require();
    for algorithm in ["EdDSA", "ES256"] {
        super::handoff::handoff_case_with_sdk(
            connect_protocol::transitions::OutgoingDisposition::Observe,
            connect_protocol::transitions::RoleScope::Session,
            false,
            Some(algorithm),
        )
        .await;
    }
}
#[tokio::test]
#[ignore = "requires independent public consumer via CONNECT_GATE6_EXTERNAL_SDK"]
async fn external_public_crate_device_move_retains_actual_generation_two_audio_and_recovers() {
    crate::external_sdk_consumer::require();
    for algorithm in ["EdDSA", "ES256"] {
        super::replacement::exercise_replacement_with_sdk(
            true,
            true,
            false,
            true,
            false,
            Some(algorithm),
        )
        .await;
    }
}

fn compare_topology(summary: &Value, recorded: &Value) {
    assert_eq!(
        summary["topology"]["topology_revision"],
        recorded["session_revision"]
    );
    assert_eq!(
        summary["topology"]["conversation_revision"],
        recorded["conversation_revision"]
    );
    for (field, host) in [
        ("retained", "retained_connection_ids"),
        ("admitted", "admitted_connection_ids"),
        ("retired", "retired_connection_ids"),
    ] {
        assert_eq!(
            summary["topology"][field], recorded[host],
            "captured move topology: {field}"
        );
    }
}

async fn registered_controller(
    f: &Fixture,
    original: &connect_service::CanonicalRuntimeSnapshot,
) -> (usize, usize, usize) {
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let current = f.server.service.canonical_runtime_snapshot().unwrap();
        if current.physical_peers == original.physical_peers + 1 {
            assert_eq!(current.connections, original.connections);
            assert_eq!(current.live_sessions, original.live_sessions);
            return (
                current.connections,
                current.physical_peers,
                current.live_sessions,
            );
        }
        assert!(
            tokio::time::Instant::now() < until,
            "Node controller registration did not settle"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
pub(super) async fn before_shutdown(f: &Fixture) -> bool {
    let node = super::super::sdk_controls::NodeMovementProbe::enabled();
    if node {
        super::super::sdk_controls::node_drained(f).await;
    }
    node
}
pub(super) fn shutdown_joined(node: bool) {
    if node {
        super::super::sdk_controls::NodeMovementProbe::server_joined();
    }
}
fn node_witness(scenario: &str, algorithms: &[&str], cases: usize) {
    let (helpers, servers) = super::super::sdk_controls::NodeMovementProbe::joined();
    assert_eq!(helpers, cases * 2);
    assert_eq!(servers, cases);
    eprintln!(
        "installed-node-movement-witness {}",
        json!({"profile":"node-movements/1","scenario":scenario,
        "algorithms":algorithms,"helpers_joined":helpers,"servers_joined":servers,"cases":cases,
        "physical_peers_drained":true,"media_readers_drained":true})
    );
}
async fn node_handoff(scope: connect_protocol::transitions::RoleScope) {
    assert!(super::super::sdk_controls::NodeMovementProbe::enabled());
    for algorithm in ["EdDSA", "ES256"] {
        for outgoing in [
            connect_protocol::transitions::OutgoingDisposition::Observe,
            connect_protocol::transitions::OutgoingDisposition::Leave,
        ] {
            super::handoff::handoff_case_with_sdk(outgoing, scope, false, Some(algorithm)).await;
        }
    }
}
#[tokio::test]
#[ignore = "Requires installed Node movement consumer and explicit CONNECT_GATE6_NODE_MOVEMENTS=1"]
async fn installed_node_session_handoff_retains_actual_audio_roles_and_recovery() {
    node_handoff(connect_protocol::transitions::RoleScope::Session).await;
    node_witness("session-handoff", &["EdDSA", "ES256"], 4);
}
#[tokio::test]
#[ignore = "Requires installed Node movement consumer and explicit CONNECT_GATE6_NODE_MOVEMENTS=1"]
async fn installed_node_conversation_handoff_retains_actual_audio_roles_and_recovery() {
    node_handoff(connect_protocol::transitions::RoleScope::Conversation).await;
    node_witness("conversation-handoff", &["EdDSA", "ES256"], 4);
}
async fn node_move(algorithm: &'static str) {
    assert!(super::super::sdk_controls::NodeMovementProbe::enabled());
    super::replacement::exercise_replacement_with_sdk(
        true,
        true,
        false,
        true,
        false,
        Some(algorithm),
    )
    .await;
}
#[tokio::test]
#[ignore = "Requires installed Node movement consumer and explicit CONNECT_GATE6_NODE_MOVEMENTS=1"]
async fn installed_node_eddsa_device_move_preserves_audio_and_current_authority() {
    node_move("EdDSA").await;
    node_witness("device-move-eddsa", &["EdDSA"], 4);
}
#[tokio::test]
#[ignore = "Requires installed Node movement consumer and explicit CONNECT_GATE6_NODE_MOVEMENTS=1"]
async fn installed_node_es256_device_move_preserves_audio_and_current_authority() {
    node_move("ES256").await;
    node_witness("device-move-es256", &["ES256"], 4);
}
#[tokio::test]
#[ignore = "Requires installed Node movement consumer and explicit CONNECT_GATE6_NODE_MOVEMENTS=1"]
async fn installed_node_device_move_cancellation_remains_recorded_after_recovery() {
    assert!(super::super::sdk_controls::NodeMovementProbe::enabled());
    for algorithm in ["EdDSA", "ES256"] {
        super::replacement::exercise_replacement_with_sdk(
            false,
            false,
            false,
            false,
            false,
            Some(algorithm),
        )
        .await;
    }
    node_witness("device-move-cancelled", &["EdDSA", "ES256"], 6);
}
#[tokio::test]
#[ignore = "Requires installed Node movement consumer and explicit CONNECT_GATE6_NODE_MOVEMENTS=1"]
async fn installed_node_device_move_recording_failure_recovers_one_known_commit() {
    assert!(super::super::sdk_controls::NodeMovementProbe::enabled());
    for algorithm in ["EdDSA", "ES256"] {
        super::replacement::exercise_replacement_with_sdk(
            true,
            true,
            false,
            true,
            true,
            Some(algorithm),
        )
        .await;
    }
    node_witness("device-move-recording-failure", &["EdDSA", "ES256"], 2);
}
