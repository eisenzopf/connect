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
    language: &'static str,
    helpers: usize,
}
static PYTHON_CONSULT_HELPERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static PYTHON_CONSULT_WORKERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
fn python_consult() -> bool {
    std::env::var("CONNECT_GATE6_PYTHON_CONSULT").as_deref() == Ok("1")
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
            let prefix = if self.language == "python"
                && matches!(self.mode.as_str(), "consult" | "return")
            {
                "installed-python-consultation-failure"
            } else if self.language == "python" {
                "installed-python-movement-failure"
            } else if matches!(self.mode.as_str(), "handoff" | "move" | "move-recovery") {
                "installed-node-movement-failure"
            } else {
                "installed-node-consultation-failure"
            };
            eprintln!("{prefix} {value}");
        }
        assert_eq!(value["phase"], expected);
        let prefix =
            if self.language == "python" && matches!(self.mode.as_str(), "consult" | "return") {
                "installed-python-consultation-phase"
            } else if self.language == "python" {
                "installed-python-movement-phase"
            } else if matches!(self.mode.as_str(), "handoff" | "move" | "move-recovery") {
                "installed-node-movement-phase"
            } else {
                "installed-node-consultation-phase"
            };
        eprintln!(
            "{prefix} {}",
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
    let config = controller_config(f, token, if mode == "consult" { 901 } else { 902 });
    start_configured(
        f,
        config,
        mode,
        detail,
        if python_consult() {
            "CONNECT_GATE6_PYTHON_CONSULT_CONSUMER"
        } else {
            "CONNECT_GATE6_NODE_CONSULT_CONSUMER"
        },
        if mode == "consult" {
            "submitted"
        } else {
            "returned"
        },
    )
    .await
}
async fn start_configured(
    f: &Fixture,
    config: CanonicalClientConfig,
    mode: &str,
    detail: Value,
    consumer_env: &str,
    initial: &str,
) -> (Process, Value) {
    let python = matches!(
        consumer_env,
        "CONNECT_GATE6_PYTHON_MOVEMENT_CONSUMER" | "CONNECT_GATE6_PYTHON_CONSULT_CONSUMER"
    );
    let token = config.credential.clone();
    let script = PathBuf::from(std::env::var_os(consumer_env).unwrap())
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
            "mode":mode,"detail":detail,"http":format!("http://{}",f.server.http_address.unwrap()),"credential":token
        }))
        .unwrap(),
    )
    .unwrap();
    let stderr = project.join(format!(
        "{}-consult-{mode}-{}-{}.stderr",
        if python { "python" } else { "node" },
        f.client.signing_algorithm,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    init::private_write(&stderr, b"").unwrap();
    let mut command = if python {
        let interpreter = PathBuf::from(std::env::var_os("CONNECT_GATE6_PYTHON").unwrap());
        assert!(interpreter.is_absolute() && interpreter.is_file());
        let bin = f.temp.path().join(format!("python-{mode}-bin"));
        std::fs::create_dir_all(&bin).unwrap();
        if !bin.join("connect").exists() {
            std::os::unix::fs::symlink(&wrapper, bin.join("connect")).unwrap();
        }
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let mut command = tokio::process::Command::new(interpreter);
        command
            .arg("-I")
            .env("PATH", std::env::join_paths(paths).unwrap())
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
        language: if python { "python" } else { "node" },
        helpers: 1,
    };
    let value = process.phase(initial).await;
    assert!(!std::fs::read_to_string(stderr).unwrap().contains(&token));
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
                "SELECT COUNT(*) FROM operations WHERE operation_id=?1",
                [if process.language == "python" {
                    "python-return-without-grant"
                } else {
                    "node-return-without-grant"
                }],
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
        assert_eq!(value["http_current_authority_refused"], true);
    }
    pub(super) async fn close(self) {
        let mut process = self.process.into_inner();
        let value = process.request(json!({"action":"close"}), "closed").await;
        assert_eq!(value["mode"], process.mode);
        assert_eq!(value["helpers_joined"], process.helpers);
        if process.language == "python" {
            assert_eq!(value["http_workers_joined"], 1);
            PYTHON_CONSULT_HELPERS.fetch_add(process.helpers, std::sync::atomic::Ordering::SeqCst);
            PYTHON_CONSULT_WORKERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
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

static MOVEMENT_HELPERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static MOVEMENT_SERVERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(in crate::canonical_native_connections::consultation) struct MovementProbe {
    process: Process,
    pub reference: Value,
}
impl MovementProbe {
    pub(in crate::canonical_native_connections::consultation) fn joined() -> (usize, usize) {
        use std::sync::atomic::Ordering;
        (
            MOVEMENT_HELPERS.load(Ordering::SeqCst),
            MOVEMENT_SERVERS.load(Ordering::SeqCst),
        )
    }
    pub(in crate::canonical_native_connections::consultation) fn server_joined() {
        MOVEMENT_SERVERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    pub(in crate::canonical_native_connections::consultation) fn enabled() -> bool {
        let node = std::env::var_os("CONNECT_GATE6_NODE_MOVEMENTS").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        let python = std::env::var_os("CONNECT_GATE6_PYTHON_MOVEMENTS").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        assert!(
            !(node && python),
            "Select exactly one installed SDK movement lane"
        );
        node || python
    }
    pub(in crate::canonical_native_connections::consultation) fn python() -> bool {
        Self::enabled()
            && std::env::var_os("CONNECT_GATE6_PYTHON_MOVEMENTS").as_deref()
                == Some(std::ffi::OsStr::new("1"))
    }
    fn consumer_env() -> &'static str {
        if Self::python() {
            "CONNECT_GATE6_PYTHON_MOVEMENT_CONSUMER"
        } else {
            "CONNECT_GATE6_NODE_MOVEMENT_CONSUMER"
        }
    }
    pub(in crate::canonical_native_connections::consultation) async fn register(
        f: &Fixture,
        config: CanonicalClientConfig,
        command: &TransitionCommand,
        consultation: Value,
    ) -> Self {
        assert!(Self::enabled());
        let mode = if matches!(&command.intent, TransitionIntent::Handoff { .. }) {
            "handoff"
        } else {
            "move"
        };
        let (process, _) = start_configured(
            f,
            config,
            mode,
            json!({"command":command,"consultation":consultation}),
            Self::consumer_env(),
            "registered",
        )
        .await;
        Self {
            process,
            reference: Value::Null,
        }
    }
    pub(in crate::canonical_native_connections::consultation) async fn begin(&mut self) {
        let value = self
            .process
            .request(json!({"action":"begin"}), "accepted")
            .await;
        self.reference = value["reference"].clone();
    }
    pub(in crate::canonical_native_connections::consultation) async fn completed(
        mut self,
    ) -> Value {
        let value = self
            .process
            .request(json!({"action":"completed"}), "completed")
            .await;
        self.process.helpers = 2;
        self.close().await;
        value["summary"].clone()
    }
    pub(in crate::canonical_native_connections::consultation) async fn stopped(
        mut self,
        state: connect_protocol::transitions::OperationState,
        original_revoked: bool,
    ) -> Value {
        let value = self
            .process
            .request(
                json!({"action":"stopped","state":state,"original_revoked":original_revoked}),
                "stopped",
            )
            .await;
        self.close().await;
        value
    }
    pub(in crate::canonical_native_connections::consultation) async fn recover_stopped(
        f: &Fixture,
        config: CanonicalClientConfig,
        reference: Value,
        state: connect_protocol::transitions::OperationState,
    ) {
        let (process, value) = start_configured(
            f,
            config,
            "move-recovery",
            json!({"reference":reference,"state":state}),
            Self::consumer_env(),
            "stopped",
        )
        .await;
        assert_eq!(value["state"], serde_json::to_value(state).unwrap());
        assert_eq!(value["certainty"], "recorded");
        Self {
            process,
            reference: Value::Null,
        }
        .close()
        .await;
    }
    async fn close(mut self) {
        let value = self
            .process
            .request(json!({"action":"close"}), "closed")
            .await;
        assert_eq!(value["helpers_joined"], self.process.helpers);
        drop(self.process.input);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), self.process.output.next_line())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), self.process.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        MOVEMENT_HELPERS.fetch_add(self.process.helpers, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tokio::test]
#[ignore = "Requires installed Node consultation consumer under .local"]
async fn installed_node_consultation_and_return_preserve_actual_private_and_restored_audio() {
    assert!(!python_consult());
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
    assert!(!python_consult());
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

#[tokio::test]
#[ignore = "Requires installed Python consultation consumer and explicit CONNECT_GATE6_PYTHON_CONSULT=1"]
async fn installed_python_consultation_and_return_preserve_actual_private_and_restored_audio() {
    assert!(python_consult());
    use std::sync::atomic::Ordering;
    for algorithm in ["EdDSA", "ES256"] {
        let before = (
            PYTHON_CONSULT_HELPERS.load(Ordering::SeqCst),
            PYTHON_CONSULT_WORKERS.load(Ordering::SeqCst),
        );
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
        let helpers = PYTHON_CONSULT_HELPERS.load(Ordering::SeqCst) - before.0;
        let workers = PYTHON_CONSULT_WORKERS.load(Ordering::SeqCst) - before.1;
        assert_eq!((helpers, workers), (3, 2));
        eprintln!(
            "installed-python-consultation-witness {}",
            json!({
                "profile":"python-consultation/1","algorithm":algorithm,"audio_case":"restored",
                "current_return_authority":true,"original_record_recovered":true,"helpers_joined":helpers,
                "http_workers_joined":workers,"server_joined":true,"physical_peers_drained":true,"media_readers_drained":true
            })
        );
    }
}

#[tokio::test]
#[ignore = "Requires installed Python consultation consumer and explicit CONNECT_GATE6_PYTHON_CONSULT=1"]
async fn installed_python_consultation_return_after_remote_eof_cannot_claim_a_continuing_call() {
    assert!(python_consult());
    use std::sync::atomic::Ordering;
    for algorithm in ["EdDSA", "ES256"] {
        let before = (
            PYTHON_CONSULT_HELPERS.load(Ordering::SeqCst),
            PYTHON_CONSULT_WORKERS.load(Ordering::SeqCst),
        );
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
        let helpers = PYTHON_CONSULT_HELPERS.load(Ordering::SeqCst) - before.0;
        let workers = PYTHON_CONSULT_WORKERS.load(Ordering::SeqCst) - before.1;
        assert_eq!((helpers, workers), (2, 2));
        eprintln!(
            "installed-python-consultation-witness {}",
            json!({
                "profile":"python-consultation/1","algorithm":algorithm,"audio_case":"remote-ended",
                "current_return_authority":true,"original_record_recovered":true,"helpers_joined":helpers,
                "http_workers_joined":workers,"server_joined":true,"physical_peers_drained":true,"media_readers_drained":true
            })
        );
    }
}
