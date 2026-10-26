//! Native control receipts stay separate from resource observations and media grants.
use super::*;
use crate::canonical::Payload;
use connect_protocol::native_streams::{Command, Intent};

impl Connection {
    /// End this native Connection while keeping other Connections and the peer alive.
    /// The optional assertion is a negotiated native revision. A resource revision
    /// from `refresh` cannot be used here. Completion proves actual local termination;
    /// the separate durable retirement snapshot can follow later.
    pub async fn end(
        &self,
        reason_code: u16,
        reason: String,
        expected_revision: Option<StreamRevision>,
        options: MutationOptions,
    ) -> Result<Operation<Connection>> {
        let kind = self.client().operation_kinds().connection_end;
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: OperationScope {
                conversation_id: Some(self.conversation_id().clone()),
                session_id: Some(self.session_id().clone()),
                connection_id: Some(self.id().clone()),
            },
        };
        let fail = |kind| Error::request(kind, Some(&key), false);
        let payload = conversations::extend(
            serde_json::json!({"reason_code":reason_code,"reason":reason}),
            options.extensions,
        )
        .map_err(|error| fail(error.kind))?;
        let intent = Intent {
            operation_id: key.operation_id.as_str().into(),
            conversation_id: self.conversation_id().as_str().into(),
            session_id: self.session_id().as_str().into(),
            connection_id: self.id().as_str().into(),
            expected_revision: expected_revision.map(StreamRevision::get),
            by_participant: None,
            command: Command::End,
            payload,
        };
        intent.validate().map_err(|_| fail(ErrorKind::Validation))?;
        let mut scope = key.scope.command(Some(&key.operation_id));
        scope.expected_revision = intent.expected_revision;
        let batch = self
            .client()
            .request(
                kind.command,
                intent.payload,
                scope,
                Some(&key),
                options.request,
            )
            .await?;
        let fail = || Error::request(ErrorKind::Protocol, Some(&key), true);
        let Some(Payload::OperationUpdated(update)) = batch.frames.first().map(|f| f.payload())
        else {
            return Err(fail());
        };
        if batch.frames.len() != 2
            || update.operation_id.as_str() != key.operation_id.as_str()
            || update.revision > 9_007_199_254_740_991
        {
            return Err(fail());
        }
        if serde_json::to_value(update.state).map_err(|_| fail())? == serde_json::json!("completed")
        {
            validate_end(update.result.as_ref().ok_or_else(fail)?, &key)?;
        }
        Ok(Operation::new(
            self.client().clone(),
            OperationReference { key, kind },
        ))
    }
}
pub(super) fn validate_end(value: &Value, key: &OperationKey) -> Result<()> {
    if key.command != "connection.end"
        || !connections::valid_scope(&key.scope)
        || value["ended"] != true
        || value["connection_id"].as_str()
            != key.scope.connection_id.as_ref().map(ConnectionId::as_str)
    {
        return Err(Error::request(ErrorKind::Protocol, Some(key), true));
    }
    Ok(())
}
pub(super) fn decode_end(value: Value, client: &Client, key: &OperationKey) -> Result<Connection> {
    validate_end(&value, key)?;
    client.shared.retire_connection(key)?;
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    Ok(Connection::new(
        client.clone(),
        key.scope.connection_id.clone().ok_or_else(fail)?,
        key.scope.session_id.clone().ok_or_else(fail)?,
        key.scope.conversation_id.clone().ok_or_else(fail)?,
    ))
}
