//! One immutable config, a private ledger and bounded public SDK reconnects.
use super::{bootstrap, decisions, state::AppStore, workflow, Policy};
use anyhow::{Context, Result};
use connect_client::{ClientConfig, ClientError, ConnectClient};
use std::{io::Read, path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

/// All inputs are developer-owned; peer messages cannot replace these paths.
pub struct RunOptions {
    pub config: PathBuf,
    pub policy: PathBuf,
    pub state: PathBuf,
}
/// Run until cancelled. Cancellation preserves original intents and unfinished pages.
pub async fn run(options: RunOptions, stop: CancellationToken) -> Result<()> {
    let config = ClientConfig::load(&options.config)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&options.policy)?
        .take(16385)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 16384, "Policy exceeds app limit");
    let policy: Policy = serde_json::from_slice(&bytes)?;
    policy.validate()?;
    let mut store = AppStore::open(&options.state, &config, &policy)?;
    loop {
        if stop.is_cancelled() {
            return Ok(());
        }
        let session = async {
            let mut client = ConnectClient::open(config.clone()).await?;
            let result = connected(&mut store, &mut client, &policy, &config.identity_id).await;
            client.close().await;
            result
        };
        let result = tokio::select! {result=session=>result,_=stop.cancelled()=>return Ok(())};
        match result {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    error.downcast_ref::<ClientError>(),
                    Some(ClientError::Transport | ClientError::Timeout | ClientError::Cancelled)
                ) =>
            {
                eprintln!("pending transport acceptance; reconnecting with original app state");
                tokio::select! {_=tokio::time::sleep(Duration::from_millis(200))=>{},_=stop.cancelled()=>return Ok(())}
            }
            Err(error) => {
                return Err(error.context(
                    "Connected app stopped; preserve state and inspect pending/rejected work",
                ))
            }
        }
    }
}
async fn connected(
    store: &mut AppStore,
    client: &mut ConnectClient,
    policy: &Policy,
    own_id: &str,
) -> Result<()> {
    let task = loop {
        if let Some(task) = bootstrap::bootstrap(store, client, policy).await? {
            break task;
        }
        eprintln!("pending outgoing question; reconciling original effect");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    anyhow::ensure!(
        task.owner_id == own_id,
        "Authenticated app identity does not match task owner"
    );
    println!(
        "ready role=connected-app transport=quic task={} conversation={}",
        task.id, task.conversation_id
    );
    loop {
        workflow::observe(store, client, &task.id).await?;
        store.flush(client).await?;
        if store.metadata("page")?.is_none() {
            let page = client.inbox(store.cursor()?, 100, 100).await?;
            store.stage(page)?;
        }
        while let Some(event) = store.next_event()? {
            decisions::handle(store, client, policy, &task, &event, own_id)
                .await
                .context("Handle durable app inbox event")?;
            store.flush(client).await?;
        }
        store.finish_page()?;
    }
}
