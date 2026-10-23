//! Actual native/HTTP/SDK durable admission, scope, status and cancellation.
//! Trusted connection rows are control fixtures, not physical media qualification.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
use connect_client::{transitions::*, ConnectClient};
use connect_protocol::{Operation, Outcome, Response};
use connect_store::Scope;
use serde_json::json;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fresh_exchanges_and_current_credentials_share_one_durable_transition_receipt() {
    let mut f = agent::Fixture::start().await;
    let mut config = f.issue_as_operator(&f.agent, Scope::operator());
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let mut peer = f.peer_client().await;
    let cid = client
        .create("ledger-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let sid = client
        .session_invite("invite", &cid, &f.peer.identity_id, vec!["audio".into()])
        .await
        .unwrap()
        .id;
    peer.session_accept("accept", &cid, &sid).await.unwrap();
    let snapshot = f
        .server
        .store
        .membership_snapshot(&config.credential_id, &cid)
        .unwrap();
    let owner_part = snapshot
        .participants
        .iter()
        .find(|p| p.identity_id == config.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let server = connect_server::config::ServerConfig::load(&f.server_config_path()).unwrap();
    let c = rusqlite::Connection::open(&server.database).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    for (n, identity) in [&config.identity_id, &f.peer.identity_id]
        .into_iter()
        .enumerate()
    {
        let pid = &snapshot
            .participants
            .iter()
            .find(|p| &p.identity_id == identity)
            .unwrap()
            .participant_id;
        let device = format!("ledger-device-{n}");
        let ep = format!("ledger-endpoint-{n}");
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,'tnt_default','fixture','fixture')",rusqlite::params![device,identity]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES(?1,?2,'tnt_default',?3,'webrtc',?4,'[\"audio\"]','verified','fixture',?5)",rusqlite::params![ep,identity,device,format!("https://fixture.invalid/{n}"),chrono::Utc::now().timestamp()]).unwrap();
        c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state",rusqlite::params![sid,cid,pid]).unwrap();
        c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id) VALUES(?1,?2,?3,1,?4,'tnt_default',?5,1,'active',?6)",rusqlite::params![format!("ledger-conn-{n}"),sid,pid,identity,ep,f.server.store.runtime_epoch()]).unwrap();
    }
    drop(c);
    let request = TransitionRequest {
        payload_extensions: Default::default(),
        operation_id: "same-transition".into(),
        conversation_id: cid.clone(),
        session_id: sid,
        connection_id: None,
        expected_revision: 0,
        expires_at: chrono::Utc::now().timestamp() + 300,
        intent: TransitionIntent::Consult {
            retained_connid: "ledger-conn-1".into(),
            private_participants: vec![owner_part],
            hold_treatment: HoldTreatment::Silence,
        },
    };
    let record = client.admit_transition(request.clone()).await.unwrap();
    assert_eq!(record.state, OperationState::Accepted);
    assert_eq!(record.causal_event_ids.len(), 1);
    let mut base = format!("http://{}", f.server.http_address.unwrap());
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let op = Operation::Transition {
        request: Box::new(request.clone()),
    };
    let response: serde_json::Value = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(&config.credential)
        .json(&op)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let decoded: Response = serde_json::from_value(response.clone()).unwrap();
    assert_eq!(
        decoded.result.unwrap(),
        Outcome::DurableOperation(record.clone())
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let data=json!({"base":base,"token":config.credential,"foreign_token":f.peer.credential,"request":request,"record_id":record.record_id}).to_string();
    tokio::task::spawn_blocking(move || {
        for (program, script) in [
            ("node", "sdk/tests/operations_probe.mjs"),
            ("python3", "sdk/tests/operations_probe.py"),
        ] {
            let result = std::process::Command::new(program)
                .current_dir(&root)
                .env("PYTHONPATH", root.join("sdk/python"))
                .arg(script)
                .arg(&data)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "SDK probe: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap(),
                json!({"passed":true})
            );
        }
    })
    .await
    .unwrap();
    let get = Operation::DurableOperation {
        request: OperationAction::Get {
            operation_id: request.operation_id.clone(),
        },
    };
    let status: serde_json::Value = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(&config.credential)
        .json(&get)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let decoded: Response = serde_json::from_value(status.clone()).unwrap();
    assert_eq!(
        decoded.result.unwrap(),
        Outcome::DurableOperation(record.clone())
    );
    for bad in [
        json!({"op":"transition","request":{"owner_id":config.identity_id}}),
        json!({"op":"advance_transition","worker_generation":1}),
        json!({"op":"durable_operation","request":{"action":"reconcile","operation_id":"fake","target_operation_id":request.operation_id,"outcome":"completed","evidence_id":"unverified-client-claim","owner_id":config.identity_id}}),
    ] {
        assert_eq!(
            http.post(format!("{base}/v1/operations"))
                .bearer_auth(&config.credential)
                .json(&bad)
                .send()
                .await
                .unwrap()
                .status(),
            422
        );
    }
    let old_store = f.server.store.clone();
    let (old_lease, _) = old_store
        .claim_transition(&config.credential_id, &request.operation_id, 60)
        .unwrap()
        .unwrap();
    client.close().await;
    peer.close().await;
    f = f.restart().await;
    config.endpoint = f.server.address;
    assert_eq!(
        old_store
            .advance_transition(&config.credential_id, &old_lease, OperationState::Ready)
            .unwrap_err()
            .code,
        "forbidden"
    );
    client = ConnectClient::open(config.clone()).await.unwrap();
    peer = f.peer_client().await;
    assert_eq!(
        client
            .operation_action(OperationAction::Get {
                operation_id: request.operation_id.clone()
            })
            .await
            .unwrap()
            .state,
        OperationState::Interrupted
    );
    base = format!("http://{}", f.server.http_address.unwrap());
    let cancel = Operation::DurableOperation {
        request: OperationAction::Cancel {
            operation_id: "cancel-transition".into(),
            target_operation_id: request.operation_id.clone(),
            reason: Some("fixture cleanup".into()),
        },
    };
    let cancelled: serde_json::Value = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(&config.credential)
        .json(&cancel)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let decoded: Response = serde_json::from_value(cancelled.clone()).unwrap();
    let Outcome::DurableOperation(decision) = decoded.result.unwrap() else {
        panic!()
    };
    assert_eq!(
        decision.result.as_ref().unwrap()["state_at_decision"],
        "cancelled"
    );
    assert_eq!(
        client
            .operation_action(OperationAction::Get {
                operation_id: request.operation_id.clone()
            })
            .await
            .unwrap()
            .state,
        OperationState::Cancelled
    );
    assert_eq!(
        client.admit_transition(request).await.unwrap().record_id,
        record.record_id
    );
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_OPERATION_SAMPLES") {
        let dir = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, request, response) in [
            ("admit", op, response),
            ("status", get, status),
            ("cancel", cancel, cancelled),
        ] {
            std::fs::write(
                dir.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&json!({"request":request,"response":response})).unwrap(),
            )
            .unwrap();
        }
    }
    client.close().await;
    peer.close().await;
    f.close().await;
}
