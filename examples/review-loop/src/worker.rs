//! Each role can initiate, respond and recover using the same native SDK.
use crate::store::WorkStore;
use anyhow::{Context, Result};
use connect_client::{ClientConfig, ConnectClient, Operation, Outcome};
use std::{path::Path, time::Duration};

pub async fn run(
    config_path: &Path,
    state: &Path,
    agent: bool,
    run_for_ms: Option<u64>,
) -> Result<()> {
    let config = ClientConfig::load(config_path)?;
    let mut client = ConnectClient::open(config.clone()).await?;
    let role = if agent { "agent" } else { "reviews" };
    let own = client.resolve(&format!("connect:{role}@local")).await?;
    anyhow::ensure!(
        own.id == config.identity_id,
        "Client configuration identity does not match the authenticated example role"
    );
    let peer = client
        .resolve(if agent {
            "connect:reviews@local"
        } else {
            "connect:agent@local"
        })
        .await?;
    let mut store = WorkStore::open(state, &own.id)?;
    println!("ready role={role} transport=quic deterministic=true");
    let create = store.prepare(
        "startup-create",
        Operation::Create {
            request_id: uuid::Uuid::new_v4().to_string(),
            request: connect_client::conversations::CreateRequest::persistent(vec![peer.id]),
        },
    )?;
    let outcome = if let Some(outcome) = create.outcome {
        outcome
    } else {
        let outcome = client.request(create.operation).await?;
        store.accepted(&create.key, &outcome)?;
        outcome
    };
    let conversation = match outcome {
        Outcome::Conversation(c) => c,
        _ => anyhow::bail!("Invalid create outcome"),
    };
    store.prepare(
        "startup-send",
        Operation::Send {
            request_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation.id,
            message: connect_client::messages::MessageDraft::text_all(if agent {
                "ask_pending_draft"
            } else {
                "review_draft:draft-002"
            }),
        },
    )?;
    let stop = async {
        if let Some(ms) = run_for_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok::<(), std::io::Error>(())
        } else {
            tokio::signal::ctrl_c().await
        }
    };
    tokio::pin!(stop);
    loop {
        for command in store.pending()? {
            let outcome=client.request(command.operation).await.context("Command acceptance may be unknown; restart with the same work state to recover its logical ID")?;
            #[cfg(feature = "test-support")]
            crate::store::fault("after-server-ack");
            store.accepted(&command.key, &outcome)?;
        }
        let cursor = store.cursor()?;
        let operation = Operation::Inbox {
            cursor,
            limit: 100,
            wait_ms: 100,
        };
        let outcome = tokio::select! {result=client.request(operation)=>result?,result=&mut stop=>{result?;break}};
        match outcome {
            Outcome::Inbox { events, cursor } => {
                for event in events {
                    store.handle(&event, &own.id, agent)?;
                }
                store.save_cursor(cursor.as_deref())?;
            }
            _ => anyhow::bail!("Invalid inbox outcome"),
        }
    }
    client.close().await;
    println!("{}", serde_json::to_string(&store.statistics()?)?);
    Ok(())
}
