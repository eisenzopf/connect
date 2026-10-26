//! Trusted fixture setup; the example process communicates only through QUIC SDK.
#![allow(clippy::unwrap_used, dead_code)]
use connect_client::{tasks::*, ClientConfig, ConnectClient};
use connect_review_loop::task_runner::{Statistics, TaskStore};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;
use std::{fs::File, path::PathBuf, process::Stdio, time::Duration};
use tokio::process::{Child, Command};

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub directory: PathBuf,
    pub config: ServerConfig,
    pub server: Option<RunningServer>,
    pub owner_config: ClientConfig,
    pub peer_config: ClientConfig,
    pub owner: Option<ConnectClient>,
    pub peer: Option<ConnectClient>,
    pub task: TaskRecord,
    pub grant_id: String,
}
pub struct Worker {
    child: Child,
    log: PathBuf,
}
impl Fixture {
    pub async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("local");
        init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        let server =
            RunningServer::start_local(config.clone(), Some("127.0.0.1:0".parse().unwrap()), false)
                .await
                .unwrap();
        let mut owner_config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
        let mut peer_config = ClientConfig::load(&directory.join("agent.toml")).unwrap();
        let credential = server
            .store
            .issue(
                &owner_config.identity_id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        owner_config.endpoint = server.address;
        owner_config.credential_id = credential.id;
        owner_config.credential = credential.token;
        peer_config.endpoint = server.address;
        let mut owner = ConnectClient::open(owner_config.clone()).await.unwrap();
        let mut peer = ConnectClient::open(peer_config.clone()).await.unwrap();
        let cid = owner
            .create("runner-conversation", vec![peer_config.identity_id.clone()])
            .await
            .unwrap()
            .id;
        owner
            .negotiate_tasks("connect:agent@local", Some(&cid))
            .await
            .unwrap();
        peer.negotiate_tasks("connect:reviews@local", Some(&cid))
            .await
            .unwrap();
        let permissions = vec![
            TaskPermission::Send,
            TaskPermission::History,
            TaskPermission::Inbox,
            TaskPermission::RequestStatus,
        ];
        let task = owner
            .task(TaskCommand::Create {
                request_id: "runner-task".into(),
                definition: TaskDefinition {
                    conversation_id: cid,
                    scope: TaskScope {
                        operations: permissions.clone(),
                        recipients: vec![peer_config.identity_id.clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: chrono::Utc::now().timestamp() + 600,
                    initiating_event_id: None,
                },
            })
            .await
            .unwrap();
        let TaskOutcome::Record(task) = task else {
            panic!("record")
        };
        let issued = server
            .service
            .execute_task_access(
                owner_config.credential_id.clone(),
                TaskAccessRequest::new(TaskAccessCommand::Issue {
                    request_id: "runner-grant".into(),
                    definition: GrantDefinition {
                        task_id: task.id.clone(),
                        binding: InvocationBinding {
                            kind: InvocationKind::Native,
                            adapter: "reference-runner-test".into(),
                            external_id: "runner".into(),
                        },
                        operations: permissions,
                        expires_at: chrono::Utc::now().timestamp() + 180,
                    },
                }),
            )
            .await
            .unwrap();
        let TaskAccessOutcome::Issued(grant) = issued else {
            panic!("grant")
        };
        let invocation = owner_config.clone().with_task_grant(&grant).unwrap();
        for (file, config) in [
            ("controller.toml", &owner_config),
            ("invocation.toml", &invocation),
        ] {
            init::private_write(
                &directory.join(file),
                toml::to_string_pretty(config).unwrap().as_bytes(),
            )
            .unwrap();
        }
        Self {
            _temp: temp,
            directory,
            config,
            server: Some(server),
            owner_config,
            peer_config,
            owner: Some(owner),
            peer: Some(peer),
            task: *task,
            grant_id: grant.grant.id,
        }
    }

    pub fn state(&self) -> PathBuf {
        self.directory.join("runner.sqlite3")
    }
    pub fn spawn(&self, fault: Option<&str>, state: Option<PathBuf>) -> Worker {
        let path = self
            .directory
            .join(format!("runner-{}.log", uuid::Uuid::new_v4()));
        let log = File::create(&path).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_connect-example"));
        command
            .args(["task", "--controller-config"])
            .arg(self.directory.join("controller.toml"))
            .arg("--invocation-config")
            .arg(self.directory.join("invocation.toml"))
            .arg("--task-id")
            .arg(&self.task.id)
            .arg("--state")
            .arg(state.unwrap_or_else(|| self.state()))
            .env_remove("CONNECT_TASK_RUNNER_FAULT")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .kill_on_drop(true);
        if let Some(fault) = fault {
            command.env("CONNECT_TASK_RUNNER_FAULT", fault);
        }
        Worker {
            child: command.spawn().unwrap(),
            log: path,
        }
    }

    pub async fn question(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let history = self
                    .peer
                    .as_mut()
                    .unwrap()
                    .history(&self.task.conversation_id, None, 100)
                    .await
                    .unwrap();
                if let Some(message) = history
                    .messages
                    .iter()
                    .find(|m| m.body == "Please review draft-002 and reply to this question.")
                {
                    return message.id.clone();
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap()
    }
    pub async fn answer(&mut self, question: &str) {
        self.peer
            .as_mut()
            .unwrap()
            .task(TaskCommand::Reply {
                request_id: "review-response".into(),
                conversation_id: self.task.conversation_id.clone(),
                reply_to: question.into(),
                body: "Approved draft-002.".into(),
            })
            .await
            .unwrap();
    }
    pub async fn settled(&self, effects: u64) -> Statistics {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Ok(stats) = TaskStore::inspect(&self.state()) {
                    if stats.business_effects == effects
                        && stats.pending_commands == 0
                        && !stats.page_pending
                    {
                        return stats;
                    }
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap()
    }
    pub async fn finish(&mut self) {
        if let Some(client) = self.owner.take() {
            client.close().await;
        }
        if let Some(client) = self.peer.take() {
            client.close().await;
        }
        if let Some(server) = self.server.take() {
            server.shutdown().await.unwrap();
        }
    }
    pub async fn reissue(&mut self) {
        let grant = self
            .server
            .as_ref()
            .unwrap()
            .service
            .execute_task_access(
                self.owner_config.credential_id.clone(),
                TaskAccessRequest::new(TaskAccessCommand::Issue {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    definition: GrantDefinition {
                        task_id: self.task.id.clone(),
                        binding: InvocationBinding {
                            kind: InvocationKind::Native,
                            adapter: "reference-runner-test".into(),
                            external_id: uuid::Uuid::new_v4().to_string(),
                        },
                        operations: self.task.scope.operations.clone(),
                        expires_at: chrono::Utc::now().timestamp() + 180,
                    },
                }),
            )
            .await
            .unwrap();
        let TaskAccessOutcome::Issued(grant) = grant else {
            panic!("grant")
        };
        let invocation = self.owner_config.clone().with_task_grant(&grant).unwrap();
        init::private_replace(
            &self.directory.join("invocation.toml"),
            toml::to_string_pretty(&invocation).unwrap().as_bytes(),
        )
        .unwrap();
    }
    pub fn diagnostic_config(&self, name: &str) -> PathBuf {
        let native = ClientConfig::load(&self.directory.join(format!("{name}.toml"))).unwrap();
        let application = connect_client::application::ApplicationClientConfig {
            endpoint: self.server.as_ref().unwrap().http_address.unwrap(),
            credential: native.credential,
            expires_at: chrono::DateTime::from_timestamp(self.task.expires_at, 0).unwrap(),
            timeout_ms: 10_000,
        };
        let path = self.directory.join(format!("{name}.application.toml"));
        init::private_replace(
            &path,
            toml::to_string_pretty(&application).unwrap().as_bytes(),
        )
        .unwrap();
        path
    }
    pub async fn restart(&mut self) {
        let address = self.server.as_ref().unwrap().address;
        let application_address = self.server.as_ref().unwrap().http_address.unwrap();
        self.finish().await;
        // start_local's optional bind is HTTP; native QUIC uses config.bind.
        self.config.bind = address;
        self.server = Some(
            RunningServer::start_local(self.config.clone(), Some(application_address), false)
                .await
                .unwrap(),
        );
        let mut owner = ConnectClient::open(self.owner_config.clone())
            .await
            .unwrap();
        owner.negotiate_task(&self.task.id).await.unwrap();
        self.owner = Some(owner);
        let mut peer = ConnectClient::open(self.peer_config.clone()).await.unwrap();
        peer.negotiate_tasks("connect:reviews@local", Some(&self.task.conversation_id))
            .await
            .unwrap();
        self.peer = Some(peer);
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
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if std::fs::read_to_string(&worker.log)
                .unwrap()
                .contains("ready role=task-runner")
            {
                break;
            }
            assert!(
                worker.child.try_wait().unwrap().is_none(),
                "Runner exited before readiness"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let pid = worker.child.id().unwrap();
    assert!(Command::new("kill")
        .arg("-INT")
        .arg(pid.to_string())
        .status()
        .await
        .unwrap()
        .success());
    exited(worker, 0).await;
}
