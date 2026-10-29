//! Installed public Node/Python administrator; independent Rust host comparisons.
use super::*;
use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
static HELPERS: AtomicUsize = AtomicUsize::new(0);
static VALIDATORS: AtomicUsize = AtomicUsize::new(0);
static SERVERS: AtomicUsize = AtomicUsize::new(0);
static INDEX: AtomicUsize = AtomicUsize::new(0);
pub(super) fn enabled() -> bool {
    let node = std::env::var("CONNECT_GATE6_NODE_ADMIN").as_deref() == Ok("1");
    assert!(
        !(node && python()),
        "Select one installed administrative language"
    );
    node || python()
}
pub(super) fn python() -> bool {
    std::env::var("CONNECT_GATE6_PYTHON_ADMIN").as_deref() == Ok("1")
}
fn language() -> &'static str {
    if python() {
        "python"
    } else {
        "node"
    }
}
fn profile() -> &'static str {
    if python() {
        "python-administration/1"
    } else {
        "node-administration/3"
    }
}
struct Process {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    algorithm: String,
}
impl Process {
    async fn phase(&mut self, expected: &str) -> Value {
        let started = tokio::time::Instant::now();
        let line = tokio::time::timeout(Duration::from_secs(15), self.output.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("Node administrative phase");
        assert!(line.len() <= 128 * 1024);
        let value: Value = serde_json::from_str(&line).unwrap();
        if value["phase"] == "failed" {
            eprintln!("installed-{}-administrative-failure {value}", language());
        }
        assert_eq!(value["phase"], expected);
        eprintln!(
            "installed-{}-administrative-phase {}",
            language(),
            json!({"algorithm":self.algorithm,"phase":expected,"seconds":started.elapsed().as_secs_f64(),"deadline_seconds":15})
        );
        value
    }
    async fn request(&mut self, value: Value, expected: &str) -> Value {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        assert!(bytes.len() < 128 * 1024);
        bytes.push(b'\n');
        self.input.write_all(&bytes).await.unwrap();
        self.phase(expected).await
    }
}
pub(super) struct Probe {
    process: tokio::sync::Mutex<Option<Process>>,
    records: std::sync::Mutex<BTreeMap<String, String>>,
}
impl Probe {
    pub(super) async fn start(f: &Fixture, config: CanonicalClientConfig) -> Self {
        let application = super::app(f, &config);
        Self::start_with_application(f, config, application).await
    }
    async fn start_with_application(
        f: &Fixture,
        config: CanonicalClientConfig,
        application: ApplicationClientConfig,
    ) -> Self {
        let index = INDEX.fetch_add(1, Ordering::SeqCst);
        let consumer = if python() {
            "CONNECT_GATE6_PYTHON_ADMIN_CONSUMER"
        } else {
            "CONNECT_GATE6_NODE_ADMIN_CONSUMER"
        };
        let script = PathBuf::from(std::env::var_os(consumer).unwrap())
            .canonicalize()
            .unwrap();
        assert!(script.starts_with(
            std::env::current_dir()
                .unwrap()
                .join(".local")
                .canonicalize()
                .unwrap()
        ));
        let project = script.parent().unwrap();
        let sdk_config = sdk::ClientConfig {
            control: config.clone(),
            tenant_id: "tnt_default".parse().unwrap(),
            application: Some(application),
        };
        let native = f.temp.path().join(format!("node-admin-{index}.toml"));
        init::private_write(&native, toml::to_string(&sdk_config).unwrap().as_bytes()).unwrap();
        let pid_file = f.temp.path().join(format!("node-admin-{index}.pids"));
        let wrapper = f.temp.path().join(format!("node-admin-{index}-connect"));
        init::private_write(&wrapper,br#"#!/usr/bin/env python3
import os,sys
fd=os.open(os.path.join(os.environ['CONNECT_G6_ADMIN_STDERR_DIR'],'helper-'+str(os.getpid())+'.stderr'),os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
os.dup2(fd,2)
os.close(fd)
path=os.environ['CONNECT_G6_NODE_PID_FILE']+('.validators' if sys.argv[1:2]==['sdk-validate-provisioning'] else '')
with open(path,'a') as stream:stream.write(str(os.getpid())+'\n')
os.execv(os.environ['CONNECT_G6_NODE_CONNECT'],[os.environ['CONNECT_G6_NODE_CONNECT'],*sys.argv[1:]])
"#).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let input = f.temp.path().join(format!("node-admin-{index}.json"));
        init::private_write(&input,&serde_json::to_vec(&json!({"project":project,"config":native,"executable":wrapper,"pid_file":pid_file})).unwrap()).unwrap();
        let trace = project.join(format!(
            "{}-admin-{}-{index}-{}",
            language(),
            config.signing_algorithm,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&trace).unwrap();
        std::fs::set_permissions(&trace, std::fs::Permissions::from_mode(0o700)).unwrap();
        let stderr = trace.join("consumer.stderr");
        let mut command = if python() {
            let interpreter = PathBuf::from(std::env::var_os("CONNECT_GATE6_PYTHON").unwrap());
            assert!(interpreter.is_absolute() && interpreter.is_file());
            let bin = f.temp.path().join(format!("python-admin-{index}-bin"));
            std::fs::create_dir(&bin).unwrap();
            std::os::unix::fs::symlink(&wrapper, bin.join("connect")).unwrap();
            let mut path = vec![bin];
            path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
            let mut command = tokio::process::Command::new(interpreter);
            command
                .arg("-I")
                .env("PATH", std::env::join_paths(path).unwrap())
                .env_remove("PYTHONPATH")
                .env_remove("PYTHONHOME")
                .env_remove("PYTHONUSERBASE");
            command
        } else {
            tokio::process::Command::new("node")
        };
        let mut child = command
            .arg(&script)
            .arg(input)
            .current_dir(project)
            .env("CONNECT_G6_NODE_PID_FILE", pid_file)
            .env("CONNECT_G6_NODE_CONNECT", env!("CARGO_BIN_EXE_connect"))
            .env("CONNECT_G6_ADMIN_STDERR_DIR", &trace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(std::fs::File::create(stderr).unwrap()))
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut process = Process {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            algorithm: config.signing_algorithm,
        };
        process.phase("ready").await;
        Self {
            process: tokio::sync::Mutex::new(Some(process)),
            records: Default::default(),
        }
    }
    pub(super) async fn apply(&self, command: Value) -> Value {
        let id = command["operation_id"].as_str().unwrap().to_owned();
        let value = self
            .process
            .lock()
            .await
            .as_mut()
            .unwrap()
            .request(json!({"action":"apply","command":command}), "applied")
            .await;
        let record = value["record_id"].as_str().unwrap().to_owned();
        if let Some(original) = self.records.lock().unwrap().insert(id, record.clone()) {
            assert_eq!(original, record);
        }
        value
    }
    pub(super) fn record(&self, id: &str) -> String {
        self.records.lock().unwrap().get(id).unwrap().clone()
    }
    pub(super) async fn read(&self, reference: Value, expected: Option<String>) -> Value {
        self.process
            .lock()
            .await
            .as_mut()
            .unwrap()
            .request(
                json!({"action":"read","reference":reference,"expected_record":expected}),
                "read",
            )
            .await
    }
    async fn begin_held(&self, command: Value, queued: Value) -> Value {
        self.process
            .lock()
            .await
            .as_mut()
            .unwrap()
            .request(
                json!({"action":"held","command":command,"queued":queued}),
                "submitted",
            )
            .await
    }
    async fn cancel_held(&self) -> Value {
        self.process
            .lock()
            .await
            .as_mut()
            .unwrap()
            .request(json!({"action":"cancel-held"}), "uncertain")
            .await
    }
    pub(super) async fn read_refused(&self, reference: Value, revoked: bool) {
        self.process
            .lock()
            .await
            .as_mut()
            .unwrap()
            .request(
                json!({"action":"read-refused","reference":reference,"revoked":revoked}),
                "read-refused",
            )
            .await;
    }
    pub(super) async fn refuse(&self, command: Value, kind: &str, certainty: &str) {
        self.process
            .lock()
            .await
            .as_mut()
            .unwrap()
            .request(
                json!({"action":"refuse","command":command,"kind":kind,"certainty":certainty}),
                "refused",
            )
            .await;
    }
    pub(super) async fn close(&self) {
        let Some(mut process) = self.process.lock().await.take() else {
            return;
        };
        let value = process.request(json!({"action":"close"}), "closed").await;
        assert_eq!(value["helpers_joined"], 1);
        let validators = value["validators_joined"].as_u64().unwrap() as usize;
        drop(process.input);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), process.output.next_line())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), process.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        HELPERS.fetch_add(1, Ordering::SeqCst);
        VALIDATORS.fetch_add(validators, Ordering::SeqCst);
    }
}
pub(super) async fn drained(f: &Fixture) {
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = f.server.service.canonical_runtime_snapshot().unwrap();
            if view.physical_peers == 0 && view.media_readers == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if drained.is_err() {
        let view = f.server.service.canonical_runtime_snapshot().unwrap();
        eprintln!(
            "installed-{}-administrative-drain-failure {}",
            language(),
            json!({
                "physical_peers":view.physical_peers,"media_readers":view.media_readers,
                "helpers_joined":HELPERS.load(Ordering::SeqCst),"validators_joined":VALIDATORS.load(Ordering::SeqCst)
            })
        );
    }
    drained.unwrap();
}
pub(super) fn server_joined() {
    if enabled() {
        SERVERS.fetch_add(1, Ordering::SeqCst);
    }
}
pub(super) fn witness(algorithm: &str) {
    assert_eq!(HELPERS.load(Ordering::SeqCst), 2);
    assert_eq!(SERVERS.load(Ordering::SeqCst), 2);
    assert_eq!(VALIDATORS.load(Ordering::SeqCst), 26);
    eprintln!(
        "installed-{}-administrative-witness {}",
        language(),
        json!({"profile":profile(),"scenario":"commands","algorithm":algorithm,"command_families":11,"helpers_joined":2,"validators_joined":26,"servers_joined":2,"original_record_recovered":true,"one_original_receipt":true,"changed_intent_refused":true,"physical_peers_drained":true,"media_readers_drained":true})
    );
}

pub(super) fn authority_witness(algorithm: &str) {
    assert_eq!(HELPERS.load(Ordering::SeqCst), 4);
    assert_eq!(SERVERS.load(Ordering::SeqCst), 1);
    assert_eq!(VALIDATORS.load(Ordering::SeqCst), 10);
    eprintln!(
        "installed-{}-administrative-witness {}",
        language(),
        json!({
            "profile":profile(),"scenario":"authority","algorithm":algorithm,
            "helpers_joined":4,"validators_joined":10,"servers_joined":1,
            "ordinary_submit_read_refused":true,"revoked_current_read_refused":true,
            "revoked_startup_replay_and_submit_not_submitted":true,
            "independent_authorized_original_record_recovered":true,"no_forbidden_or_revoked_receipts":true,
            "physical_peers_drained":true,"media_readers_drained":true
        })
    );
}

pub(super) async fn uncertainty(
    f: Fixture,
    config: CanonicalClientConfig,
    endpoint: std::net::SocketAddr,
    ready: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
    proxy: tokio::task::JoinHandle<()>,
) {
    let mut application = super::app(&f, &config);
    application.endpoint = endpoint;
    let node = Probe::start_with_application(&f, config.clone(), application).await;
    let submitted = node
        .begin_held(
            serde_json::to_value(super::identity(
                "sdk-admin-uncertain",
                "sdk-admin-uncertain-target",
            ))
            .unwrap(),
            serde_json::to_value(super::identity(
                "sdk-admin-never-written",
                "sdk-admin-never-written-target",
            ))
            .unwrap(),
        )
        .await;
    assert_eq!(submitted["reference"], super::saved("sdk-admin-uncertain"));
    // The original host must actually commit before either local wait is cancelled.
    tokio::time::timeout(Duration::from_secs(5), ready.notified())
        .await
        .unwrap();
    let cancelled = node.cancel_held().await;
    assert_eq!(cancelled["reference"], submitted["reference"]);
    assert_eq!(cancelled["original_id"], "sdk-admin-uncertain");
    assert_eq!(cancelled["original_certainty"], "uncertain");
    assert_eq!(cancelled["queued_id"], "sdk-admin-never-written");
    assert_eq!(cancelled["queued_certainty"], "not-submitted");
    node.close().await;
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), proxy)
        .await
        .unwrap()
        .unwrap();
    let recovered = super::connect(&f, config).await;
    let reference = super::saved("sdk-admin-uncertain");
    let observed = recovered
        .restore(reference.clone())
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    assert_eq!(observed.value["alias"], "sdk-admin-uncertain-target");
    let read = recovered
        .node
        .as_ref()
        .unwrap()
        .read(
            reference,
            Some(
                observed
                    .snapshot
                    .record_id
                    .as_ref()
                    .unwrap()
                    .as_str()
                    .to_owned(),
            ),
        )
        .await;
    assert_eq!(read["value"], observed.value);
    recovered
        .node
        .as_ref()
        .unwrap()
        .read_refused(super::saved("sdk-admin-never-written"), false)
        .await;
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let original: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-admin-uncertain'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let queued: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-admin-never-written'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, 1);
    assert_eq!(queued, 0);
    let original_effect: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM identities WHERE alias='sdk-admin-uncertain-target'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let queued_effect: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM identities WHERE alias='sdk-admin-never-written-target'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original_effect, 1);
    assert_eq!(queued_effect, 0);
    drop(db);
    recovered.close(options()).await.unwrap();
    drained(&f).await;
    f.server.shutdown().await.unwrap();
    server_joined();
    assert_eq!(HELPERS.load(Ordering::SeqCst), 2);
    assert_eq!(VALIDATORS.load(Ordering::SeqCst), 4);
    assert_eq!(SERVERS.load(Ordering::SeqCst), 1);
}
pub(super) fn uncertainty_witness(algorithm: &str) {
    eprintln!(
        "installed-{}-administrative-witness {}",
        language(),
        json!({
            "profile":profile(),"scenario":"uncertainty","algorithm":algorithm,
            "helpers_joined":2,"validators_joined":4,"servers_joined":1,
            "actual_commit_held_response":true,"original_cancelled_uncertain":true,"queued_cancelled_not_submitted":true,
            "fresh_original_record_recovered_without_replay":true,"one_original_receipt":true,"zero_queued_receipts":true,"no_hidden_resubmission":true,
            "physical_peers_drained":true,"media_readers_drained":true
        })
    );
}
