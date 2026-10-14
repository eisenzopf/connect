//! Synthetic known schema17 state, actual public migration CLI; no historical binary claim.
#![allow(clippy::unwrap_used)]
use connect_server::cli::{first_run, init::private_replace, recovery};
use connect_store::Store;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn binary() -> PathBuf {
    std::env::var_os("CONNECT_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_connect").into())
}
fn cli(
    arguments: &[&str],
    state: &Path,
    backup: Option<&Path>,
    operator: &str,
) -> std::process::Output {
    let mut command = Command::new(binary());
    command.env_clear();
    for name in ["PATH", "HOME", "TMPDIR", "LANG"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.args(arguments);
    if let Some(backup) = backup {
        command
            .arg("--config")
            .arg(state.join(operator))
            .arg("--server-config")
            .arg(state.join("server.toml"))
            .arg("--backup")
            .arg(backup);
    } else {
        command
            .arg("--directory")
            .arg(state)
            .args(["--bind", "127.0.0.1:0"]);
    }
    command.output().unwrap()
}
fn synthetic_seventeen(state: &Path) {
    assert_eq!(
        connect_store::SCHEMA_VERSION,
        18,
        "Review known17→18 fixture if another migration is added"
    );
    let mut setup: first_run::SetupState =
        serde_json::from_slice(&std::fs::read(state.join("setup.json")).unwrap()).unwrap();
    let database = rusqlite::Connection::open(state.join("state.sqlite3")).unwrap();
    database.execute_batch("DROP TABLE connector_observations; DROP TABLE connector_receipt_events; DROP TABLE connector_incoming; DROP TABLE connector_voice_observations; PRAGMA user_version=17;").unwrap();
    setup.schema = 17;
    private_replace(
        &state.join("setup.json"),
        &serde_json::to_vec_pretty(&setup).unwrap(),
    )
    .unwrap();
}
fn original_receipts(state: &Path) -> Vec<(String, String, String)> {
    let database = rusqlite::Connection::open(state.join("state.sqlite3")).unwrap();
    let mut statement = database
        .prepare(
            "SELECT actor,id,result FROM requests WHERE id LIKE 'first-run-%' ORDER BY actor,id",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    rows
}
#[test]
fn public_known_seventeen_upgrade_and_interrupted_metadata_reconcile_preserve_authority() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/gate5-first-run-migration-cli");
    std::fs::create_dir_all(&root).unwrap();
    let directory = tempfile::tempdir_in(root).unwrap();
    for interrupted in [false, true] {
        let state = directory
            .path()
            .join(if interrupted { "interrupted" } else { "normal" });
        assert!(cli(&["setup", "demo"], &state, None, "operator.toml")
            .status
            .success());
        synthetic_seventeen(&state);
        let original = original_receipts(&state);
        assert!(!original.is_empty());
        let protected = [
            "human.toml",
            "agent.toml",
            "operator.toml",
            "secret-references.toml",
        ]
        .map(|name| (name, std::fs::read(state.join(name)).unwrap()));
        let backup = directory.path().join(if interrupted {
            "old-interrupted"
        } else {
            "old-normal"
        });
        recovery::backup(
            &state.join("server.toml"),
            &state.join("operator.toml"),
            &backup,
        )
        .unwrap();
        assert!(!cli(&["migrate"], &state, Some(&backup), "human.toml")
            .status
            .success());
        let metadata = std::fs::read(state.join("setup.json")).unwrap();
        let mut altered: first_run::SetupState = serde_json::from_slice(&metadata).unwrap();
        altered.provider_mode = "unreviewed-change".into();
        private_replace(
            &state.join("setup.json"),
            &serde_json::to_vec_pretty(&altered).unwrap(),
        )
        .unwrap();
        assert!(!cli(&["migrate"], &state, Some(&backup), "operator.toml")
            .status
            .success());
        assert_eq!(
            Store::inspect_snapshot(&state.join("state.sqlite3"))
                .unwrap()
                .schema,
            17
        );
        private_replace(&state.join("setup.json"), &metadata).unwrap();

        assert_eq!(
            Store::inspect_snapshot(&state.join("state.sqlite3"))
                .unwrap()
                .schema,
            17
        );
        let selected = if interrupted {
            let manifest = recovery::verified_manifest(&backup).unwrap();
            // Simulate precisely DB commit then process interruption before setup publication.
            Store::migrate(
                &state.join("state.sqlite3"),
                &backup.join("state.sqlite3"),
                &manifest.snapshot,
            )
            .unwrap();
            assert!(first_run::inspect(&state).is_err());
            let setup_bytes = std::fs::read(state.join("setup.json")).unwrap();
            assert!(!cli(&["migrate"], &state, Some(&backup), "operator.toml")
                .status
                .success());
            assert_eq!(
                std::fs::read(state.join("setup.json")).unwrap(),
                setup_bytes
            );
            let current = directory.path().join("exact-current");
            recovery::backup(
                &state.join("server.toml"),
                &state.join("operator.toml"),
                &current,
            )
            .unwrap();
            assert!(!cli(&["migrate"], &state, Some(&current), "human.toml")
                .status
                .success());
            assert_eq!(
                std::fs::read(state.join("setup.json")).unwrap(),
                setup_bytes
            );
            current
        } else {
            backup
        };
        assert!(cli(&["migrate"], &state, Some(&selected), "operator.toml")
            .status
            .success());
        assert_eq!(
            first_run::inspect(&state).unwrap().schema,
            connect_store::SCHEMA_VERSION
        );
        assert!(
            original_receipts(&state) == original,
            "Original receipts changed during migration"
        );
        assert!(protected
            .iter()
            .all(|(name, bytes)| std::fs::read(state.join(name)).unwrap() == *bytes));
        assert!(cli(&["setup", "demo"], &state, None, "operator.toml")
            .status
            .success());
        assert!(
            original_receipts(&state) == original,
            "Original native mutation receipts changed on replay"
        );
        let database = rusqlite::Connection::open(state.join("state.sqlite3")).unwrap();
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}
