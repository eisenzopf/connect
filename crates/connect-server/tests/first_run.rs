//! Isolated provider-free setup acceptance; no owner paths or model account.
#![allow(clippy::unwrap_used)]
use connect_server::cli::first_run;
#[test]
fn public_fresh_setup_commands_emit_one_json_document_and_legacy_init_keeps_status() {
    let binary = std::env::var_os("CONNECT_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_connect").into());
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/gate5-first-run-cli");
    std::fs::create_dir_all(&root).unwrap();
    let directory = tempfile::tempdir_in(root).unwrap();
    let execute = |args: &[&str], state: &std::path::Path| {
        let output = std::process::Command::new(&binary)
            .args(args)
            .arg("--directory")
            .arg(state)
            .args(["--bind", "127.0.0.1:0"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let state = directory.path().join("fresh");
    let first: first_run::SetupState =
        serde_json::from_slice(&execute(&["setup", "init"], &state)).unwrap();
    let repeated: first_run::SetupState =
        serde_json::from_slice(&execute(&["setup", "init"], &state)).unwrap();
    assert_eq!(first, repeated);
    let demo: serde_json::Value = serde_json::from_slice(&execute(
        &["setup", "demo"],
        &directory.path().join("fresh-demo"),
    ))
    .unwrap();
    assert_eq!(demo["profile"], "connect-first-run-proof/1");
    assert_eq!(demo["restart_history"], true);
    assert_eq!(demo["provider_requests"], 0);
    let legacy = execute(&["init"], &directory.path().join("legacy"));
    assert!(String::from_utf8_lossy(&legacy).starts_with("Initialized local installation:"));
}
#[tokio::test]
async fn provider_free_native_demo_repeats_and_preserves_restart_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh");
    let state = first_run::initialize(&path, "127.0.0.1:0".parse().unwrap()).unwrap();
    assert_ne!(state.bind.port(), 0);
    assert!(!state.tasks_required);
    let old = std::fs::read(path.join("agent.toml")).unwrap();
    assert_eq!(
        state,
        first_run::initialize(&path, "127.0.0.1:0".parse().unwrap()).unwrap()
    );
    assert_eq!(old, std::fs::read(path.join("agent.toml")).unwrap());
    first_run::verify(&path).await.unwrap();
    first_run::verify(&path).await.unwrap();
}
#[test]
fn conflicts_partial_state_and_permissions_are_actionable_and_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("occupied");
    let port = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    assert!(first_run::initialize(&path, port.local_addr().unwrap())
        .unwrap_err()
        .to_string()
        .contains("occupied"));
    assert!(!path.exists());
    let partial = dir.path().join("partial");
    std::fs::create_dir(&partial).unwrap();
    std::fs::write(partial.join("state.sqlite3"), b"preserved").unwrap();
    assert!(first_run::initialize(&partial, "127.0.0.1:0".parse().unwrap()).is_err());
    assert_eq!(
        std::fs::read(partial.join("state.sqlite3")).unwrap(),
        b"preserved"
    );
    let interrupted = dir.path().join("interrupted");
    std::fs::create_dir(dir.path().join(".interrupted.connect-initializing")).unwrap();
    assert!(
        first_run::initialize(&interrupted, "127.0.0.1:0".parse().unwrap())
            .unwrap_err()
            .to_string()
            .contains("Interrupted")
    );
    let fresh = dir.path().join("fresh");
    first_run::initialize(&fresh, "127.0.0.1:0".parse().unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            fresh.join("agent.toml"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(first_run::inspect(&fresh).is_err());
    }
}

#[tokio::test]
async fn selected_agent_export_keeps_normal_key_and_preserves_existing_host_config() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("setup");
    first_run::initialize(&directory, "127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mcp_bind = listener.local_addr().unwrap();
    drop(listener);
    let output = temp.path().join("codex");
    first_run::Command::AgentInstall {
        directory: directory.clone(),
        mcp_bind,
        output: output.clone(),
    }
    .execute()
    .await
    .unwrap();
    let old = std::fs::read(directory.join("agent.toml")).unwrap();
    let host = output.join("unrelated.toml");
    std::fs::write(&host, b"preserve-host-config").unwrap();
    assert!(first_run::Command::AgentInstall {
        directory: directory.clone(),
        mcp_bind,
        output: output.clone()
    }
    .execute()
    .await
    .is_err());
    assert_eq!(old, std::fs::read(directory.join("agent.toml")).unwrap());
    assert_eq!(std::fs::read(host).unwrap(), b"preserve-host-config");
    let client = connect_client::ClientConfig::load(&output.join("agent.toml")).unwrap();
    assert_eq!(
        client.credential_id,
        connect_client::ClientConfig::load(&directory.join("agent.toml"))
            .unwrap()
            .credential_id
    );
    let store =
        connect_store::Store::open(&directory.join("state.sqlite3"), Default::default()).unwrap();
    let actor = store.authenticate(&client.credential).unwrap();
    assert!(actor.permits("send"));
    assert!(!actor.permits("admin"));
    assert!(!actor.permits("task_create"));
}

#[test]
fn first_run_backup_restores_human_config_and_metadata_only_secret_references() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    first_run::initialize(&source, "127.0.0.1:0".parse().unwrap()).unwrap();
    let backup = tmp.path().join("backup");
    connect_server::cli::recovery::backup(
        &source.join("server.toml"),
        &source.join("operator.toml"),
        &backup,
    )
    .unwrap();
    let restored = tmp.path().join("restored");
    let bind = "127.0.0.1:54321".parse().unwrap();
    connect_server::cli::recovery::restore(&backup, &restored, bind, None).unwrap();
    assert_eq!(first_run::inspect(&restored).unwrap().bind, bind);
    assert_eq!(
        std::fs::read(source.join("secret-references.toml")).unwrap(),
        std::fs::read(restored.join("secret-references.toml")).unwrap()
    );
    let human = connect_client::ClientConfig::load(&restored.join("human.toml")).unwrap();
    assert_eq!(human.endpoint, bind);
    connect_store::Store::open(&restored.join("state.sqlite3"), Default::default())
        .unwrap()
        .authenticate(&human.credential)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in ["human.toml", "setup.json", "secret-references.toml"] {
            assert_eq!(
                std::fs::metadata(restored.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
    }
}
