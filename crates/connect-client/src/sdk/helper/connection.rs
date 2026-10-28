//! Native ownership remains with the original public SDK Client across RPCs.
use super::*;

fn nonempty<T>(values: Vec<T>) -> Result<NonEmpty<T>> {
    let mut values = values.into_iter();
    Ok(NonEmpty {
        first: values
            .next()
            .ok_or_else(|| Error::local(ErrorKind::Validation))?,
        rest: values.collect(),
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    id: StreamId,
    kind: Medium,
    direction: Direction,
    codecs: Vec<String>,
}
impl TryFrom<Offer> for OfferedStream {
    type Error = Error;
    fn try_from(v: Offer) -> Result<Self> {
        Ok(Self {
            id: v.id,
            kind: v.kind,
            direction: v.direction,
            codecs: nonempty(v.codecs)?,
        })
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Update {
    Mute {
        streams: Vec<StreamId>,
    },
    Unmute {
        streams: Vec<StreamId>,
    },
    Hold {
        streams: Option<Vec<StreamId>>,
        direction: HoldDirection,
    },
    Resume {
        streams: Option<Vec<StreamId>>,
    },
    Quality {
        interval_ms: u32,
    },
    CodecRenegotiate {
        streams: Option<Vec<StreamId>>,
        codec_preferences: Vec<String>,
    },
}
impl TryFrom<Update> for ConnectionUpdate {
    type Error = Error;
    fn try_from(v: Update) -> Result<Self> {
        Ok(match v {
            Update::Mute { streams } => Self::Mute {
                streams: nonempty(streams)?,
            },
            Update::Unmute { streams } => Self::Unmute {
                streams: nonempty(streams)?,
            },
            Update::Hold { streams, direction } => Self::Hold {
                streams: streams.map(nonempty).transpose()?,
                direction,
            },
            Update::Resume { streams } => Self::Resume {
                streams: streams.map(nonempty).transpose()?,
            },
            Update::Quality { interval_ms } => Self::Quality { interval_ms },
            Update::CodecRenegotiate {
                streams,
                codec_preferences,
            } => Self::CodecRenegotiate {
                streams: streams.map(nonempty).transpose()?,
                codec_preferences: nonempty(codec_preferences)?,
            },
        })
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    Stream {
        stream_id: StreamId,
    },
    Participant {
        participant_id: ParticipantId,
        kinds: Option<Vec<Medium>>,
    },
}
impl TryFrom<Selector> for StreamSelector {
    type Error = Error;
    fn try_from(v: Selector) -> Result<Self> {
        Ok(match v {
            Selector::Stream { stream_id } => Self::Stream(stream_id),
            Selector::Participant {
                participant_id,
                kinds,
            } => Self::Participant {
                participant_id,
                kinds: kinds.map(nonempty).transpose()?,
            },
        })
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConnectionAction {
    Offer {
        streams: Vec<Offer>,
        mutation: Mutation,
    },
    Update {
        update: Update,
        expected_revision: StreamRevision,
        mutation: Mutation,
    },
    End {
        reason_code: u16,
        reason: String,
        #[serde(default)]
        expected_revision: Option<StreamRevision>,
        mutation: Mutation,
    },
    Subscribe {
        selectors: Vec<Selector>,
        expected_revision: StreamRevision,
        mutation: Mutation,
    },
    Unsubscribe {
        streams: Vec<StreamId>,
        expected_revision: StreamRevision,
        mutation: Mutation,
    },
    Refresh,
    StreamRefresh {
        stream_id: StreamId,
    },
}
pub(super) async fn execute(
    client: &Client,
    cid: ConversationId,
    sid: SessionId,
    connid: ConnectionId,
    action: ConnectionAction,
    options: RequestOptions,
) -> Result<Value> {
    let session = client.conversation_by_id(cid).session_by_id(sid);
    let connection = session.connection_by_id(connid.clone());
    Ok(match action {
        ConnectionAction::Offer { streams, mutation } => session
            .offer_connection(
                connid,
                nonempty(
                    streams
                        .into_iter()
                        .map(OfferedStream::try_from)
                        .collect::<Result<Vec<_>>>()?,
                )?,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        ConnectionAction::Update {
            update,
            expected_revision,
            mutation,
        } => connection
            .update(
                update.try_into()?,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        ConnectionAction::End {
            reason_code,
            reason,
            expected_revision,
            mutation,
        } => connection
            .end(
                reason_code,
                reason,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        ConnectionAction::Subscribe {
            selectors,
            expected_revision,
            mutation,
        } => connection
            .subscribe(
                nonempty(
                    selectors
                        .into_iter()
                        .map(StreamSelector::try_from)
                        .collect::<Result<Vec<_>>>()?,
                )?,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        ConnectionAction::Unsubscribe {
            streams,
            expected_revision,
            mutation,
        } => connection
            .unsubscribe(
                nonempty(streams)?,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json(),
        ConnectionAction::Refresh => {
            let v = connection.refresh(options).await?;
            json!({"id":v.id,"conversationId":v.conversation_id,"sessionId":v.session_id,"generation":v.generation,"streamRevision":v.stream_revision,"state":v.state})
        }
        ConnectionAction::StreamRefresh { stream_id } => {
            stream_json(connection.stream_by_id(stream_id).refresh(options).await?)
        }
    })
}
pub(super) fn handle_json(v: Connection) -> Value {
    json!({"id":v.id(),"conversationId":v.conversation_id(),"sessionId":v.session_id(),"receiptStreamRevision":v.receipt_stream_revision()})
}
fn parameters(v: NegotiatedStream) -> Value {
    json!({"id":v.id,"medium":v.medium,"direction":v.direction,"codec":v.codec,"streamLocalId":v.stream_local_id})
}
fn codec(v: CodecObservation) -> Value {
    json!({"parameters":parameters(v.parameters),"streamGeneration":v.stream_generation,"connectionGeneration":v.connection_generation})
}
pub(super) fn update_json(v: ConnectionUpdateReceipt) -> Value {
    let outcome = match v.outcome {
        ConnectionUpdateOutcome::Quality { interval_ms } => {
            json!({"kind":"quality","intervalMs":interval_ms})
        }
        ConnectionUpdateOutcome::Codecs {
            resources_changed,
            streams,
        } => {
            json!({"kind":"codecs","resourcesChanged":resources_changed,"streams":streams.into_iter().map(codec).collect::<Vec<_>>()})
        }
        ConnectionUpdateOutcome::Gates { action, streams } => {
            json!({"kind":"gates","action":action,"streams":streams.into_iter().map(|s|json!({"parameters":parameters(s.parameters),"streamGeneration":s.stream_generation,"connectionGeneration":s.connection_generation,"state":s.state,"sourceMuted":s.source_muted,"heldDirection":s.held_direction})).collect::<Vec<_>>()})
        }
    };
    json!({"connection":handle_json(v.connection),"connectionGeneration":v.connection_generation,"streamRevision":v.stream_revision,"outcome":outcome})
}
pub(super) fn selection_json(v: StreamSelectionReceipt) -> Value {
    json!({"connection":handle_json(v.connection),"connectionGeneration":v.connection_generation,"streamRevision":v.stream_revision,"resourcesChanged":v.resources_changed,"bindings":v.bindings.map(|b|b.into_iter().map(codec).collect::<Vec<_>>())})
}
fn stream_json(v: StreamSnapshot) -> Value {
    let ownership = match v.ownership {
        MediaOwnership::Observation => "observation",
        MediaOwnership::LocalSource => "local-source",
        MediaOwnership::LocalSink => "local-sink",
        MediaOwnership::LocalDuplex => "local-duplex",
    };
    json!({"id":v.id,"conversationId":v.conversation_id,"sessionId":v.session_id,"connectionId":v.connection_id,"medium":v.medium,"state":v.state,"resourceRevision":v.revision,"direction":v.direction,"format":v.format.map(|f|json!({"codec":f.codec,"clockRate":f.clock_rate,"channels":f.channels,"width":f.width,"height":f.height})),"clock":v.clock.map(|c|json!({"sourceId":c.source_id,"generation":c.generation,"unitsPerSecond":c.units_per_second.to_string(),"synchronized":c.synchronized})),"ownership":ownership})
}
