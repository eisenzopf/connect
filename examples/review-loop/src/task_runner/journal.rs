//! App action, processed events, follow-up outbox and checkpoint share one commit.
use super::{
    commands::{insert, logical_id, Lane},
    state::TaskStore,
};
use anyhow::{Context, Result};
use connect_client::{
    tasks::{TaskCommand, TaskEvent, TaskEventData, TaskOutcome},
    Operation,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Page {
    pub events: Vec<TaskEvent>,
    pub next: Option<String>,
}

impl TaskStore {
    pub(super) fn metadata(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM runner_metadata WHERE key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub(super) fn page(&self) -> Result<Option<Page>> {
        self.metadata("page")?
            .map(|s| Ok(serde_json::from_str(&s)?))
            .transpose()
    }

    /// Persist fetched events before remotely consuming any reply. A crash before
    /// local commit resumes this exact page and its original consume receipts.
    pub(super) fn stage_page(&self, page: &Page) -> Result<()> {
        self.validate_page(page)?;
        let encoded = serde_json::to_string(page)?;
        anyhow::ensure!(encoded.len() <= 65_536, "Task inbox page exceeds app bound");
        if let Some(existing) = self.metadata("page")? {
            anyhow::ensure!(
                existing == encoded,
                "Unfinished page must be reconciled first"
            );
        } else {
            self.connection
                .execute("INSERT INTO runner_metadata VALUES('page',?1)", [encoded])?;
        }
        Ok(())
    }

    pub(super) fn processed(&self, event: &TaskEvent) -> Result<bool> {
        let sequence: Option<u64> = self
            .connection
            .query_row(
                "SELECT sequence FROM runner_events WHERE id=?1",
                [&event.id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(sequence) = sequence {
            anyhow::ensure!(sequence == event.sequence, "Journal event identity changed");
        }
        Ok(sequence.is_some())
    }

    fn validate_page(&self, page: &Page) -> Result<()> {
        anyhow::ensure!(
            page.events.len() <= 100,
            "Task inbox exceeds app event bound"
        );
        if page.events.is_empty() {
            anyhow::ensure!(
                page.next == self.metadata("cursor")?,
                "Empty inbox cannot manufacture or regress a checkpoint"
            );
        }
        let last: u64 = self
            .metadata("sequence")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(0);
        let mut previous = 0;
        for event in &page.events {
            anyhow::ensure!(
                event.task_id == self.binding.task_id && event.sequence > previous,
                "Task inbox binding or commit ordering changed"
            );
            anyhow::ensure!(
                self.processed(event)? || event.sequence > last,
                "Unseen event precedes committed checkpoint"
            );
            if let TaskEventData::Reply(reply) = &event.data {
                anyhow::ensure!(
                    reply.task_id.as_deref() == Some(&self.binding.task_id)
                        && reply.conversation_id == self.binding.conversation_id,
                    "Reply is not explicitly assigned to this task"
                );
            }
            previous = event.sequence;
        }
        Ok(())
    }

    /// All unconsumed replies must first have their persisted successful control
    /// receipt. A competing worker's reply-consumed rejection cannot count as one.
    pub(super) fn commit_page(&mut self, page: &Page) -> Result<()> {
        self.validate_page(page)?;
        let last: u64 = self
            .metadata("sequence")?
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(0);
        let binding = self.binding.clone();
        for event in &page.events {
            if !self.processed(event)? {
                if let TaskEventData::Reply(reply) = &event.data {
                    if !reply.consumed {
                        let command = self
                            .consume_intent(&reply.event_id)?
                            .context("Reply consumption intent is missing")?;
                        anyhow::ensure!(
                            command.lane == Lane::Controller
                                && matches!(
                                    &command.command, TaskCommand::ConsumeReply { task_id, event_id, .. }
                                        if task_id == &binding.task_id && event_id == &reply.event_id
                                ),
                            "Consume intent does not match this reply"
                        );
                        let Some(TaskOutcome::Record(record)) = command.outcome else {
                            anyhow::bail!("Reply must be consumed before the app action");
                        };
                        anyhow::ensure!(
                            record.id == binding.task_id
                                && record.owner_id == binding.owner_id
                                && record.conversation_id == binding.conversation_id,
                            "Consume receipt does not match app binding"
                        );
                    }
                }
            }
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        #[cfg(feature = "test-support")]
        let mut acted = false;
        for event in &page.events {
            if tx.execute(
                "INSERT OR IGNORE INTO runner_events VALUES(?1,?2)",
                params![event.id, event.sequence],
            )? == 0
            {
                continue;
            }
            if let TaskEventData::Reply(reply) = &event.data {
                if !reply.consumed
                    && tx.execute(
                        "INSERT OR IGNORE INTO runner_actions VALUES(?1,'draft_response_recorded')",
                        [&reply.event_id],
                    )? == 1
                {
                    let request_id = logical_id();
                    #[cfg(feature = "test-support")]
                    {
                        acted = true;
                    }
                    let command = TaskCommand::Invoke {
                        request_id: request_id.clone(),
                        task_id: binding.task_id.clone(),
                        step_id: format!("response-{}", reply.event_id),
                        operation: Box::new(Operation::Send {
                            request_id,
                            conversation_id: binding.conversation_id.clone(),
                            body: "Your draft response is recorded.".into(),
                        }),
                        reply_to: Some(reply.event_id.clone()),
                        wait_for_reply: false,
                    };
                    insert(
                        &tx,
                        &format!("follow-up-{}", reply.event_id),
                        Lane::Invocation,
                        &command,
                    )?;
                }
            }
        }
        let maximum = page.events.last().map_or(last, |e| e.sequence.max(last));
        if maximum > last || page.events.is_empty() {
            if let Some(next) = &page.next {
                tx.execute("INSERT INTO runner_metadata VALUES('cursor',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [next])?;
            }
            tx.execute("INSERT INTO runner_metadata VALUES('sequence',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [maximum.to_string()])?;
        }
        tx.execute("DELETE FROM runner_metadata WHERE key='page'", [])?;
        #[cfg(feature = "test-support")]
        if acted {
            super::fault("before-app-commit");
        }
        tx.commit()?;
        #[cfg(feature = "test-support")]
        if acted {
            super::fault("after-app-commit");
        }
        Ok(())
    }
}

pub(super) fn consume_key(event: &str) -> String {
    format!("consume-{event}")
}
