//! Installation preserves ordinary authority and cannot widen existing intents.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    cli::{credentials::with_task_management, init::private_write},
    config::{McpConfig, ServerConfig},
};
use connect_client::ClientConfig;
use connect_protocol::Limits;
use connect_store::{Scope, Store};
use std::path::PathBuf;

struct Fixture {
    directory: tempfile::TempDir,
    store: Store,
    client: ClientConfig,
    config: PathBuf,
    server: PathBuf,
    address: std::net::SocketAddr,
}
impl Fixture {
    fn new(scope: Scope) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let store = Store::initialize(&root.join("state.sqlite3"), Limits::default()).unwrap();
        let identity = store.create_identity("agent").unwrap();
        let key = store
            .issue(
                &identity.id,
                scope,
                chrono::Utc::now().timestamp() + 30 * 86400,
            )
            .unwrap();
        private_write(&root.join("certificate.der"), b"local-test-certificate").unwrap();
        let address = "127.0.0.1:19193".parse().unwrap();
        let client = ClientConfig {
            endpoint: "127.0.0.1:19443".parse().unwrap(),
            server_name: "localhost".into(),
            trust_certificate: root.join("certificate.der"),
            credential: key.token,
            identity_id: identity.id,
            credential_id: key.id,
            timeout_ms: 10_000,
        };
        let config = root.join("agent.toml");
        private_write(&config, toml::to_string_pretty(&client).unwrap().as_bytes()).unwrap();
        let server = root.join("server.toml");
        let settings = ServerConfig {
            bind: client.endpoint,
            websocket_bind: None,
            database: root.join("state.sqlite3"),
            certificate: client.trust_certificate.clone(),
            private_key: root.join("private-key.der"),
            mcp: Some(McpConfig {
                bind: address,
                binding: None,
            }),
            sip: None,
            webrtc: None,
            limits: Limits::default(),
        };
        private_write(
            &server,
            toml::to_string_pretty(&settings).unwrap().as_bytes(),
        )
        .unwrap();
        Self {
            directory,
            store,
            client,
            config,
            server,
            address,
        }
    }
    fn ordinary() -> Self {
        Self::new(with_task_management(Scope::communications(
            vec!["*".into()],
        )))
    }
    fn install(&self) -> PathBuf {
        let output = self.directory.path().join("host");
        export(&self.config, &self.server, self.address, &output).unwrap();
        output
    }
}

#[test]
fn regular_export_keeps_one_agent_key_without_task_grants_or_admin_secret() {
    let fixture = Fixture::ordinary();
    let before = Store::inspect_snapshot(&fixture.directory.path().join("state.sqlite3")).unwrap();
    let output = fixture.install();
    assert_eq!(
        Store::inspect_snapshot(&fixture.directory.path().join("state.sqlite3")).unwrap(),
        before
    );
    let installation = codex::load(&output).unwrap();
    assert_eq!(
        installation.client.credential_id,
        fixture.client.credential_id
    );
    assert_eq!(installation.client.credential, fixture.client.credential);
    assert!(!output.join("task-grant.json").exists());
    assert!(!output.join("grant-renewal.json").exists());
    let manifest = std::fs::read_to_string(output.join("install.json")).unwrap();
    let config = std::fs::read_to_string(output.join("mcp-override.txt")).unwrap();
    assert!(!manifest.contains(&fixture.client.credential));
    assert!(!config.contains(&fixture.client.credential));
    assert!(config.contains("CONNECT_AGENT_CREDENTIAL"));
    assert!(!config.contains("CONNECT_TASK_GRANT"));
    assert!(export(&fixture.config, &fixture.server, fixture.address, &output).is_err());
    assert_eq!(codex::load(&output).unwrap().digest, installation.digest);
}

#[test]
fn export_rejects_admin_revoked_and_mismatched_authority_before_creating_output() {
    let admin = Fixture::new(Scope::operator());
    let output = admin.directory.path().join("forbidden");
    assert!(export(&admin.config, &admin.server, admin.address, &output).is_err());
    assert!(!output.exists());
    let fixture = Fixture::ordinary();
    let owner = fixture.store.create_identity("owner").unwrap();
    let owner_key = fixture
        .store
        .issue(
            &owner.id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 86400,
        )
        .unwrap();
    fixture
        .store
        .revoke(
            &fixture.store.authenticate(&owner_key.token).unwrap(),
            &fixture.client.credential_id,
        )
        .unwrap();
    let output = fixture.directory.path().join("revoked");
    assert!(export(&fixture.config, &fixture.server, fixture.address, &output).is_err());
    assert!(!output.exists());
    let fixture = Fixture::ordinary();
    let mut changed = fixture.client.clone();
    changed.identity_id = uuid::Uuid::new_v4().to_string();
    let path = fixture.directory.path().join("mismatched.toml");
    private_write(&path, toml::to_string_pretty(&changed).unwrap().as_bytes()).unwrap();
    let output = fixture.directory.path().join("mismatched");
    assert!(export(&path, &fixture.server, fixture.address, &output).is_err());
    assert!(!output.exists());
    let mut narrow = fixture.client.clone();
    narrow.credential_id =
        connect_protocol::tasks::task_grant_subject(&uuid::Uuid::new_v4().to_string()).unwrap();
    narrow.credential = "ctg1.fixture-secret".into();
    let path = fixture.directory.path().join("narrow.toml");
    private_write(&path, toml::to_string_pretty(&narrow).unwrap().as_bytes()).unwrap();
    let output = fixture.directory.path().join("not-broadened");
    assert!(export(&path, &fixture.server, fixture.address, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn changed_credential_assets_or_access_mode_cannot_resume_an_existing_installation() {
    let fixture = Fixture::ordinary();
    let output = fixture.install();
    let original = std::fs::read(output.join("agent.toml")).unwrap();
    let mut changed = fixture.client.clone();
    changed.credential = "a".repeat(64);
    crate::cli::init::private_replace(
        &output.join("agent.toml"),
        toml::to_string_pretty(&changed).unwrap().as_bytes(),
    )
    .unwrap();
    assert!(codex::load(&output).is_err());
    crate::cli::init::private_replace(&output.join("agent.toml"), &original).unwrap();
    crate::cli::init::private_replace(&output.join("mcp-override.txt"), b"changed").unwrap();
    assert!(codex::load(&output).is_err());
    crate::cli::init::private_replace(
        &output.join("mcp-override.txt"),
        crate::cli::task_access::codex::agent_configuration(&format!(
            "http://{}/mcp",
            fixture.address
        ))
        .as_bytes(),
    )
    .unwrap();
    private_write(&output.join("task-grant.json"), b"{}").unwrap();
    assert!(codex::load(&output).is_err());
    let saved = state::State {
        profile: "connect-codex-host/1".into(),
        installation: "another-mode".into(),
        identity_id: fixture.client.identity_id,
        credential_id: fixture.client.credential_id,
        session_id: Some(uuid::Uuid::new_v4().to_string()),
        run_id: uuid::Uuid::new_v4().to_string(),
        prompt: "Retain original work".into(),
        completed: false,
        attempts: 1,
    };
    saved.create(&output).unwrap();
    assert!(state::State::load(&output).is_err());
}

#[test]
fn identity_setup_replays_original_receipt_and_denies_conflict_or_non_admin() {
    let owner = Fixture::new(Scope::operator());
    create(
        &owner.config,
        &owner.server,
        "assistant",
        Some("same-identity-intent"),
    )
    .unwrap();
    let actor = owner.store.authenticate(&owner.client.credential).unwrap();
    let first = owner
        .store
        .resolve(&actor, "connect:assistant@local")
        .unwrap();
    let snapshot = Store::inspect_snapshot(&owner.directory.path().join("state.sqlite3")).unwrap();
    create(
        &owner.config,
        &owner.server,
        "assistant",
        Some("same-identity-intent"),
    )
    .unwrap();
    assert_eq!(
        owner
            .store
            .resolve(&actor, "connect:assistant@local")
            .unwrap()
            .id,
        first.id
    );
    assert_eq!(
        Store::inspect_snapshot(&owner.directory.path().join("state.sqlite3")).unwrap(),
        snapshot
    );
    assert!(create(
        &owner.config,
        &owner.server,
        "changed",
        Some("same-identity-intent")
    )
    .is_err());
    assert!(create(
        &owner.config,
        &owner.server,
        "agent",
        Some("another-identity-intent")
    )
    .is_err());
    assert_eq!(
        Store::inspect_snapshot(&owner.directory.path().join("state.sqlite3")).unwrap(),
        snapshot
    );
    let agent = Fixture::ordinary();
    let snapshot = Store::inspect_snapshot(&agent.directory.path().join("state.sqlite3")).unwrap();
    assert!(create(&agent.config, &agent.server, "unexpected", None).is_err());
    assert_eq!(
        Store::inspect_snapshot(&agent.directory.path().join("state.sqlite3")).unwrap(),
        snapshot
    );
}

#[cfg(unix)]
#[tokio::test]
async fn regular_host_resumes_exact_session_and_redacts_its_key_without_grant_refresh() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::ordinary();
    let output = fixture.install();
    let script = fixture.directory.path().join("fake-codex");
    private_write(&script,br##"#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
if sys.argv[1:] == ['--version']:
    print('codex-cli 0.162.0-alpha.17.2'); sys.exit(0)
if sys.argv[1:] == ['login', 'status']:
    print('Logged in using a fixture account'); sys.exit(0)
state = json.loads(Path('host-state.json').read_text())
session = state['session_id'] or '01a12f42-67d2-70b2-8b61-963c5f7db249'
if state['session_id']:
    assert sys.argv[1:4] == ['exec', 'resume', session]
else:
    assert sys.argv[1] == 'exec' and 'resume' not in sys.argv
assert 'CONNECT_TASK_GRANT' not in os.environ
assert 'TELNYX_API_KEY' not in os.environ and 'VAPI_PRIVATE_KEY' not in os.environ
assert sys.stdin.read() == state['prompt']
print(json.dumps({'type':'thread.started','thread_id':session}), flush=True)
print(json.dumps({'type':'item.completed','credential':os.environ['CONNECT_AGENT_CREDENTIAL']}), flush=True)
print(json.dumps({'type':'turn.completed'}), flush=True)
"##).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let prompt = fixture.directory.path().join("prompt.txt");
    private_write(
        &prompt,
        b"Use Connect to share the review in its conversation.",
    )
    .unwrap();
    run(&output, &script, Some(&prompt), 10).await.unwrap();
    let first = state::State::load(&output).unwrap().unwrap();
    run(&output, &script, None, 10).await.unwrap();
    let second = state::State::load(&output).unwrap().unwrap();
    assert_eq!(first.session_id, second.session_id);
    assert_eq!(first.credential_id, second.credential_id);
    assert_ne!(first.run_id, second.run_id);
    assert!(first.completed && second.completed);
    for path in std::fs::read_dir(&output)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
    {
        let bytes = std::fs::read_to_string(path).unwrap();
        assert!(!bytes.contains(&fixture.client.credential));
        assert!(bytes.contains("[REDACTED]"));
    }
    assert!(!output.join("grant-renewal.json").exists());
    let view = diagnostics::inspect(Some(&output), &fixture.client.identity_id).unwrap();
    assert_eq!(
        view.state,
        crate::cli::task_access::codex_host::diagnostics::HostState::TurnComplete
    );
    assert_eq!(view.session_id, second.session_id);
    let wrong = diagnostics::inspect(Some(&output), "another-owner").unwrap();
    assert_eq!(
        wrong.state,
        crate::cli::task_access::codex_host::diagnostics::HostState::BindingMismatch
    );
    assert!(wrong.session_id.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn unknown_host_or_missing_model_auth_preserves_installation_and_does_not_launch() {
    use std::os::unix::fs::PermissionsExt;
    for (name,script,error) in [
        ("unknown", "#!/bin/sh\nprintf 'codex-cli unknown\\n'\n", "Unknown Codex host version"),
        ("no-auth", "#!/bin/sh\nif [ \"$1\" = '--version' ]; then printf 'codex-cli 0.162.0-alpha.17.2\\n'; exit 0; fi\nexit 1\n", "no usable authentication"),
    ] {
        let fixture=Fixture::ordinary();let output=fixture.install();let binary=fixture.directory.path().join(name);
        private_write(&binary,script.as_bytes()).unwrap();std::fs::set_permissions(&binary,std::fs::Permissions::from_mode(0o700)).unwrap();
        let prompt=fixture.directory.path().join("prompt.txt");private_write(&prompt,b"Fixture only").unwrap();
        let prior=std::fs::read(output.join("install.json")).unwrap();
        assert!(run(&output,&binary,Some(&prompt),10).await.unwrap_err().to_string().contains(error));
        assert!(!output.join("host-state.json").exists());assert_eq!(prior,std::fs::read(output.join("install.json")).unwrap());
    }
}

#[test]
fn removal_deletes_only_verified_connect_configuration_and_preserves_other_files() {
    let fixture = Fixture::ordinary();
    let output = fixture.install();
    private_write(&output.join("unrelated-host.toml"), b"user-setting=1").unwrap();
    private_write(
        &output.join("saved-session-note.txt"),
        b"Explicit session resume metadata retained",
    )
    .unwrap();
    super::remove(&output).unwrap();
    assert!(!output.join("agent.toml").exists());
    assert!(!output.join("install.json").exists());
    assert_eq!(
        std::fs::read(output.join("unrelated-host.toml")).unwrap(),
        b"user-setting=1"
    );
    assert!(output.join("saved-session-note.txt").exists());
    assert!(super::remove(&output).is_err());
}
