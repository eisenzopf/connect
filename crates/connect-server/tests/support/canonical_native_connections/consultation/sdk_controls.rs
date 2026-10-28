//! Public Rust SDK controls across actual managed private audio and restoration.
use super::*;
use connect_client::sdk::{
    self, Client, ConsultInput, Hold, MutationOptions, NonEmpty, RequestOptions,
};
pub(super) fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(35)).unwrap()
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
pub(super) struct RustProbe {
    pub client: Client,
    pub reference: Value,
    operation: sdk::Operation<sdk::ConsultationResult>,
    config: CanonicalClientConfig,
}
pub(in crate::canonical_native_connections::consultation) fn controller_config(
    f: &Fixture,
    token: &str,
    index: usize,
) -> CanonicalClientConfig {
    let mut config = party_with_scope(
        f,
        &f.client.identity_id,
        index,
        vec!["audio".into()],
        Scope::collaboration(vec!["*".into()]),
    );
    config.credential = token.into();
    config.signing_key_id = format!("sdk-consultation-controller-key-{index}");
    config.signing_private_key = f
        .temp
        .path()
        .join(format!("sdk-consultation-controller-{index}.der"));
    let public = signing(&config.signing_private_key, &config.signing_algorithm);
    let credential = f.server.store.authenticate(token).unwrap().credential_id;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: format!("sdk-consultation-key-enroll-{index}"),
                identity_id: f.client.identity_id.clone(),
                credential_id: credential,
                device_id: config.device_id.clone(),
                key_id: config.signing_key_id.clone(),
                algorithm: if config.signing_algorithm == "ES256" {
                    connect_protocol::provisioning::SigningAlgorithm::ES256
                } else {
                    connect_protocol::provisioning::SigningAlgorithm::EdDSA
                },
                public_key: base64::Engine::encode(
                    &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                    public,
                ),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    config
}
async fn submit_rust(f: &Fixture, command: &TransitionCommand, token: &str) -> RustProbe {
    let config = controller_config(f, token, 901);
    let client = Client::connect(config.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let TransitionIntent::Consult {
        retained_connid,
        private_participants,
        hold_treatment,
    } = &command.intent
    else {
        panic!("consult intent");
    };
    let session = client
        .conversation_by_id(command.conversation_id.parse().unwrap())
        .session_by_id(command.session_id.parse().unwrap());
    let participants = private_participants
        .iter()
        .map(|id| id.parse::<sdk::ParticipantId>().unwrap())
        .collect::<Vec<_>>();
    let operation = session
        .consult(
            ConsultInput {
                via: command.connection_id.as_ref().unwrap().parse().unwrap(),
                retain: retained_connid.parse().unwrap(),
                with_participants: NonEmpty {
                    first: participants[0].clone(),
                    rest: participants[1..].to_vec(),
                },
                hold: match hold_treatment {
                    HoldTreatment::Silence => Hold::Silence,
                    HoldTreatment::ConfiguredMedia => Hold::ConfiguredMedia,
                },
                expected_topology_revision: sdk::TopologyRevision::try_from(
                    command.expected_revision,
                )
                .unwrap(),
            },
            mutation(&command.operation_id),
        )
        .await
        .unwrap();
    let snapshot = operation.status(options()).await.unwrap();
    assert_ne!(
        snapshot.state,
        sdk::OperationState::Completed,
        "admission cannot claim a private audio switch"
    );
    assert!(snapshot.record_id.is_some());
    RustProbe {
        reference: operation.reference_json(),
        client,
        operation,
        config,
    }
}
async fn completed_rust(f: &Fixture, probe: &RustProbe, revision: u64) {
    let completion = probe.operation.wait(options()).await.unwrap();
    assert_eq!(completion.value.topology.topology_revision.get(), revision);
    assert_eq!(completion.value.topology.admitted.len(), 1);
    assert_eq!(
        completion.value.topology.admitted[0].id().as_str(),
        "private-consultant"
    );
    assert!(completion.value.topology.admitted[0]
        .native_negotiation()
        .unwrap()
        .is_none());
    let record = completion.snapshot.record_id.unwrap();
    let refused = probe
        .operation
        .return_to_retained(
            sdk::TopologyRevision::try_from(revision).unwrap(),
            mutation("sdk-return-without-grant"),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.certainty, sdk::Certainty::Refused);
    assert_eq!(
        refused.operation.as_ref().unwrap().operation_id.as_str(),
        "sdk-return-without-grant"
    );
    let rows: i64 = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-return-without-grant'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        rows, 0,
        "consult permission cannot reserve a return operation"
    );

    let fresh = Client::connect(probe.config.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let restored = fresh
        .restore(
            probe.reference.clone(),
            fresh.operation_kinds().session_consult,
        )
        .unwrap();
    let replay = restored.wait(options()).await.unwrap();
    assert_eq!(replay.snapshot.record_id.as_ref(), Some(&record));
    assert_eq!(replay.value.topology.topology_revision.get(), revision);
    assert!(replay.value.topology.admitted[0]
        .native_negotiation()
        .unwrap()
        .is_none());
    fresh.close(options()).await.unwrap();
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let attempts: i64 = db
        .query_row(
            "SELECT SUM(attempt) FROM operation_effects WHERE operation_record_id=?1",
            [record.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 1);
    println!("SDK consultation {}:completed_topology={revision},original_record,fresh_observation_without_custody,one_native_attempt",f.client.signing_algorithm);
}
async fn restore_rust(
    f: &Fixture,
    token: &str,
    reference: Value,
    revision: u64,
    continuing: bool,
    retired: &[&str],
) -> (Client, sdk::Operation<sdk::Session>) {
    let config = controller_config(f, token, 902);
    let client = Client::connect(config, "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let consultation = client
        .restore(reference, client.operation_kinds().session_consult)
        .unwrap();
    let operation = consultation
        .return_to_retained(
            sdk::TopologyRevision::try_from(revision).unwrap(),
            mutation("return-private-native"),
        )
        .await
        .unwrap_or_else(|error|{
            let actor=f.server.store.authenticate(token);
            let db=rusqlite::Connection::open(&f.config.database).unwrap();
            let rows:Vec<(String,String)>=db.prepare("SELECT command_type,state FROM operations WHERE operation_id='return-private-native'").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().map(Result::unwrap).collect();
            panic!("SDK return refused:kind={:?},certainty={:?},problem={:?},fresh_credential_active={},operation_rows={rows:?}",error.kind,error.certainty,error.problem,actor.is_ok());
        });
    let saved = operation.reference_json();
    let result = operation.wait(options()).await.unwrap().value;
    let receipt = result.consultation_return_receipt().unwrap();
    assert_eq!(receipt.remote_continuing, continuing);
    assert_eq!(receipt.topology.topology_revision.get(), revision + 1);
    assert_eq!(
        receipt
            .topology
            .retired
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        retired
    );
    assert_eq!(
        receipt.consultation_record_id.as_str(),
        saved["continuation"]["consultation_record_id"]
            .as_str()
            .unwrap()
    );
    let restored = client
        .restore(saved, client.operation_kinds().session_consult_return)
        .unwrap();
    assert_eq!(
        restored
            .wait(options())
            .await
            .unwrap()
            .value
            .consultation_return_receipt()
            .unwrap()
            .remote_continuing,
        continuing
    );
    println!("SDK return {}:remote_continuing={continuing},topology={},retired={},fresh_permission_original_record",f.client.signing_algorithm,revision+1,retired.len());
    (client, operation)
}
#[tokio::test]
async fn production_public_rust_sdk_consultation_return_retains_actual_private_and_restored_audio()
{
    for algorithm in ["EdDSA", "ES256"] {
        private_candidate_case_with_auxiliary_checks(
            true,
            false,
            false,
            true,
            false,
            None,
            false,
            false,
            Some(Auxiliary::Sdk(algorithm)),
        )
        .await;
    }
}
#[tokio::test]
async fn production_public_rust_sdk_consultation_return_after_remote_eof_reports_no_continuing_call(
) {
    for algorithm in ["EdDSA", "ES256"] {
        private_candidate_case_with_auxiliary_checks(
            true,
            true,
            false,
            true,
            false,
            None,
            false,
            false,
            Some(Auxiliary::Sdk(algorithm)),
        )
        .await;
    }
}

#[path = "node_controls.rs"]
mod node_controls;
pub(super) use node_controls::MovementProbe as NodeMovementProbe;

#[derive(Clone)]
pub(super) struct Saved {
    reference: Value,
    node: bool,
}
pub(super) enum Probe {
    Rust(RustProbe),
    Node(node_controls::Probe),
}
impl Probe {
    pub(super) fn reference(&self) -> Saved {
        match self {
            Self::Rust(p) => Saved {
                reference: p.reference.clone(),
                node: false,
            },
            Self::Node(p) => Saved {
                reference: p.reference.clone(),
                node: true,
            },
        }
    }
    pub(super) async fn revoked(&self) {
        match self {
            Self::Rust(p) => {
                let error = p
                    .operation
                    .status(options())
                    .await
                    .err()
                    .expect("revoked consultation authority cannot read");
                let refused = error.certainty == sdk::Certainty::Refused
                    || (error.certainty == sdk::Certainty::ReadOnly
                        && matches!(
                            error.kind,
                            sdk::ErrorKind::Authentication | sdk::ErrorKind::Cancelled
                        ));
                if !refused {
                    let encoded = sdk::helper::error_json(&error);
                    eprintln!(
                        "sdk-current-read-failure {}",
                        json!({"stage":"revoked-consultation-status","kind":encoded["kind"],"certainty":encoded["certainty"]})
                    );
                }
                assert!(refused);
            }
            Self::Node(p) => p.revoked().await,
        }
    }
    pub(super) async fn close(self) {
        match self {
            Self::Rust(p) => p.client.close(options()).await.unwrap(),
            Self::Node(p) => p.close().await,
        }
    }
}
pub(super) async fn submit(
    f: &Fixture,
    command: &TransitionCommand,
    token: &str,
    node: bool,
) -> Probe {
    if node {
        Probe::Node(node_controls::submit(f, command, token).await)
    } else {
        Probe::Rust(submit_rust(f, command, token).await)
    }
}
pub(super) async fn completed(f: &Fixture, probe: &Probe, revision: u64) {
    match probe {
        Probe::Rust(p) => completed_rust(f, p, revision).await,
        Probe::Node(p) => p.completed(f, revision).await,
    }
}
pub(super) enum ReturnProbe {
    Rust(Client, sdk::Operation<sdk::Session>),
    Node(node_controls::Probe),
}
pub(super) async fn restore(
    f: &Fixture,
    token: &str,
    saved: Saved,
    revision: u64,
    continuing: bool,
    retired: &[&str],
) -> ReturnProbe {
    if saved.node {
        ReturnProbe::Node(node_controls::restore(f, token, saved.reference, revision).await)
    } else {
        let (client, operation) =
            restore_rust(f, token, saved.reference, revision, continuing, retired).await;
        ReturnProbe::Rust(client, operation)
    }
}
impl ReturnProbe {
    pub(super) async fn revoked_and_close(self) {
        match self {
            Self::Node(p) => {
                p.revoked().await;
                p.close().await;
            }
            Self::Rust(client, operation) => {
                let error = operation
                    .status(options())
                    .await
                    .err()
                    .expect("revoked return authority cannot read");
                let refused = error.certainty == sdk::Certainty::Refused
                    || (error.certainty == sdk::Certainty::ReadOnly
                        && matches!(
                            error.kind,
                            sdk::ErrorKind::Authentication | sdk::ErrorKind::Cancelled
                        ));
                if !refused {
                    let encoded = sdk::helper::error_json(&error);
                    eprintln!(
                        "sdk-current-read-failure {}",
                        json!({"stage":"revoked-return-read","kind":encoded["kind"],"certainty":encoded["certainty"]})
                    );
                }
                assert!(
                    refused,
                    "revoked return read must refuse or retire authenticated peer: {error:?}"
                );
                client.close(options()).await.unwrap();
            }
        }
    }
}

pub(super) async fn node_drained(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = f.server.service.canonical_runtime_snapshot().unwrap();
            if state.physical_peers == 0 && state.media_readers == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
