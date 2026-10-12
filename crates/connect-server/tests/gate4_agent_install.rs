//! Protected owner provisioning and agent installs preserve separate authority.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
use connect_protocol::Limits;
use connect_server::{
    cli::{agent_access, init::private_write},
    config::ServerConfig,
};
use connect_store::{Scope, Store};
use std::time::Duration;

#[test]
fn protected_identity_setup_is_atomic_idempotent_and_rechecks_current_admin() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite3");
    let store = Store::initialize(&database, Limits::default()).unwrap();
    let owner = store.create_identity("owner").unwrap();
    let ordinary = store.create_identity("ordinary").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let admin_key = store.issue(&owner.id, Scope::operator(), expiry).unwrap();
    let ordinary_key = store
        .issue(
            &ordinary.id,
            Scope::communications(vec!["*".into()]),
            expiry,
        )
        .unwrap();
    let admin = store.authenticate(&admin_key.token).unwrap();
    let non_admin = store.authenticate(&ordinary_key.token).unwrap();
    let before = Store::inspect_snapshot(&database).unwrap();
    assert_eq!(
        store
            .create_identity_as(&non_admin, "provision-agent", "helper")
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(&database).unwrap(), before);
    let helper = store
        .create_identity_as(&admin, "provision-agent", "helper")
        .unwrap();
    let committed = Store::inspect_snapshot(&database).unwrap();
    assert_eq!(
        store
            .create_identity_as(&admin, "provision-agent", "helper")
            .unwrap(),
        helper
    );
    assert_eq!(Store::inspect_snapshot(&database).unwrap(), committed);
    assert_eq!(
        store
            .create_identity_as(&admin, "provision-agent", "different")
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        store
            .create_identity_as(&admin, "adopt-another-request", "helper")
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        store
            .create_identity_as(&admin, "bad/request", "new-helper")
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(Store::inspect_snapshot(&database).unwrap(), committed);
    assert_eq!(
        store.resolve(&admin, "connect:helper@local").unwrap(),
        helper
    );
    assert!(store.resolve(&admin, "connect:different@local").is_err());
    store.revoke(&admin, &admin.credential_id).unwrap();
    let revoked = Store::inspect_snapshot(&database).unwrap();
    assert_eq!(
        store
            .create_identity_as(&admin, "provision-agent", "helper")
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert_eq!(
        store
            .create_identity_as(&admin, "new-after-admin-revoke", "another")
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert!(
        matches!(store.issue_as(&admin, &helper.id, Scope::messaging(vec![]), expiry),
        Err(error) if error.code == "unauthorized")
    );
    assert_eq!(Store::inspect_snapshot(&database).unwrap(), revoked);
}

#[tokio::test]
async fn readonly_install_authentication_never_writes_or_purges_and_observes_revoke_and_expiry() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("state.sqlite3");
    let store = Store::initialize(&database, Limits::default()).unwrap();
    let owner = store.create_identity("owner").unwrap();
    let identity = store.create_identity("agent").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let admin_key = store.issue(&owner.id, Scope::operator(), expiry).unwrap();
    let admin = store.authenticate(&admin_key.token).unwrap();
    let key = store
        .issue_as(&admin, &identity.id, Scope::messaging(vec![]), expiry)
        .unwrap();
    let orphan = store
        .blob_directory()
        .join(format!("{}.blob", uuid::Uuid::new_v4()));
    std::fs::write(
        &orphan,
        b"A read-only credential inspection must leave this file alone.",
    )
    .unwrap();
    let before = Store::inspect_snapshot(&database).unwrap();
    let observed = Store::authenticate_file(&database, &key.token).unwrap();
    assert_eq!(observed.identity_id, identity.id);
    assert_eq!(observed.credential_id, key.id);
    assert!(!observed.permits("admin"));
    for invalid in ["not-a-credential".to_owned(), "x".repeat(64)] {
        assert_eq!(
            Store::authenticate_file(&database, &invalid)
                .unwrap_err()
                .code,
            "unauthorized"
        );
    }
    assert_eq!(Store::inspect_snapshot(&database).unwrap(), before);
    assert!(orphan.exists());
    store.revoke(&admin, &key.id).unwrap();
    let revoked = Store::inspect_snapshot(&database).unwrap();
    assert_eq!(
        Store::authenticate_file(&database, &key.token)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert_eq!(Store::inspect_snapshot(&database).unwrap(), revoked);
    let short_expiry = chrono::Utc::now().timestamp() + 2;
    let short = store
        .issue_as(&admin, &identity.id, Scope::messaging(vec![]), short_expiry)
        .unwrap();
    assert!(Store::authenticate_file(&database, &short.token).is_ok());
    let before_expiry = Store::inspect_snapshot(&database).unwrap();
    while chrono::Utc::now().timestamp() < short_expiry {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        Store::authenticate_file(&database, &short.token)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert_eq!(Store::inspect_snapshot(&database).unwrap(), before_expiry);
    assert!(orphan.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_export_contains_only_selected_agent_authority_and_no_operator_or_provider_secret()
{
    let f = agent::Fixture::start().await;
    let server_path = f.server_config_path();
    let state = server_path.parent().unwrap();
    let client_path = state.join("selected-agent.toml");
    private_write(
        &client_path,
        toml::to_string_pretty(&f.agent).unwrap().as_bytes(),
    )
    .unwrap();
    let provider_secret = "offline-provider-master-secret-do-not-copy";
    private_write(
        &state.join("private-provider.env"),
        provider_secret.as_bytes(),
    )
    .unwrap();
    let mut settings = ServerConfig::load(&server_path).unwrap();
    settings.mcp = Some(connect_server::config::McpConfig {
        bind: f.server.mcp_address.unwrap(),
        binding: None,
    });
    let selected_server_path = state.join("selected-server.toml");
    private_write(
        &selected_server_path,
        toml::to_string_pretty(&settings).unwrap().as_bytes(),
    )
    .unwrap();
    let output = state.join("ordinary-host");
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    agent_access::export(
        &client_path,
        &selected_server_path,
        f.server.mcp_address.unwrap(),
        &output,
    )
    .unwrap();
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    let installed = connect_client::ClientConfig::load(&output.join("agent.toml")).unwrap();
    assert_eq!(installed.credential, f.agent.credential);
    assert_eq!(installed.credential_id, f.agent.credential_id);
    assert_eq!(installed.identity_id, f.agent.identity_id);
    assert!(!f
        .server
        .store
        .authenticate(&installed.credential)
        .unwrap()
        .permits("admin"));
    for path in ["install.json", "mcp-override.txt"] {
        let contents = std::fs::read_to_string(output.join(path)).unwrap();
        assert!(!contents.contains(&f.agent.credential));
        assert!(!contents.contains(&f.operator_secret));
        assert!(!contents.contains(provider_secret));
    }
    assert!(!output.join("operator.toml").exists());
    assert!(!output.join("private-provider.env").exists());
    assert!(!output.join("task-grant.json").exists());
    assert!(!output.join("grant-renewal.json").exists());
    assert!(agent_access::export(
        &client_path,
        &selected_server_path,
        f.server.mcp_address.unwrap(),
        &output
    )
    .is_err());
    assert_eq!(
        connect_client::ClientConfig::load(&output.join("agent.toml"))
            .unwrap()
            .credential,
        installed.credential
    );
    let forbidden = state.join("operator-host");
    assert!(agent_access::export(
        &state.join("operator.toml"),
        &selected_server_path,
        f.server.mcp_address.unwrap(),
        &forbidden
    )
    .is_err());
    assert!(!forbidden.exists());
    f.revoke(&f.agent.credential_id);
    let denied = state.join("revoked-host");
    assert!(agent_access::export(
        &client_path,
        &selected_server_path,
        f.server.mcp_address.unwrap(),
        &denied
    )
    .is_err());
    assert!(!denied.exists());
    f.close().await;
}
