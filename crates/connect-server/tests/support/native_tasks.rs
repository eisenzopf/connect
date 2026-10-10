//! Public-client task fixture. Store access is limited to trusted credential setup.
#![allow(clippy::unwrap_used, dead_code)]
use connect_client::{tasks::*, ClientConfig, ConnectClient};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;

pub(crate) struct Harness {
    _temp: tempfile::TempDir,
    pub server: RunningServer,
    pub owner: ConnectClient,
    pub peer: ConnectClient,
    pub owner_config: ClientConfig,
    pub peer_id: String,
    pub cid: String,
}
impl Harness {
    pub async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("state");
        init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        let server =
            RunningServer::start_local(config, Some("127.0.0.1:0".parse().unwrap()), false)
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
        let peer_id = peer_config.identity_id.clone();
        let mut owner = ConnectClient::open(owner_config.clone()).await.unwrap();
        let peer = ConnectClient::open(peer_config).await.unwrap();
        let cid = owner
            .create("native-task-conversation", vec![peer_id.clone()])
            .await
            .unwrap()
            .id;
        Self {
            _temp: temp,
            server,
            owner,
            peer,
            owner_config,
            peer_id,
            cid,
        }
    }
    pub fn definition(&self) -> TaskDefinition {
        TaskDefinition {
            conversation_id: self.cid.clone(),
            scope: TaskScope {
                operations: vec![TaskPermission::Send, TaskPermission::History],
                recipients: vec![self.peer_id.clone()],
                sms_enrollments: Vec::new(),
            },
            expires_at: chrono::Utc::now().timestamp() + 600,
            initiating_event_id: None,
        }
    }
    pub async fn create(&mut self, id: &str) -> TaskRecord {
        record(
            self.owner
                .task(TaskCommand::Create {
                    request_id: id.into(),
                    definition: self.definition(),
                })
                .await
                .unwrap(),
        )
    }
    pub async fn close(self) {
        self.owner.close().await;
        self.peer.close().await;
        self.server.shutdown().await.unwrap();
    }
}
pub(crate) fn record(outcome: TaskOutcome) -> TaskRecord {
    let TaskOutcome::Record(record) = outcome else {
        panic!("expected task record")
    };
    *record
}
