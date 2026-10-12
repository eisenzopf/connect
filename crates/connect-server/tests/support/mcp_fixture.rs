//! Trusted issuance fixture. Model operations use MCP, never database shortcuts.
#![allow(clippy::unwrap_used, dead_code)]
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::tasks::*;
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub server: RunningServer,
    pub owner: ClientConfig,
    pub peer: ClientConfig,
    pub task: TaskRecord,
    pub binding: InvocationBinding,
    pub issued: connect_service::tasks::IssuedTaskGrant,
    admin: connect_store::Actor,
}
impl Fixture {
    pub async fn start() -> Self {
        Self::start_configured(false).await
    }
    pub async fn start_configured(mcp: bool) -> Self {
        Self::start_permitted(mcp, vec![TaskPermission::History]).await
    }
    pub async fn start_permitted(mcp: bool, permissions: Vec<TaskPermission>) -> Self {
        Self::start_options(mcp, false, permissions, None, 180).await
    }
    pub async fn start_permitted_with_grant_ttl(
        permissions: Vec<TaskPermission>,
        ttl: i64,
    ) -> Self {
        Self::start_options(true, false, permissions, None, ttl).await
    }
    pub async fn start_independent() -> Self {
        Self::start_independent_with_connections(64).await
    }
    pub async fn start_independent_permitted(permissions: Vec<TaskPermission>) -> Self {
        Self::start_options(true, true, permissions, Some(64), 180).await
    }
    pub async fn start_independent_with_connections(connections: usize) -> Self {
        Self::start_options(
            true,
            true,
            vec![TaskPermission::History],
            Some(connections),
            180,
        )
        .await
    }
    pub fn directory(&self) -> &std::path::Path {
        self._temp.path()
    }
    async fn start_options(
        mcp: bool,
        independent: bool,
        permissions: Vec<TaskPermission>,
        connections: Option<usize>,
        grant_ttl: i64,
    ) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("state");
        init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let binding = InvocationBinding {
            kind: InvocationKind::Native,
            adapter: "connect-mcp-test".into(),
            external_id: "installed-task-runner".into(),
        };
        let mut config = ServerConfig::load(&dir.join("server.toml")).unwrap();
        if let Some(connections) = connections {
            config.limits.connections = connections;
        }
        if mcp {
            config.mcp = Some(connect_server::config::McpConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                binding: (!independent).then(|| binding.clone()),
            });
        }
        let http = independent.then(|| "127.0.0.1:0".parse().unwrap());
        let server = RunningServer::start_interfaces(config, http, http, false)
            .await
            .unwrap();
        let mut owner = ClientConfig::load(&dir.join("reviews.toml")).unwrap();
        let operator = ClientConfig::load(&dir.join("operator.toml")).unwrap();
        let admin = server.store.authenticate(&operator.credential).unwrap();
        let mut peer = ClientConfig::load(&dir.join("agent.toml")).unwrap();
        peer.endpoint = server.address;
        let mut operations = vec![
            "create",
            "resolve",
            "send",
            "history",
            "context_view",
            "inbox",
            "request_status",
            "task_create",
            "task_get",
            "task_invoke",
            "task_cancel",
            "task_grant",
        ];
        for permission in &permissions {
            if !operations.contains(&permission.required_scope()) {
                operations.push(permission.required_scope());
            }
        }
        let issued = server
            .store
            .issue(
                &owner.identity_id,
                Scope {
                    operations: operations.iter().map(|op| (*op).into()).collect(),
                    recipients: vec![peer.identity_id.clone()],
                },
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        owner.endpoint = server.address;
        owner.credential_id = issued.id;
        owner.credential = issued.token;
        let mut client = ConnectClient::open(owner.clone()).await.unwrap();
        let cid = client
            .create("mcp-conversation", vec![peer.identity_id.clone()])
            .await
            .unwrap()
            .id;
        client
            .negotiate_tasks("connect:agent@local", Some(&cid))
            .await
            .unwrap();
        let TaskOutcome::Record(task) = client
            .task(TaskCommand::Create {
                request_id: "mcp-task".into(),
                definition: TaskDefinition {
                    conversation_id: cid,
                    scope: TaskScope {
                        operations: permissions.clone(),
                        recipients: vec![peer.identity_id.clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: chrono::Utc::now().timestamp() + 600,
                    initiating_event_id: None,
                },
            })
            .await
            .unwrap()
        else {
            panic!("task record")
        };
        client.close().await;
        server
            .service
            .register_task_binding(
                owner.credential_id.clone(),
                task.id.clone(),
                binding.clone(),
            )
            .await
            .unwrap();
        let issued = server
            .service
            .issue_task_grant(
                owner.credential_id.clone(),
                "mcp-grant".into(),
                GrantDefinition {
                    task_id: task.id.clone(),
                    binding: binding.clone(),
                    operations: permissions,
                    expires_at: chrono::Utc::now().timestamp() + grant_ttl,
                },
            )
            .await
            .unwrap();
        Self {
            _temp: temp,
            server,
            owner,
            peer,
            task: *task,
            binding,
            issued,
            admin,
        }
    }
    pub fn revoke_parent(&self) {
        self.server
            .store
            .revoke(&self.admin, &self.owner.credential_id)
            .unwrap();
    }
    pub async fn close(self) {
        self.server.shutdown().await.unwrap();
    }
}
