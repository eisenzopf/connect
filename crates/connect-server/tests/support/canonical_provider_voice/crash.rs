//! A parent-owned HTTP provider survives an actual canonical worker SIGKILL.
use super::*;
use serde::{Deserialize, Serialize};
use std::{fs::File, os::unix::process::ExitStatusExt, process::Stdio};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};
const CHILD: &str = "canonical_provider_voice::crash::provider_creation_child";
const ROOT: &str = "CONNECT_G6_PROVIDER_CRASH_ROOT";
const API: &str = "CONNECT_G6_PROVIDER_CRASH_API";
const ALGORITHM: &str = "CONNECT_G6_PROVIDER_CRASH_ALGORITHM";
#[derive(Serialize, Deserialize)]
struct Prepared {
    config: CanonicalServerConfig,
    client: CanonicalClientConfig,
    policy: ProviderSessionVoicePolicy,
    epoch: String,
    installation: String,
    invitation: connect_protocol::session_control::Request,
    receipt: serde_json::Value,
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
struct Scratch(Option<tempfile::TempDir>);
impl Scratch {
    fn path(&self) -> &Path {
        self.0.as_ref().unwrap().path()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "Retained failed provider crash fixture: {}",
                self.0.take().unwrap().keep().display()
            );
        }
    }
}
fn driver(api: &str) -> Arc<connect_connectors::Vapi> {
    Arc::new(
        connect_connectors::Vapi::with_reference("isolated-fixture-key", api, "OFFLINE_FIXTURE")
            .unwrap(),
    )
}
#[ignore = "child-only entry; parent requires actual provider POST and SIGKILL"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_creation_child() {
    let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
    let algorithm = std::env::var(ALGORITHM).unwrap();
    let (f, policy) = setup_for(&algorithm).await;
    assert!(f.config.database.starts_with(&root));
    let worker = Worker::with_routes(
        &f.server.service,
        vec![ProviderVoiceRoute {
            policy: policy.clone(),
            driver: driver(&std::env::var(API).unwrap()),
            automatic_outbound: true,
        }],
    )
    .unwrap();
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let (invitation, _) = invite(&f, &mut client).await;
    let binding = f
        .server
        .store
        .provider_session_voice_binding(&policy)
        .unwrap();
    let caller = f.server.store.authenticate(&f.client.credential).unwrap();
    let selected = f
        .server
        .store
        .provider_session_voice_invited_target(
            &caller.credential_id,
            &invitation,
            &binding.0,
            &binding.1,
        )
        .unwrap();
    assert!(
        selected.is_some(),
        "exact completed invitation did not select configured provider target"
    );
    let receipt =
        serde_json::to_value(client.session_command(invitation.clone()).await.unwrap()).unwrap();
    let mut config = f.config.clone();
    config.bind = f.server.address;
    let prepared = Prepared {
        config,
        client: f.client.clone(),
        policy,
        epoch: f.server.store.runtime_epoch().into(),
        installation: f.server.store.installation_id().unwrap(),
        invitation,
        receipt,
    };
    std::fs::write(
        root.join("prepared.pending"),
        serde_json::to_vec(&prepared).unwrap(),
    )
    .unwrap();
    std::fs::rename(root.join("prepared.pending"), root.join("prepared.json")).unwrap();
    std::future::pending::<()>().await;
    drop((client, worker, f));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sigkill_provider_create_reconciles_original_external_call_once_eddsa() {
    creation("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sigkill_provider_create_reconciles_original_external_call_once_es256() {
    creation("ES256").await;
}
async fn creation(algorithm: &str) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let scratch = workspace.join(".local/gate-6/recovery-process");
    std::fs::create_dir_all(&scratch).unwrap();
    let temp = Scratch(Some(tempfile::tempdir_in(scratch).unwrap()));
    let provider = MockServer::start().await;
    let original = Arc::new(std::sync::Mutex::new(None::<String>));
    let queries = Arc::new(AtomicUsize::new(0));
    let indexed = Arc::new(AtomicBool::new(false));
    let captured = original.clone();
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path("/call"))
        .respond_with(move |r: &wiremock::Request| {
            let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            assert_eq!(v["assistantId"], ASSISTANT);
            assert_eq!(v["phoneNumberId"], PHONE);
            assert_eq!(v["customer"]["number"], "+15555550111");
            let mut original = captured.lock().unwrap();
            assert!(original.is_none(), "provider create retried");
            *original = Some(v["name"].as_str().unwrap().into());
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(10))
                .set_body_json(json!({"id":REMOTE,"status":"queued"}))
        })
        .mount(&provider)
        .await;
    let make_row = {
        let original = original.clone();
        move || json!({"id":REMOTE,"name":original.lock().unwrap().clone().unwrap(),"type":"outboundPhoneCall","assistantId":ASSISTANT,"phoneNumberId":PHONE,"customer":{"number":"+15555550111"},"createdAt":chrono::Utc::now().to_rfc3339(),"status":"ended","endedReason":"customer-ended-call"})
    };
    let listed = indexed.clone();
    let row = make_row.clone();
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path("/call"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(if listed.load(Ordering::SeqCst) {
                vec![row()]
            } else {
                vec![]
            })
        })
        .mount(&provider)
        .await;
    let queried = queries.clone();
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path(format!("/call/{REMOTE}")))
        .respond_with(move |_: &wiremock::Request| {
            queried.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(make_row())
        })
        .mount(&provider)
        .await;
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
            .env(API, provider.uri())
            .env(ALGORITHM, algorithm)
            .env("TMPDIR", temp.path())
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output))
            .spawn()
            .unwrap(),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while original.lock().unwrap().is_none() || !temp.path().join("prepared.json").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child failed: {}",
            log.display()
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "child submission timed out: {}",
            log.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let prepared: Prepared =
        serde_json::from_slice(&std::fs::read(temp.path().join("prepared.json")).unwrap()).unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &prepared.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let original_effect:(String,String,u64)=db.query_row("SELECT p.id,e.id,e.attempt FROM provider_session_voice p JOIN operation_effects e ON e.id=p.create_effect_id WHERE p.provider_id IS NULL AND e.state='submitting'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(original_effect.2, 1);
    child.0.kill().unwrap();
    let death = child.0.wait().unwrap();
    assert_eq!(death.signal(), Some(9));
    indexed.store(true, Ordering::SeqCst);
    let server = RunningCanonicalServer::start(prepared.config.clone())
        .await
        .unwrap();
    assert_ne!(server.store.runtime_epoch(), prepared.epoch);
    assert_eq!(
        server.store.installation_id().unwrap(),
        prepared.installation
    );
    let worker = Worker::with_routes(
        &server.service,
        vec![ProviderVoiceRoute {
            policy: prepared.policy,
            driver: driver(&provider.uri()),
            automatic_outbound: true,
        }],
    )
    .unwrap();
    let mut config = prepared.client;
    config.endpoint = server.address;
    let mut client = CanonicalClient::open(config).await.unwrap();
    let ended = view(
        &mut client,
        &prepared.invitation.conversation_id,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    assert_eq!(ended.id, original_effect.0);
    assert!(ended.closed_at.is_some());
    assert_eq!(ended.operation_id, prepared.invitation.operation_id);
    let receipt = client
        .session_command(prepared.invitation.clone())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(receipt).unwrap(), prepared.receipt);
    assert!(
        queries.load(Ordering::SeqCst) > 0,
        "successor did not query the original provider resource"
    );
    assert_eq!(
        db.query_row(
            "SELECT attempt FROM operation_effects WHERE id=?1",
            [&original_effect.1],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM provider_session_voice", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM connections", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let settled: (String, bool, bool) = db.query_row(
        "SELECT e.state,p.provider_id=?2,p.closed_at IS NOT NULL FROM provider_session_voice p JOIN operation_effects e ON e.id=p.create_effect_id WHERE p.id=?1",
        [&original_effect.0, REMOTE], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!(settled, ("confirmed".into(), true, true));
    worker.shutdown().await.unwrap();
    client.close();
    server.shutdown().await.unwrap();
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .count(),
        1
    );
    assert!(requests
        .iter()
        .all(|r| r.url.path() == "/call" || r.url.path() == format!("/call/{REMOTE}")));
    if let Some(prefix) = std::env::var_os("CONNECT_G6_PROVIDER_CRASH_CAPTURE") {
        let prefix = std::path::PathBuf::from(prefix);
        assert!(prefix.is_absolute() && prefix.starts_with(workspace.join(".local/gate-6")));
        std::fs::write(prefix.with_extension(format!("{algorithm}.json")),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"phase":"submitted","killed_signal":death.signal(),"external_http_provider_survived":true,"actual_vapi_sdk":true,"create_posts":1,"original_effect_attempt":1,"original_call_reconciled_by_authenticated_list_and_get":true,"status_gets":queries.load(Ordering::SeqCst),"public_state":"ended","local_media":false,"session_started_facts":0,"connections":0,"replay_preserved_original_receipt":true,"worker_and_server_drained":true})).unwrap()).unwrap();
    }
}
