//! Original-ID receive selections and historical native binding observations.
use super::*;
use connect_protocol::native_streams::{Command, Intent, SelectionObservation};
use serde_json::json;
#[derive(Clone, Debug)]
pub enum StreamSelector {
    Stream(StreamId),
    Participant {
        participant_id: ParticipantId,
        kinds: Option<NonEmpty<Medium>>,
    },
}
impl StreamSelector {
    fn payload(self) -> Value {
        match self {
            Self::Stream(id) => json!({"strm_id":id}),
            Self::Participant {
                participant_id,
                kinds,
            } => {
                let mut v = json!({"from_participant":participant_id});
                if let Some(kinds) = kinds {
                    v["kinds"] = json!(kinds.into_vec());
                }
                v
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct StreamSelectionReceipt {
    pub connection: Connection,
    pub connection_generation: u64,
    pub stream_revision: StreamRevision,
    pub resources_changed: bool,
    /// None means the committed effect's concurrent binding capture was unavailable.
    /// A known empty set is Some(vec![]). These facts never grant foreign custody.
    pub bindings: Option<Vec<CodecObservation>>,
}
pub(super) fn is_command(command: &str) -> bool {
    matches!(command, "stream.subscribe" | "stream.unsubscribe")
}
pub(super) fn parse(v: &Value, key: &OperationKey) -> Result<SelectionObservation> {
    let bad = || Error::request(ErrorKind::Protocol, Some(key), true);
    if !(is_command(&key.command) || key.command == "stream.reconcile")
        || !connections::valid_scope(&key.scope)
        || serde_json::to_vec(v).map_err(|_| bad())?.len() > 32 * 1024
        || !v
            .as_object()
            .is_some_and(|o| o.contains_key("stream_bindings"))
    {
        return Err(bad());
    }
    let mut value = v.clone();
    if key.command == "stream.reconcile" {
        if value["server_reconciliation"] != json!(true)
            || value["subscription_record_id"]
                .as_str()
                .and_then(|s| s.parse::<OperationRecordId>().ok())
                .is_none()
        {
            return Err(bad());
        }
        let object = value.as_object_mut().ok_or_else(bad)?;
        object.remove("server_reconciliation");
        object.remove("subscription_record_id");
    }
    let out: SelectionObservation = serde_json::from_value(value).map_err(|_| bad())?;
    out.validate().map_err(|_| bad())?;
    if out.command_type.name() != key.command
        || key.scope.connection_id.as_ref().map(ConnectionId::as_str)
            != Some(out.connection_id.as_str())
    {
        return Err(bad());
    }
    Ok(out)
}
impl Connection {
    pub async fn subscribe(
        &self,
        selectors: NonEmpty<StreamSelector>,
        expected: StreamRevision,
        options: MutationOptions,
    ) -> Result<Operation<StreamSelectionReceipt>> {
        self.select(Command::Subscribe,json!({"subscriptions":selectors.into_vec().into_iter().map(StreamSelector::payload).collect::<Vec<_>>()}),expected,options).await
    }
    pub async fn unsubscribe(
        &self,
        streams: NonEmpty<StreamId>,
        expected: StreamRevision,
        options: MutationOptions,
    ) -> Result<Operation<StreamSelectionReceipt>> {
        self.select(
            Command::Unsubscribe,
            json!({"strm_ids":streams.into_vec()}),
            expected,
            options,
        )
        .await
    }
    async fn select(
        &self,
        command: Command,
        body: Value,
        expected: StreamRevision,
        options: MutationOptions,
    ) -> Result<Operation<StreamSelectionReceipt>> {
        let kind = match command {
            Command::Subscribe => self.client().operation_kinds().stream_subscribe,
            Command::Unsubscribe => self.client().operation_kinds().stream_unsubscribe,
            _ => unreachable!(),
        };
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: OperationScope {
                conversation_id: Some(self.conversation_id().clone()),
                session_id: Some(self.session_id().clone()),
                connection_id: Some(self.id().clone()),
            },
        };
        let bad = |kind| Error::request(kind, Some(&key), false);
        let payload = conversations::extend(body, options.extensions).map_err(|e| bad(e.kind))?;
        let intent = Intent {
            operation_id: key.operation_id.as_str().into(),
            conversation_id: self.conversation_id().as_str().into(),
            session_id: self.session_id().as_str().into(),
            connection_id: self.id().as_str().into(),
            expected_revision: Some(expected.get()),
            by_participant: None,
            command,
            payload,
        };
        intent.validate().map_err(|_| bad(ErrorKind::Validation))?;
        let mut scope = key.scope.command(Some(&key.operation_id));
        scope.expected_revision = Some(expected.get());
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
        let bad = || Error::request(ErrorKind::Protocol, Some(&key), true);
        let Some(crate::canonical::Payload::OperationUpdated(update)) =
            batch.frames.first().map(|f| f.payload())
        else {
            return Err(bad());
        };
        if batch.frames.len() != 2
            || update.operation_id.as_str() != key.operation_id.as_str()
            || update.revision > 9_007_199_254_740_991
        {
            return Err(bad());
        }
        if serde_json::to_value(update.state).map_err(|_| bad())? == json!("completed") {
            let out = parse(update.result.as_ref().ok_or_else(bad)?, &key)?;
            if out.expected_revision != expected.get() {
                return Err(bad());
            }
        }
        Ok(Operation::new(
            self.client().clone(),
            OperationReference {
                key,
                kind,
                continuation: None,
            },
        ))
    }
}
pub(super) fn decode(
    v: Value,
    client: &Client,
    key: &OperationKey,
) -> Result<StreamSelectionReceipt> {
    let out = parse(&v, key)?;
    if is_command(&key.command) {
        client.shared.record_selection(key, &out)?;
    }
    let bad = || Error::request(ErrorKind::Protocol, Some(key), true);
    let bindings = out
        .stream_bindings
        .map(|bindings| {
            bindings
                .into_iter()
                .map(|b| {
                    Ok(CodecObservation {
                        parameters: NegotiatedStream {
                            id: b.stream_id.parse().map_err(|_| bad())?,
                            medium: serde_json::from_value(json!(b.medium)).map_err(|_| bad())?,
                            direction: serde_json::from_value::<Direction>(json!(b.direction))
                                .map_err(|_| bad())?
                                .reverse(),
                            codec: b.codec,
                            stream_local_id: (b.medium != "data").then_some(b.stream_local_id),
                        },
                        stream_generation: b.stream_generation,
                        connection_generation: b.connection_generation,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    Ok(StreamSelectionReceipt {
        connection: client
            .conversation_by_id(key.scope.conversation_id.clone().ok_or_else(bad)?)
            .session_by_id(key.scope.session_id.clone().ok_or_else(bad)?)
            .connection_by_id(key.scope.connection_id.clone().ok_or_else(bad)?),
        connection_generation: out.connection_generation,
        stream_revision: StreamRevision::try_from(out.stream_revision).map_err(|_| bad())?,
        resources_changed: out.resources_changed,
        bindings,
    })
}
