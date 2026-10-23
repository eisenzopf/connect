//! Explicit current local record erasure; provider deletion is never inferred.
use super::*;
use crate::Service;
use connect_protocol::{
    record_mutations::{Action, Mode, Outcome, Request},
    Error, Result as ConnectResult,
};
use rvoip_core::records::redaction::{SourceRedaction, SourceRedactionMode};
impl Service {
    pub async fn mutate_record(
        &self,
        credential: String,
        request: Request,
    ) -> ConnectResult<Outcome> {
        request.validate()?;
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || {
            store.mutate_authorized_record(&credential, &request, |current, expected, sources| {
                // Validate even deletion: an unknown persisted shape is not
                // silently transformed or treated as a supported public record.
                let canonical = record(current).map_err(|_| Error::storage())?;
                let Action::Redact { .. } = &request.action else {
                    return Ok(None);
                };
                let intent = VconRedaction::new(
                    request.operation_id.clone(),
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
                .map_err(|_| Error::new("invalid-request", "Invalid redaction intent"))?;
                let next = rvoip_core::records::redaction::apply_redaction(&canonical, &intent)
                    .map_err(|_| Error::new("invalid-request", "Redaction cannot be applied"))?;
                bytes(&next).map(Some).map_err(|_| Error::storage())
            })
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
