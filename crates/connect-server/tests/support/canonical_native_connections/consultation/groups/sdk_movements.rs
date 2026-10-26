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
pub(super) struct HandoffProbe {
    client: Client,
    operation: sdk::Operation<sdk::HandoffResult>,
    config: CanonicalClientConfig,
}
pub(super) struct ReplacementProbe {
    client: Client,
    operation: sdk::Operation<sdk::MoveResult>,
    config: CanonicalClientConfig,
    pub baseline: (usize, usize, usize),
}
pub(super) async fn handoff(
    f: &Fixture,
    command: &TransitionCommand,
    token: &str,
    group: &Group,
) -> HandoffProbe {
    let config = super::super::sdk_controls::controller_config(f, token, 911);
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
    HandoffProbe {
        client,
        operation,
        config,
    }
}
pub(super) async fn handoff_completed(
    f: &Fixture,
    probe: HandoffProbe,
    command: &TransitionCommand,
    recorded: &Value,
) {
    let value = probe.operation.wait(options()).await.unwrap().value;
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
    let fresh = Client::connect(probe.config, "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let restored = fresh
        .restore(
            probe.operation.reference_json(),
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
    probe.client.close(options()).await.unwrap();
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
    let client = Client::connect(config.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let before = f.server.service.canonical_runtime_snapshot().unwrap();
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
        client,
        operation,
        config,
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
    let value = probe.operation.wait(options()).await.unwrap().value;
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
    let fresh = Client::connect(probe.config, "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let restored = fresh
        .restore(
            probe.operation.reference_json(),
            fresh.operation_kinds().connection_replace,
        )
        .unwrap();
    let replay = restored.wait(options()).await.unwrap().value;
    assert_eq!(replay.connection.id(), value.connection.id());
    assert_eq!(replay.binding.generation, value.binding.generation);
    assert!(replay.connection.native_negotiation().unwrap().is_none());
    fresh.close(options()).await.unwrap();
    probe.client.close(options()).await.unwrap();
    println!("SDK replacement {}:{} generation={},retained={},retired={},captured_topology,fresh_observation_no_custody",f.client.signing_algorithm,command.operation_id,value.binding.generation,command.intent.retained(),command.connection_id.as_ref().unwrap());
}
pub(super) async fn replacement_stopped(probe: ReplacementProbe, state: OperationState) {
    assert_eq!(
        probe.operation.status(options()).await.unwrap().state,
        state
    );
    assert!(
        matches!(probe.operation.wait(options()).await,Err(e) if e.certainty==sdk::Certainty::Recorded)
    );
    probe.client.close(options()).await.unwrap();
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
