//! Typed native runtime controls and signed codec replacement receipts.
use super::*;
use connect_protocol::{
    native_gates::GateStream,
    native_streams::{Command, Intent},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldDirection {
    Inactive,
    Sendonly,
    Recvonly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateAction {
    Hold,
    Resume,
    Mute,
    Unmute,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateState {
    Active,
    Paused,
}
#[derive(Clone, Debug)]
pub enum ConnectionUpdate {
    Hold {
        streams: Option<NonEmpty<StreamId>>,
        direction: HoldDirection,
    },
    Resume {
        streams: Option<NonEmpty<StreamId>>,
    },
    Mute {
        streams: NonEmpty<StreamId>,
    },
    Unmute {
        streams: NonEmpty<StreamId>,
    },
    Quality {
        interval_ms: u32,
    },
    CodecRenegotiate {
        streams: Option<NonEmpty<StreamId>>,
        codec_preferences: NonEmpty<String>,
    },
}
impl ConnectionUpdate {
    fn payload(self) -> Value {
        let (mut value, streams) = match self {
            Self::Hold { streams, direction } => (
                json!({"action":"hold","details":{"direction":direction}}),
                streams,
            ),
            Self::Resume { streams } => (json!({"action":"resume"}), streams),
            Self::Mute { streams } => (json!({"action":"mute"}), Some(streams)),
            Self::Unmute { streams } => (json!({"action":"unmute"}), Some(streams)),
            Self::Quality { interval_ms } => (
                json!({"action":"quality","quality_interval_ms":interval_ms}),
                None,
            ),
            Self::CodecRenegotiate {
                streams,
                codec_preferences,
            } => (
                json!({"action":"codec-renegotiate","codec_preferences":codec_preferences.into_vec()}),
                streams,
            ),
        };
        if let Some(streams) = streams {
            value["streams"] = json!(streams.into_vec());
        }
        value
    }
}
#[derive(Clone, Debug)]
pub struct StreamGateObservation {
    /// Captured parameters at this client, not a new local ownership grant.
    pub parameters: NegotiatedStream,
    pub stream_generation: u64,
    pub connection_generation: u64,
    pub state: GateState,
    pub source_muted: bool,
    /// Hold direction at this client, reversed from the server's captured direction.
    pub held_direction: Option<Direction>,
}
#[derive(Clone, Debug)]
pub struct CodecObservation {
    /// Historical parameters at this client; not physical media ownership.
    pub parameters: NegotiatedStream,
    pub stream_generation: u64,
    pub connection_generation: u64,
}
#[derive(Clone, Debug)]
pub enum ConnectionUpdateOutcome {
    /// Historical server-observed transport generation; no local media grant.
    IceRestart {
        ice_generation: u64,
    },
    Codecs {
        resources_changed: bool,
        streams: Vec<CodecObservation>,
    },
    Gates {
        action: GateAction,
        streams: Vec<StreamGateObservation>,
    },
    Quality {
        interval_ms: u32,
    },
}
#[derive(Clone, Debug)]
pub struct ConnectionUpdateReceipt {
    pub connection: Connection,
    pub connection_generation: u64,
    pub stream_revision: StreamRevision,
    pub outcome: ConnectionUpdateOutcome,
}
impl Connection {
    /// Apply native runtime controls, quality settings or signed codec replacement.
    /// A codec reoffer is validated and confirmed by this client's owned driver;
    /// its old binding remains local until the paired completion receipt arrives.
    /// Hold directions describe this client. Assertions use the native model's
    /// StreamRevision, independently of snapshot resource revisions. A completed
    /// receipt records the effect; it does not prove current media flow.
    pub async fn update(
        &self,
        update: ConnectionUpdate,
        expected_revision: StreamRevision,
        options: MutationOptions,
    ) -> Result<Operation<ConnectionUpdateReceipt>> {
        let kind = self.client().operation_kinds().connection_update;
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
        let payload = conversations::extend(update.payload(), options.extensions)
            .map_err(|e| fail(e.kind))?;
        let intent = Intent {
            operation_id: key.operation_id.as_str().into(),
            conversation_id: self.conversation_id().as_str().into(),
            session_id: self.session_id().as_str().into(),
            connection_id: self.id().as_str().into(),
            expected_revision: Some(expected_revision.get()),
            by_participant: None,
            command: Command::Update,
            payload,
        };
        intent.validate().map_err(|_| fail(ErrorKind::Validation))?;
        if intent.payload["action"] == "codec-renegotiate"
            && !intent.payload["codec_preferences"]
                .as_array()
                .is_some_and(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(connections::codec_name)
                        .any(|name| matches!(name.as_str(), "pcmu" | "pcma" | "opus" | "vp8"))
                })
        {
            return Err(fail(ErrorKind::Unsupported));
        }
        let mut scope = key.scope.command(Some(&key.operation_id));
        scope.expected_revision = intent.expected_revision;
        let batch = self
            .client()
            .request(
                kind.command,
                intent.payload.clone(),
                scope,
                Some(&key),
                options.request,
            )
            .await?;
        let fail = || Error::request(ErrorKind::Protocol, Some(&key), true);
        let Some(crate::canonical::Payload::OperationUpdated(update)) =
            batch.frames.first().map(|f| f.payload())
        else {
            return Err(fail());
        };
        if batch.frames.len() != 2 || update.operation_id.as_str() != key.operation_id.as_str() {
            return Err(fail());
        }
        if serde_json::to_value(update.state).map_err(|_| fail())? == json!("completed") {
            let observed = parse(update.result.as_ref().ok_or_else(fail)?, &key)?;
            validate_intent(&observed, &intent.payload, expected_revision.get(), &key)?;
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
pub(super) struct Observed {
    pub ice: Option<u64>,
    pub generation: u64,
    pub revision: StreamRevision,
    pub action: Option<GateAction>,
    pub gates: Vec<GateStream>,
    pub quality: Option<u32>,
    pub codecs: Option<Vec<connect_protocol::native_streams::CodecBinding>>,
    pub resources_changed: bool,
}
fn positive(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)
}
pub(super) fn parse(value: &Value, key: &OperationKey) -> Result<Observed> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    if key.command != "connection.update"
        || !connections::valid_scope(&key.scope)
        || value["connection_id"].as_str()
            != key.scope.connection_id.as_ref().map(ConnectionId::as_str)
    {
        return Err(fail());
    }
    if value["action"] == "ice-restart" {
        if value["resources_changed"] != false
            || [
                "codec_bindings",
                "gate_streams",
                "quality_interval_ms",
                "setup",
                "sdp",
            ]
            .iter()
            .any(|k| value.get(k).is_some())
        {
            return Err(fail());
        }
        return Ok(Observed {
            generation: positive(&value["connection_generation"]).ok_or_else(fail)?,
            revision: StreamRevision::try_from(
                positive(&value["stream_revision"]).ok_or_else(fail)?,
            )
            .map_err(|_| fail())?,
            ice: Some(positive(&value["ice_generation"]).ok_or_else(fail)?),
            action: None,
            gates: vec![],
            quality: None,
            codecs: None,
            resources_changed: false,
        });
    }
    if value.get("ice_generation").is_some() {
        return Err(fail());
    }
    if let Some(bindings) = value.get("codec_bindings") {
        let bindings: Vec<connect_protocol::native_streams::CodecBinding> =
            serde_json::from_value(bindings.clone()).map_err(|_| fail())?;
        connect_protocol::native_streams::validate_codec_bindings(&bindings).map_err(|_| fail())?;
        if value.get("gate_streams").is_some()
            || value.get("quality_interval_ms").is_some()
            || value.get("action").is_some()
        {
            return Err(fail());
        }
        let generation = bindings.first().ok_or_else(fail)?.connection_generation;
        if bindings
            .iter()
            .any(|b| b.connection_generation != generation)
        {
            return Err(fail());
        }
        let revision =
            StreamRevision::try_from(positive(&value["stream_revision"]).ok_or_else(fail)?)
                .map_err(|_| fail())?;
        return Ok(Observed {
            ice: None,
            generation,
            revision,
            action: None,
            gates: vec![],
            quality: None,
            codecs: Some(bindings),
            resources_changed: value["resources_changed"].as_bool().ok_or_else(fail)?,
        });
    }
    if value["resources_changed"] != false {
        return Err(fail());
    }
    let generation = positive(&value["connection_generation"]).ok_or_else(fail)?;
    let revision = StreamRevision::try_from(positive(&value["stream_revision"]).ok_or_else(fail)?)
        .map_err(|_| fail())?;
    if let Some(interval) = value.get("quality_interval_ms") {
        let interval = interval
            .as_u64()
            .filter(|n| (100..=60_000).contains(n))
            .ok_or_else(fail)? as u32;
        if value.get("gate_streams").is_some() || value.get("action").is_some() {
            return Err(fail());
        }
        return Ok(Observed {
            ice: None,
            generation,
            revision,
            action: None,
            gates: vec![],
            quality: Some(interval),
            codecs: None,
            resources_changed: false,
        });
    }
    let action: GateAction = serde_json::from_value(value["action"].clone()).map_err(|_| fail())?;
    let gates: Vec<GateStream> =
        serde_json::from_value(value["gate_streams"].clone()).map_err(|_| fail())?;
    let mut payload = json!({"action":action,"streams":gates.iter().map(|s|&s.binding.stream_id).collect::<Vec<_>>()});
    if action == GateAction::Hold {
        let first = gates.first().ok_or_else(fail)?;
        let direction = if first.state == "paused" {
            Direction::Inactive
        } else {
            serde_json::from_value::<Direction>(json!(first
                .held_direction
                .as_ref()
                .ok_or_else(fail)?))
            .map_err(|_| fail())?
            .reverse()
        };
        payload["details"] = json!({"direction":direction});
    }
    connect_protocol::native_gates::validate_observation(&payload, &gates).map_err(|_| fail())?;
    if gates
        .iter()
        .any(|g| g.binding.connection_generation != generation)
    {
        return Err(fail());
    }
    Ok(Observed {
        ice: None,
        generation,
        revision,
        action: Some(action),
        gates,
        quality: None,
        codecs: None,
        resources_changed: false,
    })
}
pub(super) fn validate_intent(
    observed: &Observed,
    payload: &Value,
    expected: u64,
    key: &OperationKey,
) -> Result<()> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    if payload["action"] == "ice-restart" {
        let intent =
            connect_protocol::webrtc_ice_restart::parse_update(payload).map_err(|_| fail())?;
        if observed.ice != Some(intent.details.expected_ice_generation + 1)
            || observed.revision.get() != expected
        {
            return Err(fail());
        }
    } else if payload["action"] == "codec-renegotiate" {
        if observed.codecs.is_none()
            || !observed.resources_changed
            || expected.checked_add(1) != Some(observed.revision.get())
        {
            return Err(fail());
        }
    } else if payload["action"] == "quality" {
        if observed.codecs.is_some()
            || observed.quality.map(u64::from) != payload["quality_interval_ms"].as_u64()
            || observed.revision.get() != expected
        {
            return Err(fail());
        }
    } else {
        if observed.codecs.is_some()
            || observed.quality.is_some()
            || expected.checked_add(1) != Some(observed.revision.get())
            || serde_json::to_value(observed.action).map_err(|_| fail())? != payload["action"]
        {
            return Err(fail());
        }
        connect_protocol::native_gates::validate_observation(payload, &observed.gates)
            .map_err(|_| fail())?;
    }
    Ok(())
}
pub(super) fn decode(
    value: Value,
    client: &Client,
    key: &OperationKey,
) -> Result<ConnectionUpdateReceipt> {
    let observed = parse(&value, key)?;
    client.shared.record_connection_update(key, &observed)?;
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let outcome = if let Some(ice_generation) = observed.ice {
        ConnectionUpdateOutcome::IceRestart { ice_generation }
    } else if let Some(bindings) = observed.codecs {
        let mut streams = Vec::new();
        for binding in bindings {
            let data = binding.medium == "data";
            streams.push(CodecObservation {
                parameters: NegotiatedStream {
                    id: binding.stream_id.parse().map_err(|_| fail())?,
                    medium: serde_json::from_value(json!(binding.medium)).map_err(|_| fail())?,
                    direction: serde_json::from_value::<Direction>(json!(binding.direction))
                        .map_err(|_| fail())?
                        .reverse(),
                    codec: binding.codec,
                    stream_local_id: (!data).then_some(binding.stream_local_id),
                },
                stream_generation: binding.stream_generation,
                connection_generation: binding.connection_generation,
            });
        }
        ConnectionUpdateOutcome::Codecs {
            resources_changed: observed.resources_changed,
            streams,
        }
    } else if let Some(interval_ms) = observed.quality {
        ConnectionUpdateOutcome::Quality { interval_ms }
    } else {
        let mut streams = Vec::new();
        for gate in observed.gates {
            let binding = gate.binding;
            streams.push(StreamGateObservation {
                parameters: NegotiatedStream {
                    id: binding.stream_id.parse().map_err(|_| fail())?,
                    medium: serde_json::from_value(json!(binding.medium)).map_err(|_| fail())?,
                    direction: serde_json::from_value::<Direction>(json!(binding.direction))
                        .map_err(|_| fail())?
                        .reverse(),
                    codec: binding.codec,
                    stream_local_id: Some(binding.stream_local_id),
                },
                stream_generation: binding.stream_generation,
                connection_generation: binding.connection_generation,
                state: serde_json::from_value(json!(gate.state)).map_err(|_| fail())?,
                source_muted: gate.source_muted,
                held_direction: gate
                    .held_direction
                    .map(|d| {
                        serde_json::from_value::<Direction>(json!(d))
                            .map(Direction::reverse)
                            .map_err(|_| fail())
                    })
                    .transpose()?,
            });
        }
        ConnectionUpdateOutcome::Gates {
            action: observed.action.ok_or_else(fail)?,
            streams,
        }
    };
    // Receipt reads can advance matching owned gates, never install bindings or reset newer facts.
    Ok(ConnectionUpdateReceipt {
        connection: Connection::new(
            client.clone(),
            key.scope.connection_id.clone().ok_or_else(fail)?,
            key.scope.session_id.clone().ok_or_else(fail)?,
            key.scope.conversation_id.clone().ok_or_else(fail)?,
        ),
        connection_generation: observed.generation,
        stream_revision: observed.revision,
        outcome,
    })
}

#[cfg(test)]
mod ice_receipt_tests {
    use super::*;
    fn key() -> OperationKey {
        OperationKey {
            operation_id: "original-ice".parse().unwrap(),
            command: "connection.update".into(),
            scope: OperationScope {
                conversation_id: Some("conversation".parse().unwrap()),
                session_id: Some("session".parse().unwrap()),
                connection_id: Some("connection".parse().unwrap()),
            },
        }
    }
    fn receipt() -> Value {
        json!({"connection_id":"connection","connection_generation":7,"stream_revision":4,"resources_changed":false,"action":"ice-restart","ice_generation":3})
    }
    #[test]
    fn historical_ice_counter_decodes_without_any_local_gate_codec_or_resource_effect() {
        let key = key();
        let observed = parse(&receipt(), &key).unwrap();
        assert_eq!(observed.ice, Some(3));
        assert_eq!(observed.revision.get(), 4);
        assert_eq!(observed.generation, 7);
        assert!(
            observed.action.is_none()
                && observed.gates.is_empty()
                && observed.codecs.is_none()
                && observed.quality.is_none()
                && !observed.resources_changed
        );
        let intent = json!({"action":"ice-restart","details":{"expected_ice_generation":2,"setup":{"kind":"websocket+webrtc","sdp":"v=0\r\n"}}});
        validate_intent(&observed, &intent, 4, &key).unwrap();
        assert!(validate_intent(&observed, &intent, 3, &key).is_err());
        let mut wrong = intent;
        wrong["details"]["expected_ice_generation"] = json!(1);
        assert!(validate_intent(&observed, &wrong, 4, &key).is_err());
    }
    #[test]
    fn historical_ice_receipts_refuse_unsafe_mixed_or_foreign_metadata() {
        for (field, value) in [
            ("ice_generation", json!(0)),
            ("ice_generation", json!(1u64 << 53)),
            ("connection_id", json!("foreign")),
            ("resources_changed", json!(true)),
            ("gate_streams", json!([])),
            ("codec_bindings", json!([])),
            ("quality_interval_ms", json!(1000)),
            ("setup", json!({"sdp":"PRIVATE"})),
        ] {
            let mut bad = receipt();
            bad[field] = value;
            assert!(parse(&bad, &key()).is_err(), "{field}");
        }
        let mut bad = receipt();
        bad["action"] = json!("quality");
        assert!(parse(&bad, &key()).is_err());
    }
}
