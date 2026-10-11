//! Inbox checkpoints, one business effect and its response outbox commit together.
use super::{domain::digest, fault, ledger, state::AppStore};
use anyhow::{ensure, Result};
use connect_client::{Event, InboxPage, Operation};
use rusqlite::{params, OptionalExtension};

impl AppStore {
    pub(super) fn stage(&mut self, page: InboxPage) -> Result<()> {
        ensure!(
            self.metadata("page")?.is_none(),
            "Do not replace an unfinished page"
        );
        ensure!(
            page.events.len() <= 100 && serde_json::to_vec(&page.events)?.len() <= 512 * 1024,
            "Inbox page exceeds app limit"
        );
        let total: u64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM app_events", [], |r| r.get(0))?;
        ensure!(
            total + page.events.len() as u64 <= 10000,
            "App event ledger full; preserve checkpoint"
        );
        let tx = self.connection.transaction()?;
        for event in page.events {
            let prior: Option<String> = tx
                .query_row(
                    "SELECT event FROM app_events WHERE id=?1",
                    [&event.id],
                    |r| r.get(0),
                )
                .optional()?;
            let json = serde_json::to_string(&event)?;
            if let Some(prior) = prior {
                ensure!(prior == json, "Journal event identity changed");
            } else {
                tx.execute(
                    "INSERT INTO app_events(id,event) VALUES(?1,?2)",
                    params![event.id, json],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO app_metadata VALUES('page',?1)",
            [serde_json::to_string(&page.cursor)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn next_event(&self) -> Result<Option<Event>> {
        let s: Option<String> = self
            .connection
            .query_row(
                "SELECT event FROM app_events WHERE result IS NULL ORDER BY sequence LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        s.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    pub(super) fn finish_event(&self, event: &str, result: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE app_events SET result=?1 WHERE id=?2 AND result IS NULL",
            params![result, event],
        )?;
        Ok(())
    }
    pub(super) fn finish_page(&mut self) -> Result<()> {
        ensure!(
            self.next_event()?.is_none(),
            "Page still contains unfinished work"
        );
        if let Some(cursor) = self.metadata("page")? {
            let tx = self.connection.transaction()?;
            tx.execute("INSERT INTO app_metadata VALUES('cursor',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[cursor])?;
            tx.execute("DELETE FROM app_metadata WHERE key='page'", [])?;
            tx.commit()?;
        }
        Ok(())
    }
    pub(super) fn cursor(&self) -> Result<Option<String>> {
        self.metadata("cursor")?
            .map(|s| Ok(serde_json::from_str::<Option<String>>(&s)?))
            .transpose()
            .map(Option::flatten)
    }
    /// Atomic app-local effect, its durable response intent and processed event.
    /// Reusing a semantic business ID with changed content is a visible conflict.
    pub(super) fn business(
        &mut self,
        key: &str,
        kind: &str,
        task: &str,
        event: &Event,
        payload: serde_json::Value,
        response: Operation,
    ) -> Result<()> {
        let input_digest = digest(&payload)?;
        let tx = self.connection.transaction()?;
        let prior: Option<String> = tx
            .query_row(
                "SELECT input_digest FROM app_business WHERE key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        let result = if let Some(prior) = prior {
            if prior == input_digest {
                "duplicate"
            } else {
                "conflict"
            }
        } else {
            fault("before-business");
            tx.execute(
                "INSERT INTO app_business VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    key,
                    kind,
                    input_digest,
                    task,
                    event.conversation_id,
                    event.id,
                    serde_json::to_string(&payload)?
                ],
            )?;
            ledger::insert(&tx, &format!("response:{key}"), &response)?;
            fault("after-business");
            "committed"
        };
        if kind == "review" && result == "committed" {
            tx.execute(
                "UPDATE app_commands SET effect_state='completed' WHERE key='outgoing-question'",
                [],
            )?;
        }
        tx.execute(
            "UPDATE app_events SET result=?1 WHERE id=?2 AND result IS NULL",
            params![result, event.id],
        )?;
        tx.commit()?;
        fault("after-business-commit");
        Ok(())
    }
}
