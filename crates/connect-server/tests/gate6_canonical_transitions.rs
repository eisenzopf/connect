//! Signed dispatch, durable admission/refusal and replay using isolated adapter rows.
//! These rows do not qualify real media readiness, handoff or device migration.
#![allow(clippy::unwrap_used)]
use connect_client::canonical::{CanonicalClient, CanonicalClientConfig};
use connect_protocol::{transitions::*, Operation, Outcome};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::canonical_init,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;
struct Fixture {
    _temp: tempfile::TempDir,
    server: RunningCanonicalServer,
    credential: String,
    config: CanonicalClientConfig,
    http: reqwest::Client,
    cid: String,
    sid: String,
    parts: Vec<String>,
}

#[tokio::test]
async fn canonical_http_transition_commands_share_native_admission_and_current_operation_decisions()
{
    use connect_client::{
        application::ApplicationClientConfig,
        canonical::{CanonicalHttpClient, CommandScope},
    };
    for kind in 0..3 {
        let f = Fixture::new().await;
        let mut http = CanonicalHttpClient::open(ApplicationClientConfig {
            endpoint: f.server.http_address.unwrap(),
            credential: f.credential.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            timeout_ms: 10_000,
        })
        .await
        .unwrap();
        let mut native = CanonicalClient::open(f.config.clone()).await.unwrap();
        let command = f.command(kind, "http-transition");
        let admitted = http.transition(command.clone()).await.unwrap();
        assert_eq!(
            serde_json::to_value(admitted.state).unwrap(),
            json!("accepted")
        );
        let expiry = f.saved("http-transition").expires_at;
        let expected = if kind != 0 {
            f.wait_preflight_refusal(&mut native, "http-transition", kind)
                .await
        } else {
            serde_json::to_value(admitted).unwrap()
        };
        let replay = native.transition(command.clone()).await.unwrap();
        assert_eq!(expected, serde_json::to_value(replay).unwrap());
        assert_eq!(f.saved("http-transition").expires_at, expiry);
        let scope = CommandScope {
            conversation_id: Some(f.cid.clone()),
            session_id: Some(f.sid.clone()),
            connection_id: command.connection_id.clone(),
            ..Default::default()
        };
        http.operation_get("http-transition", scope.clone())
            .await
            .unwrap();
        let mut bad = scope.clone();
        bad.session_id = Some("foreign".into());
        assert!(
            matches!(http.operation_get("http-transition",bad).await,Err(connect_client::ClientError::Application(e)) if e.code=="forbidden")
        );
        let cancel=serde_json::from_value(json!({"type":"operation.cancel","payload":{"operation_id":"http-transition","reason":"cancel pending intent"}})).unwrap();
        let batch = http
            .request(
                cancel,
                CommandScope {
                    operation_id: Some("http-transition-cancel".into()),
                    ..scope.clone()
                },
            )
            .await
            .unwrap();
        batch.ack().unwrap();
        let target = http
            .operation_get("http-transition", scope.clone())
            .await
            .unwrap();
        let terminal = if kind != 0 { "failed" } else { "cancelled" };
        assert_eq!(serde_json::to_value(target.state).unwrap(), json!(terminal));
        if kind != 0 {
            assert_eq!(target.problem, Some(expected["problem"].clone()));
        }
        let evidence = f
            .server
            .store
            .record_transition_evidence(
                &f.server
                    .store
                    .authenticate(&f.credential)
                    .unwrap()
                    .credential_id,
                "http-transition",
                "trusted-terminal-proof",
                connect_store::EvidenceSource::LocalState,
                if kind != 0 {
                    ReconcileOutcome::Failed
                } else {
                    ReconcileOutcome::Cancelled
                },
                None,
            )
            .unwrap();
        let reconcile=serde_json::from_value(json!({"type":"operation.reconcile","payload":{"operation_id":"http-transition","outcome":terminal,"evidence_id":evidence}})).unwrap();
        let batch = http
            .request(
                reconcile,
                CommandScope {
                    operation_id: Some("http-transition-reconcile".into()),
                    ..scope
                },
            )
            .await
            .unwrap();
        batch.ack().unwrap();
        let c = Connection::open(f.server.store.path()).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&f.sid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM connections WHERE session_id=?1",
                [&f.sid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            3
        );
        drop(c);
        native.close();
        f.server.shutdown().await.unwrap();
    }
}
impl Fixture {
    // These adapter rows have no target policy reference or physical native
    // readiness. Handoff and replacement refuse these unowned fixture routes.
    // Wait for the owned definitive refusal instead of racing its
    // asynchronous update with a replay of the initial accepted receipt.
    async fn wait_preflight_refusal(
        &self,
        native: &mut CanonicalClient,
        id: &str,
        kind: usize,
    ) -> Value {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let value = loop {
            let status = native.operation_get(id, Default::default()).await.unwrap();
            let value = serde_json::to_value(status).unwrap();
            if value["state"] == "failed" {
                break value;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "Missing preflight refusal: {value}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        assert_eq!(
            value["problem"],
            if kind == 1 {
                json!({"code":"invalid-request","source":"native-handoff"})
            } else {
                json!({"code":"forbidden","source":"native-replacement"})
            }
        );
        assert_eq!(value["revision"], 1);
        assert!(value.get("result").is_none());
        let c = Connection::open(self.server.store.path()).unwrap();
        let effects: u64 = c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id=?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(effects, 0);
        let revision: u64 = c
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&self.sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(revision, 0);
        value
    }
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        canonical_init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let mut server_config = CanonicalServerConfig::load(&state.join("server.toml")).unwrap();
        server_config.http_bind = Some("127.0.0.1:0".parse().unwrap());
        let server = RunningCanonicalServer::start(server_config).await.unwrap();
        let mut config = CanonicalClientConfig::load(&state.join("operator.toml")).unwrap();
        config.endpoint = server.address;
        let a = CanonicalClientConfig::load(&state.join("reviews.toml")).unwrap();
        let b = CanonicalClientConfig::load(&state.join("agent.toml")).unwrap();
        let ids = vec![
            config.identity_id.clone(),
            a.identity_id.clone(),
            b.identity_id,
        ];
        let actor = server.store.authenticate(&config.credential).unwrap();
        let review = server
            .store
            .issue(
                &a.identity_id,
                connect_store::Scope::communications(ids.clone()),
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        let Outcome::Conversation(conv) = server
            .store
            .execute(
                &actor.credential_id,
                &Operation::Create {
                    request_id: "fixture-conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!("conversation")
        };
        let parts = ids
            .iter()
            .map(|id| {
                conv.participants
                    .iter()
                    .find(|p| &p.identity_id == id)
                    .unwrap()
                    .participant_id
                    .clone()
            })
            .collect::<Vec<_>>();
        let Outcome::Session(session) = server
            .store
            .execute(
                &actor.credential_id,
                &Operation::SessionInvite {
                    request_id: "fixture-session".into(),
                    conversation_id: conv.id.clone(),
                    participant_id: ids[1].clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!("session")
        };
        server
            .store
            .execute(
                &review.id,
                &Operation::SessionAccept {
                    request_id: "fixture-accept".into(),
                    conversation_id: conv.id.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        // Trusted fixture adapter observations only; no media/provider is contacted.
        let c = Connection::open(server.store.path()).unwrap();
        for n in 0..4 {
            let owner = if n == 3 { 0 } else { n };
            let device = format!("transition-device-{n}");
            let endpoint = format!("transition-endpoint-{n}");
            c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,'tnt_default','fixture','fixture')",params![device,ids[owner]]).unwrap();
            c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES(?1,?2,'tnt_default',?3,'webrtc',?4,'[\"audio\"]','verified','fixture-proof',?5)",params![endpoint,ids[owner],device,format!("https://fixture.invalid/transition/{n}"),chrono::Utc::now().timestamp()]).unwrap();
            if n == 3 {
                continue;
            }
            c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state",params![session.id,conv.id,parts[n]]).unwrap();
            c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id,observed_at,readiness_json) VALUES(?1,?2,?3,1,?4,'tnt_default',?5,1,?6,?7,?8,'{\"ingress\":true,\"egress\":true}')",params![format!("transition-connection-{n}"),session.id,parts[n],ids[n],endpoint,if n==2 {"ready"} else {"active"},server.store.runtime_epoch(),chrono::Utc::now().timestamp()]).unwrap();
        }
        let http = tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build())
            .await
            .unwrap()
            .unwrap();
        Self {
            _temp: temp,
            credential: config.credential.clone(),
            config,
            server,
            http,
            cid: conv.id,
            sid: session.id,
            parts,
        }
    }
    fn command(&self, kind: usize, id: &str) -> TransitionCommand {
        TransitionCommand {
            payload_extensions: Default::default(),
            operation_id: id.into(),
            conversation_id: self.cid.clone(),
            session_id: self.sid.clone(),
            connection_id: Some("transition-connection-0".into()),
            expected_revision: 0,
            intent: match kind {
                0 => TransitionIntent::Consult {
                    retained_connid: "transition-connection-1".into(),
                    private_participants: vec![self.parts[2].clone(), self.parts[0].clone()],
                    hold_treatment: HoldTreatment::Silence,
                },
                1 => TransitionIntent::Handoff {
                    source_participant: self.parts[0].clone(),
                    target_participant: self.parts[2].clone(),
                    retained_connid: "transition-connection-1".into(),
                    candidate_connid: Some("transition-connection-2".into()),
                    outgoing: OutgoingDisposition::Leave,
                    role_scope: RoleScope::Session,
                    expected_conversation_revision: None,
                },
                _ => TransitionIntent::Replace {
                    endpoint_id: Some("transition-endpoint-3".into()),
                    candidate_connid: None,
                    retained_connid: "transition-connection-1".into(),
                },
            },
        }
    }
    // Fixture intent is encoded as the current canonical wire request. There is
    // no production compatibility adapter or legacy operation endpoint here.
    async fn call(&self, name: &str, operation: Operation) -> (u16, Value) {
        let mut value = json!({"v":2,"id":format!("http_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now()});
        match operation {
            Operation::Transition { request } => {
                let mut payload = serde_json::to_value(request.intent).unwrap();
                let kind = payload["type"].as_str().unwrap().to_owned();
                payload
                    .as_object_mut()
                    .unwrap()
                    .retain(|k, v| k != "type" && !v.is_null());
                value["type"] = json!(kind);
                value["payload"] = payload;
                value["cid"] = json!(request.conversation_id);
                value["sid"] = json!(request.session_id);
                value["operation_id"] = json!(request.operation_id);
                value["expected_revision"] = json!(request.expected_revision);
                if let Some(connid) = request.connection_id {
                    value["connid"] = json!(connid);
                }
            }
            Operation::DurableOperation { request } => match request {
                OperationAction::Get { operation_id } => {
                    value["type"] = json!("operation.get");
                    value["payload"] = json!({"operation_id":operation_id});
                }
                OperationAction::Cancel {
                    operation_id,
                    target_operation_id,
                    reason,
                } => {
                    value["type"] = json!("operation.cancel");
                    value["operation_id"] = json!(operation_id);
                    value["payload"] = json!({"operation_id":target_operation_id});
                    if let Some(reason) = reason {
                        value["payload"]["reason"] = json!(reason);
                    }
                }
                _ => panic!("Unconfigured fixture action"),
            },
            _ => panic!("Unconfigured fixture command"),
        }
        let response = self
            .http
            .post(format!(
                "http://{}/v2/commands",
                self.server.http_address.unwrap()
            ))
            .bearer_auth(&self.credential)
            .json(&value)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let response: Value = response.json().await.unwrap();
        if let Some(directory) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("canonical-transition-{name}.json")),serde_json::to_vec(&json!({"api":"canonical","route":"/v2/commands","request":value,"valid_request":true,"status":status,"response":response})).unwrap()).unwrap();
        }
        (status, response)
    }
    fn saved(&self, id: &str) -> TransitionRequest {
        let c = Connection::open(self.server.store.path()).unwrap();
        let raw: String = c
            .query_row(
                "SELECT intent_json FROM operations WHERE operation_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        let mut value: Value = serde_json::from_str(&raw).unwrap();
        value["operation_id"] = json!(id);
        serde_json::from_value(value).unwrap()
    }
}
#[tokio::test]
async fn signed_consult_handoff_replace_share_http_receipts_and_cancellation_without_claiming_media(
) {
    for kind in 0..3 {
        let f = Fixture::new().await;
        let mut native = CanonicalClient::open(f.config.clone()).await.unwrap();
        let command = f.command(kind, "signed-transition");
        let first = native.transition(command.clone()).await.unwrap();
        assert_eq!(
            serde_json::to_value(first.state).unwrap(),
            json!("accepted")
        );
        assert!(first.result.is_none());
        let expected = if kind != 0 {
            f.wait_preflight_refusal(&mut native, "signed-transition", kind)
                .await
        } else {
            serde_json::to_value(&first).unwrap()
        };
        let (status, status_body) = f
            .call(
                &format!("{kind}-status"),
                Operation::DurableOperation {
                    request: OperationAction::Get {
                        operation_id: "signed-transition".into(),
                    },
                },
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(status_body["envelopes"][0]["payload"], expected);
        let saved = f.saved("signed-transition");

        assert_eq!(
            serde_json::to_value(native.transition(command.clone()).await.unwrap()).unwrap(),
            expected
        );
        let (status, http) = f
            .call(
                &format!("{kind}-replay"),
                Operation::Transition {
                    request: Box::new(saved.clone()),
                },
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(http["envelopes"][0]["payload"], expected);
        assert_eq!(f.saved("signed-transition").expires_at, saved.expires_at);
        let mut changed = command.clone();
        changed.expected_revision = 9;
        assert!(
            matches!(native.transition(changed).await,Err(connect_client::ClientError::Application(e)) if e.code=="conflict")
        );
        let (status, _) = f
            .call(
                &format!("{kind}-cancel"),
                Operation::DurableOperation {
                    request: OperationAction::Cancel {
                        operation_id: "cancel-signed-transition".into(),
                        target_operation_id: "signed-transition".into(),
                        reason: None,
                    },
                },
            )
            .await;
        assert_eq!(status, 200);
        let record = native
            .operation_get("signed-transition", Default::default())
            .await
            .unwrap();
        let terminal = if kind != 0 { "failed" } else { "cancelled" };
        assert_eq!(serde_json::to_value(record.state).unwrap(), json!(terminal));
        if kind != 0 {
            assert_eq!(record.problem, Some(expected["problem"].clone()));
        }
        let retry = native.transition(command).await.unwrap();
        assert_eq!(serde_json::to_value(retry.state).unwrap(), json!(terminal));
        assert_eq!(f.saved("signed-transition").expires_at, saved.expires_at);
        let c = Connection::open(f.server.store.path()).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='signed-transition'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT state FROM connections WHERE id='transition-connection-0'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        assert_eq!(
            c.query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&f.sid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        native.shutdown().await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn http_first_transition_retains_original_expiry_and_lost_source_policy_blocks_signed_replay()
{
    let f = Fixture::new().await;
    let mut native = CanonicalClient::open(f.config.clone()).await.unwrap();
    let command = f.command(0, "http-first");
    let request = command.with_expiry(chrono::Utc::now().timestamp() + 300);
    let (status, value) = f
        .call(
            "http-first",
            Operation::Transition {
                request: Box::new(request.clone()),
            },
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(value["envelopes"][0]["payload"]["state"], "accepted");
    let first = native.transition(command.clone()).await.unwrap();
    assert_eq!(
        serde_json::to_value(first.state).unwrap(),
        json!("accepted")
    );
    let original_expiry = f.saved("http-first").expires_at;
    assert!(
        original_expiry > chrono::Utc::now().timestamp()
            && original_expiry <= chrono::Utc::now().timestamp() + MAX_OPERATION_TTL
    );
    let actor = f.server.store.authenticate(&f.credential).unwrap();
    let revision = f
        .server
        .store
        .membership_snapshot(&actor.credential_id, &f.cid)
        .unwrap()
        .revision;
    let remove = serde_json::from_value(json!({"type":"conversation.participant.remove","payload":{"participant_id":f.parts[2],"reason":"fixture source removed"}})).unwrap();
    native
        .participant_change(&f.cid, "remove-transition-source", Some(revision), remove)
        .await
        .unwrap();
    // Departure does not erase an otherwise authorized owner's receipt.
    let known = native.transition(command.clone()).await.unwrap();
    assert_eq!(
        serde_json::to_value(known.state).unwrap(),
        json!("accepted")
    );
    assert_eq!(
        f.server
            .store
            .claim_transition(&actor.credential_id, "http-first", 10)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let c = Connection::open(f.server.store.path()).unwrap();
    // Fault fixture: current issuer policy removes this source recipient.
    // Retain the real Identity writer while changing current credential metadata.
    {
        let _writer = f
            .server
            .store
            .acquire_authority(
                &[connect_store::AuthorityScope::identity(
                    &actor.identity_id,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap()],
                &connect_store::AuthorityWait::new(std::time::Duration::from_secs(1)),
            )
            .unwrap();
        let recipients = serde_json::to_string(&vec![
            f.config.identity_id.clone(),
            CanonicalClientConfig::load(&f._temp.path().join("state/reviews.toml"))
                .unwrap()
                .identity_id,
        ])
        .unwrap();
        c.execute(
            "UPDATE credentials SET recipients=?2 WHERE id=?1",
            params![actor.credential_id, recipients],
        )
        .unwrap();
    }
    let replay = native.transition(command).await;
    assert!(
        matches!(&replay,Err(connect_client::ClientError::Application(e)) if e.code=="forbidden"),
        "{replay:?}"
    );
    let (status, value) = f
        .call(
            "lost-source",
            Operation::Transition {
                request: Box::new(request),
            },
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(value["envelopes"][0]["payload"]["reason"], "forbidden");
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='http-first'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    native.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn signed_transition_invalid_resource_and_missing_permission_have_no_ledger_effect() {
    let f = Fixture::new().await;
    let mut native = CanonicalClient::open(f.config.clone()).await.unwrap();
    let mut command = f.command(0, "unavailable-resource");
    command.session_id = "unknown-session".into();
    assert!(
        matches!(native.transition(command).await,Err(connect_client::ClientError::Application(e)) if e.code=="forbidden")
    );
    let mut invalid = f.command(2, "invalid-replacement");
    invalid.connection_id = None;
    assert!(
        matches!(native.transition(invalid).await,Err(connect_client::ClientError::Application(e)) if e.code=="invalid-request")
    );
    native.shutdown().await.unwrap();
    let actor = f.server.store.authenticate(&f.credential).unwrap();
    let c = Connection::open(f.server.store.path()).unwrap();
    c.execute(
        "UPDATE credentials SET operations='[\"health\"]' WHERE id=?1",
        [actor.credential_id],
    )
    .unwrap();
    let mut reduced = CanonicalClient::open(f.config.clone()).await.unwrap();
    assert!(
        matches!(reduced.transition(f.command(0,"missing-permission")).await,Err(connect_client::ClientError::Application(e)) if e.code=="forbidden")
    );
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operations WHERE operation_id IN ('unavailable-resource','invalid-replacement','missing-permission')",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    reduced.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn signed_and_http_transition_extensions_share_exact_original_intent_and_refuse_changed_replay(
) {
    use connect_client::{application::ApplicationClientConfig, canonical::CanonicalHttpClient};
    for kind in 0..3 {
        let f = Fixture::new().await;
        let mut native = CanonicalClient::open(f.config.clone()).await.unwrap();
        let mut http = CanonicalHttpClient::open(ApplicationClientConfig {
            endpoint: f.server.http_address.unwrap(),
            credential: f.credential.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            timeout_ms: 10000,
        })
        .await
        .unwrap();
        let mut command = f.command(kind, "transition-extensions");
        command.payload_extensions = std::collections::BTreeMap::from([
            (
                "future".into(),
                json!({"ordered":[2,1],"label":"λ","assertion":{"approved":true}}),
            ),
            ("null-value".into(), json!(null)),
            ("type".into(), json!("optional-payload-type")),
        ]);
        let original = native.transition(command.clone()).await.unwrap();
        let saved = f.saved("transition-extensions");
        assert_eq!(saved.payload_extensions, command.payload_extensions);
        let expected = if kind != 0 {
            f.wait_preflight_refusal(&mut native, "transition-extensions", kind)
                .await
        } else {
            serde_json::to_value(&original).unwrap()
        };
        let replay = http.transition(command.clone()).await.unwrap();
        assert_eq!(expected, serde_json::to_value(replay).unwrap());
        let c = Connection::open(f.server.store.path()).unwrap();
        let before:(u64,u64,u64)=c.query_row("SELECT (SELECT COUNT(*) FROM operations),(SELECT COUNT(*) FROM operation_effects),(SELECT COUNT(*) FROM journal)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        drop(c);
        let mut changed = command.clone();
        changed.payload_extensions.get_mut("future").unwrap()["ordered"] = json!([1, 2]);
        assert!(native.transition(changed.clone()).await.is_err());
        assert!(http.transition(changed).await.is_err());
        let mut absent = command.clone();
        absent.payload_extensions.remove("null-value");
        assert!(http.transition(absent).await.is_err());
        let mut override_known = command;
        override_known
            .payload_extensions
            .insert("retained_connid".into(), json!("foreign"));
        assert!(native.transition(override_known.clone()).await.is_err());
        assert!(http.transition(override_known).await.is_err());
        assert_eq!(f.saved("transition-extensions"), saved);
        let c = Connection::open(f.server.store.path()).unwrap();
        let after:(u64,u64,u64)=c.query_row("SELECT (SELECT COUNT(*) FROM operations),(SELECT COUNT(*) FROM operation_effects),(SELECT COUNT(*) FROM journal)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(after, before);
        drop(c);
        native.close();
        f.server.shutdown().await.unwrap();
    }
}
