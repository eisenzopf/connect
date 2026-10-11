//! Opt-in actual Codex skill/MCP discovery; no inference/account qualification.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_runtime.rs"]
mod runtime;
use connect_protocol::tasks::*;
use connect_server::cli::{
    init::private_write,
    task_access::{codex, GrantFile},
};
use connect_store::Store;
use serde_json::{json, Value};
use std::path::PathBuf;

async fn thread(runtime: &mut runtime::Runtime, directory: &std::path::Path) -> String {
    let response = runtime
        .ask("thread/start", json!({"cwd":directory,"ephemeral":true}))
        .await
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "thread creation failed: {response}"
    );
    response["result"]["thread"]["id"].as_str().unwrap().into()
}
async fn tool(runtime: &mut runtime::Runtime, thread: &str, name: &str, arguments: Value) -> Value {
    runtime
        .ask(
            "mcpServer/tool/call",
            json!({
                "threadId":thread,"server":"connect_task","tool":name,"arguments":arguments
            }),
        )
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires explicitly selected installed Codex; no model turn is executed"]
async fn installed_runtime_discovers_skill_and_real_mcp_with_current_authority() {
    let binary = PathBuf::from(
        std::env::var_os("CONNECT_TEST_CODEX_BINARY")
            .expect("Set CONNECT_TEST_CODEX_BINARY to the actual installed Codex executable"),
    );
    let f = fixture::Fixture::start_permitted(
        true,
        vec![TaskPermission::History, TaskPermission::Inbox],
    )
    .await;
    let source = f.directory().join("runtime-grant.json");
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
    private_write(&source, &serde_json::to_vec(&file).unwrap()).unwrap();
    let directory = f.directory().join("actual-runtime");
    codex::export(&source, f.server.mcp_address.unwrap(), &directory).unwrap();
    let directory = std::fs::canonicalize(directory).unwrap();
    let mut runtime = runtime::Runtime::start(&binary, &directory, f.issued.expose_secret())
        .await
        .unwrap();
    let skills = runtime
        .ask(
            "skills/list",
            json!({"cwds":[directory],"forceReload":true}),
        )
        .await
        .unwrap();
    assert!(
        skills.get("error").is_none(),
        "skill discovery failed: {skills}"
    );
    let entry = &skills["result"]["data"][0];
    assert_eq!(entry["errors"], json!([]));
    let found = entry["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "connect")
        .unwrap();
    assert_eq!(found["enabled"], true);
    assert_eq!(
        PathBuf::from(found["path"].as_str().unwrap()),
        directory.join(".agents/skills/connect/SKILL.md")
    );
    let id = thread(&mut runtime, &directory).await;
    let inventory = runtime
        .ask(
            "mcpServerStatus/list",
            json!({"threadId":id,"serverName":"connect_task","detail":"toolsAndAuthOnly"}),
        )
        .await
        .unwrap();
    assert!(
        inventory.get("error").is_none(),
        "MCP discovery failed: {inventory}"
    );
    let servers = inventory["result"]["data"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["name"], "connect_task");
    assert_eq!(servers[0]["toolsError"], Value::Null);
    let tools = servers[0]["tools"].as_object().unwrap();
    assert!(tools.contains_key("connect_task_status"));
    assert!(tools.contains_key("connect_task_wait"));
    assert!(!tools.contains_key("connect_task_cancel"));
    let status = tool(&mut runtime, &id, "connect_task_status", json!({})).await;
    assert_eq!(
        status["result"]["structuredContent"]["data"]["id"],
        f.task.id
    );
    let inbox = tool(
        &mut runtime,
        &id,
        "connect_task_wait",
        json!({"after":null,"limit":100,"wait_ms":0}),
    )
    .await;
    let cursor = inbox["result"]["structuredContent"]["data"]["next"].clone();
    let pending = tool(
        &mut runtime,
        &id,
        "connect_task_wait",
        json!({"after":cursor,"limit":100,"wait_ms":0}),
    )
    .await;
    assert_eq!(pending["result"]["isError"], false);
    assert_eq!(pending["result"]["_meta"]["connect"]["pending"], true);
    assert_eq!(
        pending["result"]["structuredContent"]["data"]["events"],
        json!([])
    );
    assert_eq!(
        pending["result"]["structuredContent"]["data"]["next"],
        cursor
    );
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let denied = tool(
        &mut runtime,
        &id,
        "connect_task_cancel",
        json!({"request_id":"not-delegated"}),
    )
    .await;
    assert!(denied.get("error").is_some() || denied["result"]["isError"] == true);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    runtime.stop().await;
    // A fresh runtime uses the same installed grant/task; this is not proof of
    // exact model-session resume or instruction following.
    let mut runtime = runtime::Runtime::start(&binary, &directory, f.issued.expose_secret())
        .await
        .unwrap();
    let id = thread(&mut runtime, &directory).await;
    let status = tool(&mut runtime, &id, "connect_task_status", json!({})).await;
    assert_eq!(
        status["result"]["structuredContent"]["data"]["id"],
        f.task.id
    );
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let denied = tool(&mut runtime, &id, "connect_task_status", json!({})).await;
    assert!(denied.get("error").is_some() || denied["result"]["isError"] == true);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    runtime.stop().await;
    f.close().await;
}
