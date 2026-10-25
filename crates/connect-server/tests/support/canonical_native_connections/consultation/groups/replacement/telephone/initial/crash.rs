//! The actual UDP SIP peer survives SIGKILL of the canonical server process.
use super::*;
use crate::canonical_native_connections::crash_recovery::{Child, Scratch};
use serde::{Deserialize, Serialize};
use std::{fs::File, os::unix::process::ExitStatusExt, process::Stdio};

const CHILD: &str = "canonical_native_connections::consultation::groups::replacement::telephone::initial::crash::sip_submission_child";
const ROOT: &str = "CONNECT_G6_SIP_CRASH_ROOT";
const PEER: &str = "CONNECT_G6_SIP_CRASH_PEER";
const ALGORITHM: &str = "CONNECT_G6_SIP_CRASH_ALGORITHM";

#[derive(Serialize, Deserialize)]
struct Prepared {
    config: CanonicalServerConfig,
    client: CanonicalClientConfig,
    epoch: String,
    installation: String,
    invitation: connect_protocol::session_control::Request,
}
fn marker<T: Serialize>(root: &Path, name: &str, value: &T) {
    let pending = root.join(format!("{name}.pending"));
    std::fs::write(&pending, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::rename(pending, root.join(format!("{name}.json"))).unwrap();
}
async fn wait_file(root: &Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !root.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("parent did not release the prepared child");
}

// Invoked explicitly by each parent; never a skipped qualification vector.
#[ignore = "subprocess entry point; parent requires actual external SIP IO then SIGKILL"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sip_submission_child() {
    let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
    let algorithm = std::env::var(ALGORITHM).unwrap();
    let address = std::env::var(PEER).unwrap().parse().unwrap();
    let (f, _, _) = configured_initial_at(
        &algorithm,
        30,
        EndpointTransport::Telephone,
        true,
        true,
        None,
        address,
    )
    .await;
    assert!(f.config.database.starts_with(&root));
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("crash-sip-room", input)
        .await
        .unwrap();
    let participant = |identity: &str| {
        room.participants
            .iter()
            .find(|p| p.identity_id == identity)
            .unwrap()
            .participant_id
            .clone()
    };
    let invitation = connect_protocol::session_control::Request {
        operation_id: "crash-sip-invite".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        by_participant: Some(participant(&f.client.identity_id)),
        expected_revision: None,
        action: connect_protocol::session_control::Action::Invite {
            participants: vec![participant(&f.peer.identity_id)],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    let mut config = f.config.clone();
    config.bind = f.server.address;
    marker(
        &root,
        "prepared",
        &Prepared {
            config,
            client: f.client.clone(),
            epoch: f.server.store.runtime_epoch().into(),
            installation: f.server.store.installation_id().unwrap(),
            invitation: invitation.clone(),
        },
    );
    wait_file(&root, "proceed").await;
    let receipt = client.session_command(invitation).await.unwrap();
    marker(&root, "submitted", &receipt);
    std::future::pending::<()>().await;
    drop((client, f));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sigkill_sip_submission_keeps_external_uncertainty_without_redial_eddsa() {
    submitted("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sigkill_sip_submission_keeps_external_uncertainty_without_redial_es256() {
    submitted("ES256").await;
}

async fn submitted(algorithm: &str) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let scratch = workspace.join(".local/gate-6/recovery-process");
    std::fs::create_dir_all(&scratch).unwrap();
    let temp = Scratch(Some(tempfile::tempdir_in(scratch).unwrap()));
    let peer = PeerSip::new().await;
    let log = temp.path().join("child.log");
    let output = File::create(&log).unwrap();
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                CHILD,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROOT, temp.path())
            .env(PEER, peer.address.to_string())
            .env(ALGORITHM, algorithm)
            .env("TMPDIR", temp.path())
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output))
            .spawn()
            .unwrap(),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !temp.path().join("prepared.json").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child failed: {}",
            log.display()
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "child setup timed out: {}",
            log.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let prepared: Prepared =
        serde_json::from_slice(&std::fs::read(temp.path().join("prepared.json")).unwrap()).unwrap();
    *peer.ledger.lock().unwrap() = Some(prepared.config.database.clone());
    std::fs::write(temp.path().join("proceed"), b"go").unwrap();
    until(|| peer.unique() == 1 && temp.path().join("submitted.json").exists()).await;
    assert!(!peer.task.is_finished(), "external SIP fixture failed");
    let db = rusqlite::Connection::open_with_flags(
        &prepared.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let original:(String,String,String,String,String,u64)=db.query_row(
        "SELECT o.operation_id,o.record_id,o.connection_id,i.sip_call_id,e.id,e.attempt FROM operations o JOIN initial_telephone_operations i ON i.operation_record_id=o.record_id JOIN operation_effects e ON e.id=i.effect_id WHERE i.signaling_started=1 AND o.state='preparing' AND e.state='submitting'",[],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(original.5, 1);
    assert_eq!(peer.calls.lock().unwrap()[0], original.3);
    assert_eq!(
        db.query_row("SELECT count(*) FROM initial_telephone_answers", [], |r| {
            r.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
    child.0.kill().unwrap();
    let death = child.0.wait().unwrap();
    assert_eq!(death.signal(), Some(9));
    assert!(
        !peer.task.is_finished(),
        "external peer must outlive the killed host"
    );
    assert!(
        !peer
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| matches!(m, Method::Cancel | Method::Bye)),
        "SIGKILL ran cleanup"
    );
    let server = RunningCanonicalServer::start(prepared.config.clone())
        .await
        .unwrap();
    assert_ne!(server.store.runtime_epoch(), prepared.epoch);
    assert_eq!(
        server.store.installation_id().unwrap(),
        prepared.installation
    );
    let mut http =
        CanonicalHttpClient::open(connect_client::application::ApplicationClientConfig {
            endpoint: server.http_address.unwrap(),
            credential: prepared.client.credential.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            timeout_ms: 5000,
        })
        .await
        .unwrap();
    let scope = CommandScope {
        conversation_id: Some(prepared.invitation.conversation_id.clone()),
        ..Default::default()
    };
    let old = http
        .operation_get(&original.0, scope.clone())
        .await
        .unwrap();
    assert_eq!(old.state, rvoip_uctp::v2::payloads::OperationState::Unknown);
    assert_eq!(old.record_id.as_ref().unwrap().as_str(), original.1);
    let physical:(String,u64,Option<String>,Option<String>)=db.query_row("SELECT state,generation,observed_runtime_id,readiness_json FROM connections WHERE id=?1",[&original.2],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(physical.0, "unknown");
    assert!(physical.1 > 1 && physical.2.is_none() && physical.3.is_none());
    let effect: (String, u64) = db
        .query_row(
            "SELECT state,attempt FROM operation_effects WHERE id=?1",
            [&original.4],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(effect, ("unknown".into(), 1));
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("submitted.json")).unwrap())
            .unwrap();
    let replay = http
        .session_command(prepared.invitation.clone())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), receipt);
    let operator = server
        .store
        .issue(
            &prepared.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let actor = server.store.authenticate(&operator.token).unwrap();
    let reconcile = server.store.execute_operation_action(
        &actor.credential_id,
        connect_protocol::transitions::OperationAction::Reconcile {
            operation_id: "crash-sip-false-reconcile".into(),
            target_operation_id: original.0.clone(),
            outcome: connect_protocol::transitions::ReconcileOutcome::Completed,
            evidence_id: "caller-declared-completion".into(),
        },
    );
    assert_eq!(reconcile.unwrap_err().code, "reconciliation-required");
    // The public HTTP operator reaches the same uncertainty boundary with
    // current permission and exact resource assertions after actual SIGKILL.
    let mut operator_http =
        CanonicalHttpClient::open(connect_client::application::ApplicationClientConfig {
            endpoint: server.http_address.unwrap(),
            credential: operator.token,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            timeout_ms: 5000,
        })
        .await
        .unwrap();
    let refusal = operator_http.request(
        serde_json::from_value(json!({"type":"operation.reconcile","payload":{
            "operation_id":original.0,"outcome":"completed","evidence_id":"caller-declared-completion"
        }})).unwrap(),
        CommandScope {operation_id:Some("crash-sip-operator-wire-review".into()),
            conversation_id:Some(prepared.invitation.conversation_id.clone()),
            session_id:Some(prepared.invitation.session_id.clone()), connection_id:Some(original.2.clone()),
            ..Default::default()},
    ).await.unwrap();
    let refusal = refusal.ack();
    assert!(
        matches!(refusal,Err(ClientError::Application(error)) if error.code=="conflict"),
        "authorized public reconciliation must report unresolved physical custody"
    );

    // A fresh invitation ID cannot evade unresolved custody in the same Session.
    let mut conflicting = prepared.invitation.clone();
    conflicting.operation_id = "crash-sip-conflicting-invite".into();
    let _ = http.session_command(conflicting).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!peer.task.is_finished());
    assert_eq!(peer.unique(), 1);
    assert_eq!(
        peer.calls.lock().unwrap().len(),
        1,
        "successor/replay sent another INVITE"
    );
    let after = http.operation_get(&original.0, scope).await.unwrap();
    assert_eq!(
        serde_json::to_value(&old).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM initial_telephone_operations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM initial_telephone_answers", [], |r| {
            r.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let runtime = server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(runtime.connections, 0);
    assert_eq!(runtime.media_readers, 0);
    if let Some(prefix) = std::env::var_os("CONNECT_G6_SIP_CRASH_CAPTURE") {
        let prefix = std::path::PathBuf::from(prefix);
        assert!(prefix.is_absolute() && prefix.starts_with(workspace.join(".local/gate-6")));
        std::fs::write(prefix.with_extension(format!("{algorithm}.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"phase":"submitted","killed_signal":death.signal(),"external_peer_still_live":true,"unique_external_calls":peer.unique(),"original_effect_attempt":effect.1,"original_effect_state":effect.0,"operation_state":old.state,"physical_after_restart":physical,"new_epoch_same_installation":true,"replay_preserved_receipt_and_original_operation":true,"unresolved_conflict_did_not_redial":true,"caller_declared_completion_refused":true,"session_started_facts":0,"runtime_connections":0,"runtime_media_readers":0})).unwrap()).unwrap();
    }
    drop(http);
    drop(db);
    server.shutdown().await.unwrap();
    assert!(!peer.task.is_finished());
    assert_eq!(peer.unique(), 1);
}
