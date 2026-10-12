//! Fresh local installation exports only a delegated connection and workflow.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_protocol::tasks::*;
use connect_server::cli::{
    init::private_write,
    task_access::{Command, GrantFile},
};
use connect_store::Store;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

fn input(f: &fixture::Fixture, path: &Path, request_id: &str) {
    let grant = ProtectedTaskGrant::new(f.issued.grant.clone(), f.issued.expose_secret().into());
    let file = GrantFile {
        profile: TASK_ACCESS_PROFILE.into(),
        request: TaskAccessRequest::new(TaskAccessCommand::Issue {
            request_id: request_id.into(),
            definition: GrantDefinition {
                task_id: grant.grant.task_id.clone(),
                binding: grant.grant.binding.clone(),
                operations: grant.grant.operations.clone(),
                expires_at: grant.grant.expires_at,
            },
        }),
        grant: Some(grant),
    };
    private_write(path, &serde_json::to_vec(&file).unwrap()).unwrap();
}

#[tokio::test]
async fn fresh_export_reuses_exact_grant_for_real_tools_and_cannot_renew_or_overwrite_it() {
    for kind in [InvocationKind::Native, InvocationKind::HostSession] {
        let mut f = fixture::Fixture::start_independent().await;
        let request_id = if kind == InvocationKind::HostSession {
            f.binding.kind = kind;
            f.binding.adapter = "connect-codex".into();
            f.binding.external_id = "protected-installed-session".into();
            f.server
                .service
                .register_task_binding(
                    f.owner.credential_id.clone(),
                    f.task.id.clone(),
                    f.binding.clone(),
                )
                .await
                .unwrap();
            f.issued = f
                .server
                .service
                .issue_task_grant(
                    f.owner.credential_id.clone(),
                    "codex-host-grant".into(),
                    GrantDefinition {
                        task_id: f.task.id.clone(),
                        binding: f.binding.clone(),
                        operations: vec![TaskPermission::History],
                        expires_at: chrono::Utc::now().timestamp() + 120,
                    },
                )
                .await
                .unwrap();
            "codex-host-grant"
        } else {
            "mcp-grant"
        };
        let source = f.directory().join("issued.json");
        input(&f, &source, request_id);
        let output = f.directory().join("installed-host");
        let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
        Command::ExportCodex {
            grant_file: source.clone(),
            mcp_address: f.server.mcp_address.unwrap(),
            output: output.clone(),
        }
        .execute()
        .await
        .unwrap();
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        let manifest_bytes = std::fs::read(output.join("install.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest["task_id"], f.task.id);
        assert_eq!(manifest["grant_id"], f.issued.grant.id);
        assert_eq!(manifest["status"], "exported-not-refreshed");
        let exported_bytes = std::fs::read(output.join("task-grant.json")).unwrap();
        let exported: GrantFile = serde_json::from_slice(&exported_bytes).unwrap();
        assert_eq!(
            serde_json::to_value(&exported.request).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&source).unwrap()).unwrap()
                ["request"]
        );
        let granted = exported.grant.unwrap();
        assert_eq!(granted.grant, f.issued.grant);
        assert_eq!(granted.expose_secret(), f.issued.expose_secret());
        for (name, expected) in manifest["skill_files"].as_object().unwrap() {
            let bytes = std::fs::read(output.join(".agents/skills/connect").join(name)).unwrap();
            assert_eq!(
                hex::encode(Sha256::digest(&bytes)),
                expected.as_str().unwrap()
            );
            assert!(!String::from_utf8(bytes)
                .unwrap()
                .contains(&f.owner.credential));
        }
        let config = std::fs::read_to_string(output.join("mcp-override.txt")).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        let mcp_config = &parsed["mcp_servers"]["connect_task"];
        assert_eq!(
            mcp_config["bearer_token_env_var"].as_str(),
            Some("CONNECT_TASK_GRANT")
        );
        assert_eq!(mcp_config["tool_timeout_sec"].as_integer(), Some(10));
        assert!(!config.contains(granted.expose_secret()));
        assert!(!config.contains(&f.owner.credential));
        assert!(!String::from_utf8(manifest_bytes.clone())
            .unwrap()
            .contains(granted.expose_secret()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&output).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for name in ["install.json", "task-grant.json", "mcp-override.txt"] {
                assert_eq!(
                    std::fs::metadata(output.join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        let client =
            mcp::client_from_grant(f.server.mcp_address.unwrap(), granted.expose_secret()).await;
        let tools = client.list_tools(None).await.unwrap().tools;
        assert!(tools.iter().any(|t| t.name == "connect_recall"));
        assert!(tools.iter().all(|t| t.name != "connect_send"));
        let status = mcp::success(mcp::call(&client, "connect_task_status", json!({})).await);
        assert_eq!(status["data"]["id"], f.task.id);
        f.server
            .service
            .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
            .await
            .unwrap();
        assert!(client.list_tools(None).await.is_err());
        assert!(Command::ExportCodex {
            grant_file: source,
            mcp_address: f.server.mcp_address.unwrap(),
            output: output.clone(),
        }
        .execute()
        .await
        .is_err());
        assert_eq!(
            std::fs::read(output.join("task-grant.json")).unwrap(),
            exported_bytes
        );
        assert_eq!(
            std::fs::read(output.join("install.json")).unwrap(),
            manifest_bytes
        );
        client.cancel().await.unwrap();
        f.close().await;
    }
}

#[tokio::test]
async fn provider_pending_forged_and_public_files_cannot_create_local_installations() {
    let f = fixture::Fixture::start_independent().await;
    let source = f.directory().join("original.json");
    input(&f, &source, "mcp-grant");
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let mut pending = original.clone();
    pending["grant"] = serde_json::Value::Null;
    let mut forged = original.clone();
    forged["grant"]["grant"]["task_id"] = json!("another-task");
    let mut provider = original.clone();
    provider["grant"]["grant"]["binding"]["kind"] = json!("provider_call");
    provider["request"]["command"]["definition"]["binding"]["kind"] = json!("provider_call");
    for (index, value) in [pending, forged, provider].iter().enumerate() {
        let source = f.directory().join(format!("denied-{index}.json"));
        private_write(&source, &serde_json::to_vec(value).unwrap()).unwrap();
        let output = f.directory().join(format!("denied-{index}"));
        assert!(Command::ExportCodex {
            grant_file: source,
            mcp_address: f.server.mcp_address.unwrap(),
            output: output.clone(),
        }
        .execute()
        .await
        .is_err());
        assert!(!output.exists());
    }
    let output = f.directory().join("non-loopback");
    assert!(Command::ExportCodex {
        grant_file: source.clone(),
        mcp_address: "192.0.2.1:8193".parse().unwrap(),
        output: output.clone(),
    }
    .execute()
    .await
    .is_err());
    assert!(!output.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
        let output = f.directory().join("public-input");
        assert!(Command::ExportCodex {
            grant_file: source,
            mcp_address: f.server.mcp_address.unwrap(),
            output: output.clone(),
        }
        .execute()
        .await
        .is_err());
        assert!(!output.exists());
    }
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.close().await;
}
