//! Public CLI verifies the current backup and explicit incompatible-state boundary.
#![allow(clippy::unwrap_used)]
use connect_server::cli::{first_run, recovery};
use connect_store::Store;
use std::{
    path::Path,
    process::{Command, Output},
};

fn cli(arguments: &[&std::ffi::OsStr]) -> Output {
    let binary = std::env::var_os("CONNECT_TEST_BINARY")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_connect").into());
    let mut command = Command::new(binary);
    command.env_clear();
    for name in ["PATH", "HOME", "TMPDIR", "LANG"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.args(arguments).output().unwrap()
}

fn setup(path: &Path) {
    let result = cli(&[
        "setup".as_ref(),
        "init".as_ref(),
        "--directory".as_ref(),
        path.as_os_str(),
        "--bind".as_ref(),
        "127.0.0.1:0".as_ref(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn current_backup_restores_and_incompatible_state_requires_a_fresh_installation() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("current");
    setup(&state);
    let initialized = first_run::inspect(&state).unwrap();
    assert_eq!(initialized.schema, connect_store::SCHEMA_VERSION);
    let original_installation = Store::inspect_snapshot(&state.join("state.sqlite3"))
        .unwrap()
        .installation_id;
    let backup = directory.path().join("backup");
    recovery::backup(
        &state.join("server.toml"),
        &state.join("operator.toml"),
        &backup,
    )
    .unwrap();
    let manifest = recovery::verified_manifest(&backup).unwrap();
    let restored = directory.path().join("restored");
    let result = cli(&[
        "restore".as_ref(),
        "--backup".as_ref(),
        backup.as_os_str(),
        "--directory".as_ref(),
        restored.as_os_str(),
        "--bind".as_ref(),
        "127.0.0.1:0".as_ref(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        Store::inspect_snapshot(&restored.join("state.sqlite3")).unwrap(),
        manifest.snapshot
    );
    assert_eq!(
        first_run::inspect(&restored).unwrap().schema,
        connect_store::SCHEMA_VERSION
    );
    assert_eq!(
        recovery::verified_manifest(&backup).unwrap().snapshot,
        manifest.snapshot
    );

    // This tests refusal by declared schema, not historical binary compatibility.
    let c = rusqlite::Connection::open(state.join("state.sqlite3")).unwrap();
    // A synthetic older-version header needs its historical receipt-table shape
    // for backup inspection. The current runtime still refuses that state.
    c.execute_batch("CREATE TABLE requests(actor TEXT,id TEXT,digest TEXT,operation TEXT,resources TEXT,result TEXT,PRIMARY KEY(actor,id))").unwrap();
    c.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE; PRAGMA user_version=18;",
    )
    .unwrap();
    drop(c);
    let mut metadata: first_run::SetupState =
        serde_json::from_slice(&std::fs::read(state.join("setup.json")).unwrap()).unwrap();
    metadata.schema = 18;
    connect_server::cli::init::private_replace(
        &state.join("setup.json"),
        &serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    let previous = directory.path().join("previous-backup");
    recovery::backup(
        &state.join("server.toml"),
        &state.join("operator.toml"),
        &previous,
    )
    .unwrap();
    let files = [
        "state.sqlite3",
        "setup.json",
        "operator.toml",
        "human.toml",
        "agent.toml",
    ]
    .map(|name| (name, std::fs::read(state.join(name)).unwrap()));
    let result = cli(&[
        "migrate".as_ref(),
        "--config".as_ref(),
        state.join("operator.toml").as_os_str(),
        "--server-config".as_ref(),
        state.join("server.toml").as_os_str(),
        "--backup".as_ref(),
        previous.as_os_str(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("does not convert historical state"));
    let refused = directory.path().join("refused-restore");
    let result = cli(&[
        "restore".as_ref(),
        "--backup".as_ref(),
        previous.as_os_str(),
        "--directory".as_ref(),
        refused.as_os_str(),
    ]);
    assert!(!result.status.success());
    assert!(!refused.exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Incompatible backup schema"));
    assert!(files
        .iter()
        .all(|(name, bytes)| std::fs::read(state.join(name)).unwrap() == *bytes));
    let fresh = directory.path().join("fresh-gate6");
    setup(&fresh);
    assert_ne!(
        Store::inspect_snapshot(&fresh.join("state.sqlite3"))
            .unwrap()
            .installation_id,
        original_installation
    );
    assert!(files
        .iter()
        .all(|(name, bytes)| std::fs::read(state.join(name)).unwrap() == *bytes));
}
