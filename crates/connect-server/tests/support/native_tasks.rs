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
        Self::start_with_vapi(None).await
    }
    pub async fn start_with_vapi(adapter: Option<std::sync::Arc<rvoip_vapi::VapiAdapter>>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("state");
        init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        let http = Some("127.0.0.1:0".parse().unwrap());
        let server = if let Some(adapter) = adapter {
            RunningServer::start_with_vapi_media(config, http, adapter)
                .await
                .unwrap()
        } else {
            RunningServer::start_local(config, http, false)
                .await
                .unwrap()
        };
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
    /// Restart the owned server against its existing database and TLS identity.
    /// Clients reconnect through the public SDK; no task state is reconstructed.
    pub async fn restart(self) -> Self {
        self.reopen(false).await
    }
    /// Restore a verified backup into a new installation before reconnecting.
    pub async fn restore_snapshot(self) -> Self {
        self.reopen(true).await
    }
    async fn reopen(self, restore: bool) -> Self {
        let Self {
            _temp,
            server,
            owner,
            peer,
            mut owner_config,
            peer_id,
            cid,
        } = self;
        owner.close().await;
        peer.close().await;
        let source = server.store.path().parent().unwrap().to_owned();
        server.shutdown().await.unwrap();
        let directory = if restore {
            let backup = _temp.path().join("backup");
            let restored = _temp.path().join("restored");
            let original =
                connect_store::Store::inspect_snapshot(&source.join("state.sqlite3")).unwrap();
            tokio::task::spawn_blocking(move || {
                connect_server::cli::recovery::backup(
                    &source.join("server.toml"),
                    &source.join("operator.toml"),
                    &backup,
                )
                .unwrap();
                connect_server::cli::recovery::restore(
                    &backup,
                    &restored,
                    "127.0.0.1:0".parse().unwrap(),
                    None,
                )
                .unwrap();
                assert_eq!(
                    connect_store::Store::inspect_snapshot(&source.join("state.sqlite3")).unwrap(),
                    original
                );
                assert_eq!(
                    connect_store::Store::inspect_snapshot(&restored.join("state.sqlite3"))
                        .unwrap(),
                    original
                );
                restored
            })
            .await
            .unwrap()
        } else {
            source
        };
        let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        let server =
            RunningServer::start_local(config, Some("127.0.0.1:0".parse().unwrap()), false)
                .await
                .unwrap();
        let mut peer_config = ClientConfig::load(&directory.join("agent.toml")).unwrap();
        owner_config.endpoint = server.address;
        owner_config.trust_certificate = directory.join("certificate.der");
        peer_config.endpoint = server.address;
        let owner = ConnectClient::open(owner_config.clone()).await.unwrap();
        let peer = ConnectClient::open(peer_config).await.unwrap();
        Self {
            _temp,
            server,
            owner,
            peer,
            owner_config,
            peer_id,
            cid,
        }
    }
    /// Transfer unchanged durable state to an independently launched CLI host.
    pub async fn stop_for_host(self) -> StoppedHost {
        let Self {
            _temp,
            server,
            owner,
            peer,
            owner_config,
            peer_id,
            cid,
        } = self;
        owner.close().await;
        peer.close().await;
        let directory = server.store.path().parent().unwrap().to_owned();
        let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        config.bind = server.address;
        server.shutdown().await.unwrap();
        init::private_replace(
            &directory.join("server.toml"),
            toml::to_string(&config).unwrap().as_bytes(),
        )
        .unwrap();
        init::private_write(
            &directory.join("host-owner.toml"),
            toml::to_string(&owner_config).unwrap().as_bytes(),
        )
        .unwrap();
        StoppedHost {
            _temp,
            directory,
            owner: owner_config,
            peer_id,
            cid,
        }
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

/// Private installation kept alive while a separate installed binary owns it.
pub(crate) struct StoppedHost {
    _temp: tempfile::TempDir,
    pub directory: std::path::PathBuf,
    pub owner: ClientConfig,
    pub peer_id: String,
    pub cid: String,
}
