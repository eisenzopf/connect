//! Bounded installation expiry uses Core's canonical selection and transform.
use super::*;
use crate::Service;
use connect_protocol::{record_mutations::Mode, Error, Result as ConnectResult};
use durable::retention::{Decision, Source};
use rvoip_core::records::redaction::{
    retention_action, RecordRetentionAction, SourceRedaction, SourceRedactionMode,
};

pub(super) fn select(
    current: &durable::RecordBytes,
    expected: &durable::RecordHandle,
    operation: &str,
    cutoff: i64,
) -> ConnectResult<Option<Decision>> {
    let current = record(current).map_err(|_| Error::storage())?;
    let cutoff = chrono::DateTime::from_timestamp(cutoff, 0).ok_or_else(Error::storage)?;
    let action = retention_action(&current, operation.into(), handle(expected.clone()), cutoff)
        .map_err(|_| Error::storage())?;
    Ok(action.map(|action| match action {
        RecordRetentionAction::Delete(_) => Decision::Delete,
        RecordRetentionAction::Redact(intent) => Decision::Redact {
            sources: intent
                .sources()
                .iter()
                .map(|s| Source {
                    source_id: s.source_id.clone(),
                    mode: match s.mode {
                        SourceRedactionMode::Content => Mode::Content,
                        SourceRedactionMode::Metadata => Mode::Metadata,
                    },
                })
                .collect(),
        },
    }))
}
fn transform(
    current: &durable::RecordBytes,
    expected: &durable::RecordHandle,
    operation: &str,
    decision: &Decision,
) -> ConnectResult<Option<durable::RecordBytes>> {
    let current = record(current).map_err(|_| Error::storage())?;
    match decision {
        Decision::Delete => Ok(None),
        Decision::Redact { sources } => {
            let intent = VconRedaction::new(
                operation.into(),
                handle(expected.clone()),
                sources
                    .iter()
                    .map(|s| SourceRedaction {
                        source_id: s.source_id.clone(),
                        mode: match s.mode {
                            Mode::Content => SourceRedactionMode::Content,
                            Mode::Metadata => SourceRedactionMode::Metadata,
                        },
                    })
                    .collect(),
            )
            .map_err(|_| Error::storage())?;
            let next = rvoip_core::records::redaction::apply_redaction(&current, &intent)
                .map_err(|_| Error::storage())?;
            bytes(&next).map(Some).map_err(|_| Error::storage())
        }
    }
}
impl Service {
    pub async fn expire_record_outcomes(
        &self,
        limit: usize,
    ) -> ConnectResult<durable::RecordIntentBatch> {
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || {
            let cutoff = chrono::Utc::now().timestamp();
            let pending = store.pending_record_retention(limit, cutoff)?;
            let mut batch = durable::RecordIntentBatch {
                examined: pending.len(),
                stored: 0,
                failed: 0,
            };
            for uuid in pending {
                let apply = || -> ConnectResult<bool> {
                    let Some(operation) = store.prepare_record_retention(&uuid, cutoff, select)?
                    else {
                        return Ok(false);
                    };
                    store.apply_record_retention(&operation, transform)
                };
                match apply() {
                    Ok(true) => batch.stored += 1,
                    Ok(false) => (),
                    Err(_) => batch.failed += 1,
                }
            }
            Ok::<_, Error>(batch)
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.as_ref().is_ok_and(|b| b.stored > 0) {
            self.notify_changed();
        }
        result
    }
}
