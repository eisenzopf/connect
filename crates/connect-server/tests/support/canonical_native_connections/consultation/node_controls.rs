//! Installed public Node controls; physical audio remains with the fixture peers.
use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, process::Stdio};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

struct Process {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    algorithm: String,
    mode: String,
    helpers: usize,
}
impl Process {
    async fn phase(&mut self, expected: &str) -> Value {
        let started = tokio::time::Instant::now();
        let line = tokio::time::timeout(Duration::from_secs(15), self.output.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("Node consultation phase");
        assert!(line.len() <= 8192);
        let value: Value = serde_json::from_str(&line).unwrap();
        if value["phase"] == "failed" {
            eprintln!("installed-node-consultation-failure {value}");
        }
        assert_eq!(value["phase"], expected);
        eprintln!(
            "installed-node-consultation-phase {}",
            json!({
                "algorithm": self.algorithm, "phase": expected,
                "seconds": started.elapsed().as_secs_f64(), "deadline_seconds": 15
            })
        );
        value
    }
    async fn request(&mut self, request: Value, expected: &str) -> Value {
        let mut bytes = serde_json::to_vec(&request).unwrap();
        bytes.push(b'\n');
        self.input.write_all(&bytes).await.unwrap();
        self.phase(expected).await
    }
}
pub(in crate::canonical_native_connections::consultation) struct Probe {
    pub reference: Value,
    process: tokio::sync::Mutex<Process>,
}
async fn start(f: &Fixture, token: &str, mode: &str, detail: Value) -> (Process, Value) {
    let script = PathBuf::from(std::env::var_os("CONNECT_GATE6_NODE_CONSULT_CONSUMER").unwrap())
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
    let config = controller_config(f, token, if mode == "consult" { 901 } else { 902 });
    let sdk_config = sdk::ClientConfig {
        control: config,
        tenant_id: "tnt_default".parse().unwrap(),
        application: None,
    };
    let native = f.temp.path().join(format!("node-consult-{mode}.toml"));
    init::private_write(&native, toml::to_string(&sdk_config).unwrap().as_bytes()).unwrap();
    let pid_file = f.temp.path().join(format!("node-consult-{mode}.pids"));
    let wrapper = f.temp.path().join(format!("node-consult-{mode}-connect"));
    init::private_write(&wrapper, br#"#!/usr/bin/env python3
import os,sys
with open(os.environ['CONNECT_G6_NODE_PID_FILE'],'a') as stream: stream.write(str(os.getpid())+'\n')
os.execv(os.environ['CONNECT_G6_NODE_CONNECT'],[os.environ['CONNECT_G6_NODE_CONNECT'],*sys.argv[1:]])
"#).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let input = f.temp.path().join(format!("node-consult-{mode}.json"));
    init::private_write(
        &input,
        &serde_json::to_vec(&json!({
            "project":project,"config":native,"executable":wrapper,"pid_file":pid_file,
            "mode":mode,"detail":detail
        }))
        .unwrap(),
    )
    .unwrap();
    let stderr = f.temp.path().join(format!("node-consult-{mode}.stderr"));
    init::private_write(&stderr, b"").unwrap();
    let mut child = tokio::process::Command::new("node")
        .arg(&script)
        .arg(input)
        .env("CONNECT_G6_NODE_PID_FILE", &pid_file)
        .env("CONNECT_G6_NODE_CONNECT", env!("CARGO_BIN_EXE_connect"))
        .current_dir(project)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(std::fs::File::create(&stderr).unwrap()))
        .spawn()
        .unwrap();
    let output = BufReader::new(child.stdout.take().unwrap()).lines();
    let input = child.stdin.take().unwrap();
    let mut process = Process {
        child,
        input,
        output,
        algorithm: f.client.signing_algorithm.clone(),
        mode: mode.into(),
        helpers: 1,
    };
    let value = process
        .phase(if mode == "consult" {
            "submitted"
        } else {
            "returned"
        })
        .await;
    assert!(!std::fs::read_to_string(stderr).unwrap().contains(token));
    (process, value)
}
pub(super) async fn submit(f: &Fixture, command: &TransitionCommand, token: &str) -> Probe {
    let TransitionIntent::Consult {
        retained_connid,
        private_participants,
        hold_treatment,
    } = &command.intent
    else {
        panic!("consult intent")
    };
    let (process, value) = start(f, token, "consult", json!({
        "cid":command.conversation_id,"sid":command.session_id,"via":command.connection_id,
        "retain":retained_connid,"participants":private_participants,
        "hold":if matches!(hold_treatment, HoldTreatment::Silence) {"silence"} else {"configured-media"},
        "revision":command.expected_revision,"operation_id":command.operation_id
    })).await;
    Probe {
        reference: value["reference"].clone(),
        process: tokio::sync::Mutex::new(process),
    }
}
impl Probe {
    pub(super) async fn completed(&self, f: &Fixture, revision: u64) {
        let mut process = self.process.lock().await;
        let value = process
            .request(
                json!({"action":"completed","revision":revision}),
                "completed",
            )
            .await;
        process.helpers = 2;
        assert_eq!(value["fresh_observation"], true);
        assert_eq!(value["return_without_grant_refused"], true);
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='node-return-without-grant'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0);
        let attempts: i64 = db
            .query_row(
                "SELECT SUM(attempt) FROM operation_effects WHERE operation_record_id=?1",
                [value["record_id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 1);
    }
    pub(super) async fn revoked(&self) {
        let value = self
            .process
            .lock()
            .await
            .request(json!({"action":"revoked"}), "revoked")
            .await;
        assert_eq!(value["current_authority_refused"], true);
    }
    pub(super) async fn close(self) {
        let mut process = self.process.into_inner();
        let value = process.request(json!({"action":"close"}), "closed").await;
        assert_eq!(value["mode"], process.mode);
        assert_eq!(value["helpers_joined"], process.helpers);
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
    }
}
pub(super) async fn restore(f: &Fixture, token: &str, reference: Value, revision: u64) -> Probe {
    let (process, value) = start(
        f,
        token,
        "return",
        json!({"reference":reference,"revision":revision}),
    )
    .await;
    assert_eq!(value["original_record_recovered"], true);
    Probe {
        reference: value["reference"].clone(),
        process: tokio::sync::Mutex::new(process),
    }
}

#[tokio::test]
#[ignore = "Requires installed Node consultation consumer under .local"]
async fn installed_node_consultation_and_return_preserve_actual_private_and_restored_audio() {
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
            Some(Auxiliary::Node(algorithm)),
        )
        .await;
        eprintln!(
            "installed-node-consultation-witness {}",
            json!({"algorithm":algorithm,"audio_case":"restored","current_return_authority":true,"original_record_recovered":true,"helpers_joined":3,"server_joined":true,"physical_peers_drained":true,"media_readers_drained":true})
        );
    }
}

#[tokio::test]
#[ignore = "Requires installed Node consultation consumer under .local"]
async fn installed_node_consultation_return_after_remote_eof_cannot_claim_a_continuing_call() {
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
            Some(Auxiliary::Node(algorithm)),
        )
        .await;
        eprintln!(
            "installed-node-consultation-witness {}",
            json!({"algorithm":algorithm,"audio_case":"remote-ended","current_return_authority":true,"original_record_recovered":true,"helpers_joined":2,"server_joined":true,"physical_peers_drained":true,"media_readers_drained":true})
        );
    }
}
