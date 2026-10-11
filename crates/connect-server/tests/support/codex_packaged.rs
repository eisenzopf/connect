//! Actual installed CLI through the packaged launcher, with no account inference.
#![allow(clippy::unwrap_used)]
use connect_server::cli::init::{private_directory, private_write};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub fn runtime(directory: &Path, binary: &Path, provider: &str) -> PathBuf {
    let codex_home = directory.join("isolated-codex-home");
    let user_home = directory.join("isolated-user-home");
    private_directory(&codex_home).unwrap();
    private_directory(&user_home).unwrap();
    let shim = directory.join("offline-runtime.py");
    private_write(
        &shim,
        include_bytes!("../fixtures/codex-packaged-runtime.py"),
    )
    .unwrap();
    private_write(
        &shim.with_extension("json"),
        &serde_json::to_vec(&json!({
            "binary":binary, "provider":provider, "codex_home":codex_home, "user_home":user_home,
        }))
        .unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    private_write(
        &directory.join("prompt.txt"),
        b"Use $connect to continue this existing delegated task.",
    )
    .unwrap();
    shim
}

pub async fn run(directory: &Path, binary: &Path, first: bool) -> std::process::Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    command
        .args(["task-access", "run-codex", "--installation"])
        .arg(directory)
        .arg("--codex-binary")
        .arg(binary)
        .args(["--timeout-seconds", "30"])
        .env_clear()
        .kill_on_drop(true);
    for key in ["PATH", "LANG", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    if first {
        command
            .arg("--prompt-file")
            .arg(directory.join("prompt.txt"));
    }
    tokio::time::timeout(Duration::from_secs(45), command.output())
        .await
        .unwrap()
        .unwrap()
}

pub fn state(directory: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(directory.join("host-state.json")).unwrap()).unwrap()
}

pub fn succeeded(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "host-turn-completed");
    report
}
