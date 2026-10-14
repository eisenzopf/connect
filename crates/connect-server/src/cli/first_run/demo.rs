//! Packaged provider-free rehearsal uses public clients; persistence includes restart.
use crate::{config::ServerConfig, runtime::RunningServer};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{Operation, Outcome};
use sha2::{Digest, Sha256};
use std::path::Path;

pub async fn verify(directory: &Path) -> Result<()> {
    let directory = directory.canonicalize()?;
    let state = super::inspect(&directory)?;
    let config = ServerConfig::load(&directory.join(&state.server_config))?;
    anyhow::ensure!(
        config.sip.is_none() && config.webrtc.is_none() && config.mcp.is_none(),
        "Local setup demo requires its provider-free profile"
    );
    let server = RunningServer::start(config.clone())
        .await
        .context("Start first-run native server")?;
    let result = exchange(&directory).await;
    server.shutdown().await?;
    wait_released(config.bind).await?;
    let (conversation, artifacts) = result?;
    let server = RunningServer::start(config.clone())
        .await
        .context("Restart first-run native server")?;
    let result = async {
        for name in [&state.human_config, &state.agent_config] {
            let mut client =
                ConnectClient::open(ClientConfig::load(&directory.join(name))?).await?;
            let page = client.history(&conversation, None, 100).await?;
            anyhow::ensure!(
                page.messages
                    .iter()
                    .any(|m| m.body == "Hello from the local human")
                    && page
                        .messages
                        .iter()
                        .any(|m| m.body == "Reply from the local agent"),
                "Two-way history did not survive restart"
            );
            for artifact in &artifacts {
                let chunk = client
                    .artifact_get(&conversation, artifact, 0, 1024)
                    .await?;
                anyhow::ensure!(
                    chunk.data == STANDARD.encode(b"Connect first-run file\n"),
                    "File did not survive restart"
                );
            }
            client.close().await;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    server.shutdown().await?;
    wait_released(config.bind).await?;
    result?;
    println!(
        "{}",
        serde_json::json!({"profile":"connect-first-run-proof/1","transport":"native-uctp-quic","messages_each_direction":true,"files_each_direction":true,"restart_history":true,"native_pairing":true,"provider_requests":0,"model_used":false,"tasks_required":false,"conversation_id":conversation})
    );
    Ok(())
}
async fn wait_released(address: std::net::SocketAddr) -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match std::net::UdpSocket::bind(address) {
                Ok(socket) => {
                    drop(socket);
                    return Ok::<_, std::io::Error>(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .context("Native port did not release within five seconds")??;
    Ok(())
}
async fn exchange(directory: &Path) -> Result<(String, Vec<String>)> {
    let human_config = ClientConfig::load(&directory.join("human.toml"))?;
    let agent_config = ClientConfig::load(&directory.join("agent.toml"))?;
    let mut human = ConnectClient::open(human_config).await?;
    let mut agent = ConnectClient::open(agent_config.clone()).await?;
    let conversation = human
        .create(
            "first-run-conversation",
            vec![agent_config.identity_id.clone()],
        )
        .await?;
    human
        .send(
            "first-run-message",
            &conversation.id,
            "Hello from the local human",
        )
        .await?;
    agent
        .send(
            "first-run-reply",
            &conversation.id,
            "Reply from the local agent",
        )
        .await?;
    let mut artifacts = Vec::new();
    for (name, client) in [("human", &mut human), ("agent", &mut agent)] {
        let body = b"Connect first-run file\n";
        let hash = hex::encode(Sha256::digest(body));
        let artifact = client
            .artifact_begin(
                &format!("first-run-{name}-file"),
                &conversation.id,
                &format!("{name}.txt"),
                "text/plain",
                body.len() as u64,
                &hash,
            )
            .await?;
        client
            .artifact_append(
                &format!("first-run-{name}-append"),
                &conversation.id,
                &artifact.id,
                0,
                &STANDARD.encode(body),
            )
            .await?;
        client
            .artifact_finish(
                &format!("first-run-{name}-finish"),
                &conversation.id,
                &artifact.id,
            )
            .await?;
        artifacts.push(artifact.id);
    }
    // Save/reuse exact start intent; a fresh run receives a new ten-minute challenge.
    let logical = uuid::Uuid::new_v4().simple().to_string();
    let Outcome::ContactPairing(pairing) = human
        .request(Operation::ContactPairingStart {
            request_id: format!("pair-{logical}"),
            peer_identity_id: agent_config.identity_id,
            expires_at: chrono::Utc::now().timestamp() + 600,
        })
        .await?
    else {
        anyhow::bail!("Unexpected pairing response")
    };
    let Outcome::ContactPairing(done) = agent
        .request(Operation::ContactPairingComplete {
            request_id: format!("complete-{logical}"),
            pairing_id: pairing.id,
            challenge: pairing.challenge.context("Missing protected challenge")?,
        })
        .await?
    else {
        anyhow::bail!("Unexpected pairing result")
    };
    anyhow::ensure!(
        done.state == connect_protocol::contacts::OwnershipState::VerifiedNative,
        "Pairing did not verify native ownership"
    );
    human.close().await;
    agent.close().await;
    Ok((conversation.id, artifacts))
}
