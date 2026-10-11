//! Protected installation and process fixtures; no model/account operation.
#![allow(clippy::unwrap_used)]
use connect_protocol::tasks::*;
use connect_server::cli::{
    init::private_write,
    task_access::{codex, GrantFile},
};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn install(f: &super::fixture::Fixture) -> (PathBuf, PathBuf) {
    let grant = ProtectedTaskGrant::new(f.issued.grant.clone(), f.issued.expose_secret().into());
    let file = GrantFile {
        profile: TASK_ACCESS_PROFILE.into(),
        request: TaskAccessRequest::new(TaskAccessCommand::Issue {
            request_id: "mcp-grant".into(),
            definition: GrantDefinition {
                task_id: grant.grant.task_id.clone(),
                binding: grant.grant.binding.clone(),
                operations: grant.grant.operations.clone(),
                expires_at: grant.grant.expires_at,
            },
        }),
        grant: Some(grant),
    };
    let source = f.directory().join("grant.json");
    private_write(&source, &serde_json::to_vec(&file).unwrap()).unwrap();
    let output = f.directory().join("host");
    codex::export(&source, f.server.mcp_address.unwrap(), &output).unwrap();
    let binary = f.directory().join("fake-codex");
    private_write(&binary, include_bytes!("../fixtures/codex-host.py")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    private_write(
        &output.join("prompt.txt"),
        b"Use $connect to continue this task.",
    )
    .unwrap();
    private_write(&output.join("fixture-mode"), b"complete").unwrap();
    (output, binary)
}
pub fn state(directory: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(directory.join("host-state.json")).unwrap()).unwrap()
}
pub fn mode(directory: &Path, value: &str) {
    connect_server::cli::init::private_replace(&directory.join("fixture-mode"), value.as_bytes())
        .unwrap();
}
pub fn events(directory: &Path) -> String {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "jsonl"))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect()
}

/// A fresh launcher process reloads its installation; no model is invoked.
pub async fn cli(directory: &Path, binary: &Path, first: bool) -> std::process::Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    command
        .args(["task-access", "run-codex", "--installation"])
        .arg(directory)
        .arg("--codex-binary")
        .arg(binary)
        .args(["--timeout-seconds", "15"])
        .kill_on_drop(true);
    if first {
        command
            .arg("--prompt-file")
            .arg(directory.join("prompt.txt"));
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
        .await
        .unwrap()
        .unwrap()
}
