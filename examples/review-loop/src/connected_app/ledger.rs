//! Exact intents survive uncertainty; known rejections remain visible.
use super::{fault, observations, state::AppStore, workflow};
use anyhow::{ensure, Context, Result};
use connect_client::{ClientError, ConnectClient, Operation, Outcome};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn insert(c: &Connection, key: &str, operation: &Operation) -> Result<()> {
    let count: u64 = c.query_row("SELECT COUNT(*) FROM app_commands", [], |r| r.get(0))?;
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_commands WHERE key=?1)",
        [key],
        |r| r.get(0),
    )?;
    ensure!(
        exists || count < 10000,
        "App command ledger full; retain state for maintenance"
    );
    c.execute(
        "INSERT OR IGNORE INTO app_commands(key,operation) VALUES(?1,?2)",
        params![key, serde_json::to_string(operation)?],
    )?;
    Ok(())
}
pub(super) async fn execute(
    store: &mut AppStore,
    client: &mut ConnectClient,
    key: &str,
    operation: Operation,
) -> Result<Outcome> {
    insert(&store.connection, key, &operation)?;
    let saved: (String, Option<String>, Option<String>) = store.connection.query_row(
        "SELECT operation,outcome,rejection FROM app_commands WHERE key=?1",
        [key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    ensure!(
        saved.2.is_none(),
        "App command has a known rejection; explicit reconciliation required"
    );
    let original: Operation = serde_json::from_str(&saved.0)?;
    // Replay even accepted intents on restart: the server rechecks current authority.
    let outcome = match client.request(original.clone()).await {
        Ok(outcome) => outcome,
        Err(error) => {
            if let ClientError::Application(ref e) = error {
                store.connection.execute(
                    "UPDATE app_commands SET rejection=?1 WHERE key=?2",
                    params![e.code, key],
                )?;
            }
            return Err(error.into());
        }
    };
    fault("after-ack");
    if key.starts_with("outgoing-artifact:") {
        fault(&format!(
            "after-{}-ack",
            key.split(':').nth(1).unwrap_or("artifact")
        ));
    }
    if key.starts_with("task:intake:") {
        fault("after-intake-task-ack");
    }
    if key.starts_with("consume:") {
        fault("after-consume-ack");
    }
    if key.starts_with("response:") {
        fault("after-response-ack");
    }
    if let Some(prior) = saved.1 {
        let prior: Outcome = serde_json::from_str(&prior)?;
        observations::verify(client, &original, &prior, &outcome)
            .await
            .context("Verify observation without rewriting initial receipt")?;
        if prior != outcome {
            store.connection.execute(
                "INSERT OR IGNORE INTO app_observations(command_key,outcome) VALUES(?1,?2)",
                params![key, serde_json::to_string(&outcome)?],
            )?;
        }
    } else {
        store.connection.execute(
            "UPDATE app_commands SET outcome=?1 WHERE key=?2",
            params![serde_json::to_string(&outcome)?, key],
        )?;
    }
    Ok(outcome)
}
impl AppStore {
    pub(super) fn accepted(&self, key: &str) -> Result<Option<Outcome>> {
        let s: Option<String> = self
            .connection
            .query_row(
                "SELECT outcome FROM app_commands WHERE key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        s.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    fn rows(&self, sql: &str) -> Result<Vec<(String, String)>> {
        let mut query = self.connection.prepare(sql)?;
        let rows = query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub(super) async fn flush(&mut self, client: &mut ConnectClient) -> Result<()> {
        let pending=self.rows("SELECT key,operation FROM app_commands WHERE key LIKE 'response:%' AND outcome IS NULL AND rejection IS NULL ORDER BY sequence LIMIT 16")?;
        for (key, json) in pending {
            let operation: Operation = serde_json::from_str(&json)?;
            let invoke = matches!(&operation,Operation::Task {request} if matches!(request.command,connect_client::tasks::TaskCommand::Invoke {..}));
            let outcome = execute(self, client, &key, operation)
                .await
                .context("Dispatch app outbox with original intent")?;
            if invoke {
                workflow::settle(self, client, &key, outcome).await?;
            }
        }
        let unresolved=self.rows("SELECT key,outcome FROM app_commands WHERE effect_state IN ('pending','uncertain') ORDER BY sequence LIMIT 16")?;
        for (key, json) in unresolved {
            workflow::settle(self, client, &key, serde_json::from_str(&json)?).await?;
        }
        Ok(())
    }
}
pub(super) fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
