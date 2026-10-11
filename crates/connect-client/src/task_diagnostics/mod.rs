//! Bounded diagnostics composed through existing authenticated public interfaces.
mod models;
mod projection;
#[cfg(test)]
mod tests;
use crate::{ClientError, ConnectClient, Result};
use connect_protocol::{tasks::*, Error};
pub use models::*;
use projection::{event, record, section, source, task, unavailable};

impl ConnectClient {
    /// Sample one page of task events, source references and unassigned replies,
    /// plus server process health. Never widens a grant or falls back to another
    /// credential. Missing optional read permission is explicit; transport,
    /// cursor and configuration errors fail the sample. Limit must be 1–100.
    ///
    /// Journal effect states are historical observations, not current delivery
    /// receipts. Use authorized EffectGet for current reconciliation. Persist
    /// the returned cursor in the application transaction only after processing
    /// its events; diagnostic reads never acknowledge or consume them.
    ///
    /// ```no_run
    /// # async fn inspect(client: &mut connect_client::ConnectClient, task: &str) -> connect_client::Result<()> {
    /// let view = client.task_diagnostics(task, &Default::default(), 100).await?;
    /// assert_eq!(view.task.task_id, task);
    /// # Ok(()) }
    /// ```
    pub async fn task_diagnostics(
        &mut self,
        task_id: &str,
        cursors: &DiagnosticCursors,
        limit: u32,
    ) -> Result<TaskDiagnostics> {
        if !(1..=100).contains(&limit) {
            return Err(
                Error::new("invalid-request", "Diagnostic page limit must be 1–100").into(),
            );
        }
        let first = self.negotiate_task(task_id).await?;
        let journal = section(
            self.task(TaskCommand::Wait {
                task_id: task_id.into(),
                after: cursors.journal.clone(),
                limit,
                wait_ms: 0,
            })
            .await,
            |outcome| {
                let TaskOutcome::Inbox { events, next } = outcome else {
                    return Err(ClientError::Protocol);
                };
                Ok(DiagnosticPage {
                    items: events.into_iter().map(event).collect(),
                    after: cursors.journal.clone(),
                    next,
                    limit,
                    omitted: None,
                })
            },
        )?;
        let sources = section(
            self.task(TaskCommand::Recall {
                task_id: task_id.into(),
                after: cursors.sources.clone(),
                limit,
            })
            .await,
            |outcome| {
                let TaskOutcome::Recall {
                    sources,
                    next,
                    omitted,
                } = outcome
                else {
                    return Err(ClientError::Protocol);
                };
                Ok(DiagnosticPage {
                    items: sources.into_iter().map(source).collect(),
                    after: cursors.sources.clone(),
                    next,
                    limit,
                    omitted: Some(omitted),
                })
            },
        )?;
        let unassigned = section(
            self.task(TaskCommand::Replies {
                conversation_id: first.conversation_id.clone(),
                after: cursors.unassigned.clone(),
                limit,
            })
            .await,
            |outcome| {
                let TaskOutcome::Replies { items, next } = outcome else {
                    return Err(ClientError::Protocol);
                };
                Ok(DiagnosticPage {
                    items: items
                        .into_iter()
                        .filter(|item| item.task_id.is_none())
                        .collect(),
                    after: cursors.unassigned.clone(),
                    next,
                    limit,
                    omitted: None,
                })
            },
        )?;
        let server_readiness = match self.health().await {
            Ok(health) => health
                .readiness
                .map(DiagnosticSection::Available)
                .unwrap_or(DiagnosticSection::Unavailable {
                    code: "readiness-not-reported".into(),
                }),
            Err(error) => unavailable(error)?,
        };
        let last = record(
            self.task(TaskCommand::Get {
                task_id: task_id.into(),
            })
            .await?,
        )?;
        if last.id != first.id
            || last.conversation_id != first.conversation_id
            || last.owner_id != first.owner_id
        {
            return Err(ClientError::Protocol);
        }
        Ok(TaskDiagnostics {
            changed_during_sample: last.revision != first.revision,
            task: task(last),
            journal,
            sources,
            unassigned,
            server_readiness,
        })
    }
}
