//! Fresh server/app/participant processes, public CLI/SDK only; no Connect Store.
#![allow(clippy::unwrap_used, dead_code)]
use connect_client::{ClientConfig, Message, Operation, Outcome};
use connect_review_loop::connected_app::{AppStore, Statistics};
use std::{fs::File, path::PathBuf, process::Stdio, time::Duration};
#[path = "connected_app_process.rs"]
mod process;
use process::{signal_stop, spawn_server};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub directory: PathBuf,
    server: Child,
    pub binary: PathBuf,
    app_log: PathBuf,
    pub mcp_address: std::net::SocketAddr,
    pub http_address: std::net::SocketAddr,
}
pub struct Worker {
    pub child: Child,
}
impl Fixture {
    pub async fn start() -> Self {
        let binary = std::env::var_os("CONNECT_TEST_SERVER_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/connect")
            });
        assert!(
            binary.is_file(),
            "Build connect-server --bin connect before running this fresh-process test"
        );
        let binary = binary.canonicalize().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("app");
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);
        let status = Command::new(&binary)
            .args(["init", "--example", "connected-app", "--directory"])
            .arg(&directory)
            .args(["--bind", &address.to_string()])
            .stdout(Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let mut config =
            connect_server::config::ServerConfig::load(&directory.join("server.toml")).unwrap();
        config.mcp = Some(connect_server::config::McpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            binding: None,
        });
        std::fs::write(
            directory.join("server.toml"),
            toml::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        let (server, mcp_address, http_address) = spawn_server(&binary, &directory).await;
        Self {
            _temp: temp,
            directory,
            binary,
            server,
            app_log: PathBuf::new(),
            mcp_address,
            http_address,
        }
    }
    pub fn state(&self) -> PathBuf {
        self.directory.join("business.sqlite3")
    }
    pub fn identity(&self, name: &str) -> String {
        ClientConfig::load(&self.directory.join(format!("{name}.toml")))
            .unwrap()
            .identity_id
    }
    pub fn spawn(&mut self, fault: Option<&str>) -> Worker {
        self.app_log = self
            .directory
            .join(format!("app-{}.log", uuid::Uuid::new_v4()));
        let file = File::create(&self.app_log).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_connect-example"));
        cmd.args(["connected-app", "--config"])
            .arg(self.directory.join("reviews.toml"))
            .arg("--policy")
            .arg(self.directory.join("connected-app.json"))
            .arg("--state")
            .arg(self.state())
            .env_remove("CONNECT_CONNECTED_APP_FAULT")
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .kill_on_drop(true);
        if let Some(fault) = fault {
            cmd.env("CONNECT_CONNECTED_APP_FAULT", fault);
        }
        Worker {
            child: cmd.spawn().unwrap(),
        }
    }
    pub async fn request(&self, name: &str, operation: Operation) -> Outcome {
        let mut cmd = Command::new(&self.binary);
        cmd.args(["request", "--config"])
            .arg(self.directory.join(format!("{name}.toml")));
        if let Operation::Task { request } = &operation {
            if let Some(task) = request.command.task_id() {
                cmd.args(["--task-id", task]);
            } else {
                cmd.args(["--task-address", "connect:reviews@local"]);
            }
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        input
            .write_all(&serde_json::to_vec(&operation).unwrap())
            .await
            .unwrap();
        drop(input);
        let output = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "CLI failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    pub async fn question(&self) -> Message {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Outcome::Inbox { events, .. } = self
                    .request(
                        "agent",
                        Operation::Inbox {
                            cursor: None,
                            limit: 100,
                            wait_ms: 0,
                        },
                    )
                    .await
                {
                    if let Some(message) = events
                        .into_iter()
                        .filter_map(|e| e.message)
                        .find(|m| m.body.contains("\"review_request\""))
                    {
                        return message;
                    }
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap()
    }
    pub async fn settled(&self, reviews: u64, tickets: u64) -> Statistics {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Ok(stats) = AppStore::inspect(&self.state()) {
                    if stats.reviews == reviews
                        && stats.tickets == tickets
                        && stats.pending_commands == 0
                        && stats.queued_events == 0
                    {
                        return stats;
                    }
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "App did not settle: {}",
                std::fs::read_to_string(&self.app_log).unwrap()
            )
        })
    }
    pub async fn history(&self, name: &str, cid: &str) -> Vec<Message> {
        let Outcome::History { messages, .. } = self
            .request(
                name,
                Operation::History {
                    conversation_id: cid.into(),
                    after: None,
                    limit: 100,
                },
            )
            .await
        else {
            panic!("history")
        };
        messages
    }
    pub async fn restart_server(&mut self) {
        signal_stop(&mut self.server).await;
        let (server, mcp, http) = spawn_server(&self.binary, &self.directory).await;
        self.server = server;
        self.mcp_address = mcp;
        self.http_address = http;
    }
    pub async fn revoke_app_credential(&mut self) {
        signal_stop(&mut self.server).await;
        let id = ClientConfig::load(&self.directory.join("reviews.toml"))
            .unwrap()
            .credential_id;
        let status = Command::new(&self.binary)
            .args(["revoke", "--config"])
            .arg(self.directory.join("operator.toml"))
            .arg("--server-config")
            .arg(self.directory.join("server.toml"))
            .arg(id)
            .stdout(Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let (server, mcp, http) = spawn_server(&self.binary, &self.directory).await;
        self.server = server;
        self.mcp_address = mcp;
        self.http_address = http;
    }
    pub async fn finish(&mut self, worker: &mut Worker) {
        signal_stop(&mut worker.child).await;
        signal_stop(&mut self.server).await;
    }
    pub fn business_payload(&self, kind: &str) -> serde_json::Value {
        let c = rusqlite::Connection::open_with_flags(
            self.state(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let s: String = c
            .query_row(
                "SELECT payload FROM app_business WHERE kind=?1",
                [kind],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&s).unwrap()
    }
}
pub async fn exited(worker: &mut Worker, code: i32) {
    let status = tokio::time::timeout(Duration::from_secs(20), worker.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(code));
}

pub async fn stop(worker: &mut Worker) {
    signal_stop(&mut worker.child).await;
}
