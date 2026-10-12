//! Ordinary agent credential fixture; communication uses only public clients.
#![allow(clippy::unwrap_used, dead_code)]
use connect_client::{ClientConfig, ConnectClient, Message};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::{Actor, Scope};

pub struct Fixture {
    _temp: tempfile::TempDir,
    config: ServerConfig,
    admin: Actor,
    pub server: RunningServer,
    pub agent: ClientConfig,
    pub peer: ClientConfig,
    pub foreign: ClientConfig,
    pub expires_at: i64,
    pub operator_secret: String,
}

/// Named communication capabilities, with optional task tracking but no admin.
pub fn scope(recipients: Vec<String>) -> Scope {
    let mut scope = Scope::communications(recipients);
    scope.operations.extend(
        [
            "task_create",
            "task_get",
            "task_list",
            "task_transition",
            "task_invoke",
            "task_inbox",
            "task_wait",
            "task_recall",
            "task_effect_get",
            "task_cancel",
        ]
        .map(String::from),
    );
    scope
}

/// Drain bounded public history pages; no fixture reads the server database.
pub async fn history(client: &mut ConnectClient, conversation_id: &str) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut after = None;
    for _ in 0..32 {
        let page = client
            .history(conversation_id, after.clone(), 100)
            .await
            .unwrap();
        messages.extend(page.messages);
        if page.next.is_none() {
            return messages;
        }
        assert_ne!(page.next, after, "history cursor must advance");
        after = page.next;
    }
    panic!("fixture exceeded bounded history page count")
}

impl Fixture {
    pub async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("state");
        init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        config.mcp = Some(connect_server::config::McpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            binding: None,
        });
        let server =
            RunningServer::start_local(config.clone(), Some("127.0.0.1:0".parse().unwrap()), false)
                .await
                .unwrap();
        let mut agent = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
        let mut peer = ClientConfig::load(&directory.join("agent.toml")).unwrap();
        let mut foreign = ClientConfig::load(&directory.join("operator.toml")).unwrap();
        let operator_secret = foreign.credential.clone();
        let admin = server.store.authenticate(&operator_secret).unwrap();
        let expires_at = chrono::Utc::now().timestamp() + 3600;
        let agent_id = agent.identity_id.clone();
        let peer_id = peer.identity_id.clone();
        let foreign_id = foreign.identity_id.clone();
        for (client, recipients) in [
            (&mut agent, vec![peer_id.clone(), agent_id.clone()]),
            (&mut peer, vec![agent_id, peer_id]),
            (&mut foreign, vec![foreign_id]),
        ] {
            let issued = server
                .store
                .issue(&client.identity_id, scope(recipients), expires_at)
                .unwrap();
            client.credential_id = issued.id;
            client.credential = issued.token;
            client.endpoint = server.address;
        }
        Self {
            _temp: temp,
            config,
            admin,
            server,
            agent,
            peer,
            foreign,
            expires_at,
            operator_secret,
        }
    }

    pub async fn agent_client(&self) -> ConnectClient {
        ConnectClient::open(self.agent.clone()).await.unwrap()
    }

    pub async fn peer_client(&self) -> ConnectClient {
        ConnectClient::open(self.peer.clone()).await.unwrap()
    }

    pub async fn foreign_client(&self) -> ConnectClient {
        ConnectClient::open(self.foreign.clone()).await.unwrap()
    }

    /// Trusted local issuance only. The exported token has no admin permission.
    pub fn short_credential(&self, expires_at: i64) -> ClientConfig {
        let issued = self
            .server
            .store
            .issue(
                &self.agent.identity_id,
                scope(vec![
                    self.peer.identity_id.clone(),
                    self.agent.identity_id.clone(),
                ]),
                expires_at,
            )
            .unwrap();
        let mut config = self.agent.clone();
        config.credential_id = issued.id;
        config.credential = issued.token;
        config
    }

    pub fn revoke(&self, credential_id: &str) {
        self.server
            .store
            .revoke(&self.admin, credential_id)
            .unwrap();
    }

    pub fn admin(&self) -> &Actor {
        &self.admin
    }

    pub fn server_config_path(&self) -> std::path::PathBuf {
        self._temp.path().join("state/server.toml")
    }

    /// The local operator manages a separate identity without sharing its key.
    pub fn issue_as_operator(&self, identity: &ClientConfig, scope: Scope) -> ClientConfig {
        let issued = self
            .server
            .store
            .issue_as(&self.admin, &identity.identity_id, scope, self.expires_at)
            .unwrap();
        let mut config = identity.clone();
        config.credential_id = issued.id;
        config.credential = issued.token;
        config
    }

    /// Reopen the same disposable installation; credentials and history persist.
    pub async fn restart(mut self) -> Self {
        self.server.shutdown().await.unwrap();
        self.server = RunningServer::start_local(
            self.config.clone(),
            Some("127.0.0.1:0".parse().unwrap()),
            false,
        )
        .await
        .unwrap();
        for client in [&mut self.agent, &mut self.peer, &mut self.foreign] {
            client.endpoint = self.server.address;
        }
        self
    }

    pub async fn close(self) {
        self.server.shutdown().await.unwrap();
    }
}
