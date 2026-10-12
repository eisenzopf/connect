//! Runnable three-process demo qualification using public clients and app-owned
//! inspection. This command never opens the server database.
use anyhow::{Context, Result};
use connect_client::{ClientConfig, ConnectClient};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;

pub async fn verify(operator_path: &Path) -> Result<()> {
    let mut operator = ConnectClient::open(ClientConfig::load(operator_path)?).await?;
    let health = operator.health().await?;
    operator.close().await;
    let parent = operator_path.parent().unwrap_or(Path::new("."));
    let binary = std::env::current_exe()?
        .parent()
        .context("Missing binary directory")?
        .join("connect-example");
    anyhow::ensure!(
        binary.is_file(),
        "Install connect-example beside connect, or build both workspace binaries"
    );
    let mut children = Vec::new();
    for role in ["reviews", "agent"] {
        let config = parent.join(format!("{role}.toml"));
        anyhow::ensure!(
            config.is_file(),
            "Initialize with --example review-loop before demo verify"
        );
        children.push(
            Command::new(&binary)
                .arg(role)
                .arg("--config")
                .arg(config)
                .arg("--run-for-ms")
                .arg("1200")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()?,
        );
    }
    let second = children.pop().context("Missing agent process")?;
    let first = children.pop().context("Missing app process")?;
    let output = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::try_join!(first.wait_with_output(), second.wait_with_output())
    })
    .await
    .context("Example processes exceeded their local demo deadline")??;
    anyhow::ensure!(output.0.status.success()&&output.1.status.success(),"Demo examples failed; stop existing example workers and run each example directly with the same work files to inspect its error");
    let mut conversations = Vec::new();
    for role in ["reviews", "agent"] {
        let config_path = parent.join(format!("{role}.toml"));
        let state = config_path.with_extension("work.sqlite3");
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new(&binary)
                .arg("inspect")
                .arg("--state")
                .arg(state)
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        anyhow::ensure!(
            output.status.success(),
            "Example checkpoint inspection failed"
        );
        let statistics: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        anyhow::ensure!(
            statistics["business_effects"]
                .as_u64()
                .is_some_and(|n| n >= 1)
                && statistics["accepted_commands"]
                    .as_u64()
                    .is_some_and(|n| n >= 3)
                && statistics["pending_commands"] == 0
                && statistics["cursor_saved"] == true,
            "Example did not complete its durable conversation/reply loop"
        );
        let id = statistics["initiated_conversation"]
            .as_str()
            .context("Example initiated conversation is missing")?;
        let config = ClientConfig::load(&config_path)?;
        let own = config.identity_id.clone();
        let mut client = ConnectClient::open(config).await?;
        let page = client.history(id, None, 100).await?;
        anyhow::ensure!(
            page.messages.iter().any(|m| m.sender == own)
                && page.messages.iter().any(|m| m.sender != own),
            "Native history is missing the initiated request or peer reply"
        );
        client.close().await;
        conversations.push(id.to_owned());
    }
    anyhow::ensure!(
        conversations[0] != conversations[1],
        "Each example must initiate a distinct conversation"
    );
    println!(
        "{}",
        serde_json::to_string(
            &serde_json::json!({"result":"passed","transport":"quic","installation_id":health.installation_id,"independent_examples":2,"initiated_conversations":conversations,"deterministic_agent":true})
        )?
    );
    Ok(())
}
