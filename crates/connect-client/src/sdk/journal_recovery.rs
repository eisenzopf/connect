//! A delivered durable event cannot outrun its owned native routing observation.
use super::*;
use crate::canonical::CommandScope;
use serde_json::json;
use tokio::time::Instant;
fn failure(kind: ErrorKind) -> Error {
    Error::request(kind, None, false)
}
impl Client {
    pub(super) async fn catch_up_native_event(
        &self,
        event: &JournalEvent,
        options: &RequestOptions,
        until: Instant,
    ) -> Result<()> {
        if event.kind != "operation_updated"
            || event.data["kind"] != "operation"
            || event.data["data"]["state"] != "completed"
        {
            return Ok(());
        }
        let Some(session) = &event.session_id else {
            return Ok(());
        };
        if !self
            .shared
            .has_native_journal_scope(&event.conversation_id, session)?
        {
            return Ok(());
        }
        let operation: OperationId =
            serde_json::from_value(event.data["data"]["operation_id"].clone())
                .map_err(|_| failure(ErrorKind::Protocol))?;
        let record: OperationRecordId =
            serde_json::from_value(event.data["data"]["record_id"].clone())
                .map_err(|_| failure(ErrorKind::Protocol))?;
        if event.operation_id.as_ref() != Some(&operation) {
            return Err(failure(ErrorKind::Protocol));
        }
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(failure(ErrorKind::Deadline));
        }
        // Operation events do not promise a Connection ID. Read the exact
        // original record under current authority before discovering its target.
        let batch = self
            .request(
                "operation.get",
                json!({"operation_id":operation}),
                CommandScope {
                    conversation_id: Some(event.conversation_id.as_str().into()),
                    session_id: Some(session.as_str().into()),
                    ..Default::default()
                },
                None,
                RequestOptions {
                    timeout: remaining,
                    signal: options.signal.clone(),
                },
            )
            .await?;
        if batch.frames.len() != 2 || batch.frames[0].kind() != "operation.updated" {
            return Err(failure(ErrorKind::Protocol));
        }
        let p = &batch.frames[0].original()["payload"];
        if p["operation_id"] != json!(operation)
            || p["record_id"] != json!(record)
            || p["revision"]
                .as_u64()
                .is_none_or(|n| n < event.revision || n > 9_007_199_254_740_991)
        {
            return Err(failure(ErrorKind::Protocol));
        }
        if p["command_type"] != "stream.reconcile"
            || p["state"] != "completed"
            || p["result"]["resources_changed"] != false
        {
            return Ok(());
        }
        let connection: ConnectionId = serde_json::from_value(p["result"]["connection_id"].clone())
            .map_err(|_| failure(ErrorKind::Protocol))?;
        if event
            .connection_id
            .as_ref()
            .is_some_and(|id| id != &connection)
        {
            return Err(failure(ErrorKind::Protocol));
        }
        let key = OperationKey {
            operation_id: operation,
            command: "stream.reconcile".into(),
            scope: OperationScope {
                conversation_id: Some(event.conversation_id.clone()),
                session_id: Some(session.clone()),
                connection_id: Some(connection),
            },
        };
        loop {
            if options.signal.is_cancelled() {
                return Err(failure(ErrorKind::Cancelled));
            }
            if Instant::now() >= until {
                return Err(failure(ErrorKind::Deadline));
            }
            if self.shared.catch_up_routing(&key, p)? {
                return Ok(());
            }
            // A different native preparation keeps its original recovery ID.
            // Waiting here holds neither a control slot nor a permission reader.
            tokio::select! {biased;
                _=options.signal.cancelled()=>return Err(failure(ErrorKind::Cancelled)),
                _=tokio::time::sleep_until(until.min(Instant::now()+Duration::from_millis(10)))=>(),
            }
        }
    }
}
