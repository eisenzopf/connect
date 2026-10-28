//! Installed public Node administrator; host comparisons use independent Rust reads.
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
    std::env::var("CONNECT_GATE6_NODE_ADMIN").as_deref() == Ok("1")
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
            eprintln!("installed-node-administrative-failure {value}");
        }
        assert_eq!(value["phase"], expected);
        eprintln!(
            "installed-node-administrative-phase {}",
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
        let index = INDEX.fetch_add(1, Ordering::SeqCst);
        let script = PathBuf::from(std::env::var_os("CONNECT_GATE6_NODE_ADMIN_CONSUMER").unwrap())
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
            application: Some(super::app(f, &config)),
        };
        let native = f.temp.path().join(format!("node-admin-{index}.toml"));
        init::private_write(&native, toml::to_string(&sdk_config).unwrap().as_bytes()).unwrap();
        let pid_file = f.temp.path().join(format!("node-admin-{index}.pids"));
        let wrapper = f.temp.path().join(format!("node-admin-{index}-connect"));
        init::private_write(&wrapper,br#"#!/usr/bin/env python3
import os,sys
path=os.environ['CONNECT_G6_NODE_PID_FILE']+('.validators' if sys.argv[1:2]==['sdk-validate-provisioning'] else '')
with open(path,'a') as stream:stream.write(str(os.getpid())+'\n')
os.execv(os.environ['CONNECT_G6_NODE_CONNECT'],[os.environ['CONNECT_G6_NODE_CONNECT'],*sys.argv[1:]])
"#).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let input = f.temp.path().join(format!("node-admin-{index}.json"));
        init::private_write(&input,&serde_json::to_vec(&json!({"project":project,"config":native,"executable":wrapper,"pid_file":pid_file})).unwrap()).unwrap();
        let stderr = f.temp.path().join(format!("node-admin-{index}.stderr"));
        let mut child = tokio::process::Command::new("node")
            .arg(&script)
            .arg(input)
            .current_dir(project)
            .env("CONNECT_G6_NODE_PID_FILE", pid_file)
            .env("CONNECT_G6_NODE_CONNECT", env!("CARGO_BIN_EXE_connect"))
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
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = f.server.service.canonical_runtime_snapshot().unwrap();
            if view.physical_peers == 0 && view.media_readers == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
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
        "installed-node-administrative-witness {}",
        json!({"profile":"node-administration/1","algorithm":algorithm,"command_families":11,"helpers_joined":2,"validators_joined":26,"servers_joined":2,"original_record_recovered":true,"one_original_receipt":true,"changed_intent_refused":true,"physical_peers_drained":true,"media_readers_drained":true})
    );
}
