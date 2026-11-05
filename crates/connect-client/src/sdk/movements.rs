//! Typed handoff and device movement preserve captured topology and original intent.
use super::consultations::Continuation;
use super::*;
pub use connect_protocol::replacement_video::{ReplacementVideo, ReplacementVideoStream};
use connect_protocol::transitions::{
    OutgoingDisposition, RoleScope, TransitionCommand, TransitionIntent,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "lowercase", deny_unknown_fields)]
pub enum HandoffRole {
    Session,
    Conversation {
        expected_conversation_revision: ConversationRevision,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outgoing {
    Leave,
    Observe,
}
#[derive(Clone)]
pub struct HandoffInput {
    pub via: ConnectionId,
    pub from_participant: ParticipantId,
    pub to: ParticipantId,
    pub retain: ConnectionId,
    pub outgoing: Outgoing,
    pub role: HandoffRole,
    pub expected_topology_revision: TopologyRevision,
    pub candidate: Option<ConnectionId>,
    pub consultation: Option<OperationReference<ConsultationResult>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum MoveTarget {
    Endpoint(EndpointId),
    Candidate(ConnectionId),
}
#[derive(Clone, Debug)]
pub struct MoveInput {
    pub video: Option<ReplacementVideo>,
    pub retain: ConnectionId,
    pub target: MoveTarget,
    pub expected_topology_revision: TopologyRevision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandoffRoleScope {
    Session,
    Conversation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffCleanup {
    Pending,
    NotRequested,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffRoleChange {
    /// Captured native participant authority revision, not a topology assertion.
    pub participant_revision: u64,
    pub role_scope: HandoffRoleScope,
    pub outgoing: Outgoing,
    pub source_participant: ParticipantId,
    pub target_participant: ParticipantId,
    pub cleanup: HandoffCleanup,
}
#[derive(Clone, Debug)]
pub struct HandoffResult {
    pub topology: TopologyChange,
    pub role_change: Option<HandoffRoleChange>,
}
#[derive(Clone, Debug)]
pub struct MoveBindingObservation {
    pub generation: u64,
    pub participant_id: ParticipantId,
    pub membership_epoch: u64,
    pub endpoint_id: EndpointId,
    pub device_id: DeviceId,
}
/// Historical activation outcome, never current media permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MoveActivationState {
    Ready,
    Stopped,
}
#[derive(Clone, Debug)]
pub struct MoveResult {
    /// None for the separately implemented telephone completion contract.
    pub activation_state: Option<MoveActivationState>,
    pub connection: Connection,
    pub topology: TopologyChange,
    pub binding: MoveBindingObservation,
}
#[derive(Deserialize)]
struct WireTopology {
    retained_connection_ids: Vec<ConnectionId>,
    admitted_connection_ids: Vec<ConnectionId>,
    retired_connection_ids: Vec<ConnectionId>,
    session_revision: TopologyRevision,
    conversation_revision: Option<ConversationRevision>,
}
#[derive(Deserialize)]
struct WireHandoff {
    #[serde(flatten)]
    topology: WireTopology,
    native_handoff: Option<HandoffRoleChange>,
}
#[derive(Deserialize)]
struct WireMove {
    #[serde(flatten)]
    topology: WireTopology,
    connection_id: ConnectionId,
    replaces: ConnectionId,
    generation: u64,
    participant_id: ParticipantId,
    membership_epoch: u64,
    endpoint_id: EndpointId,
    device_id: DeviceId,
    readiness: Value,
    activation_state: Option<MoveActivationState>,
    physical_readiness: Option<Value>,
}
fn bad(key: &OperationKey, submitted: bool) -> Error {
    Error::request(ErrorKind::Protocol, Some(key), submitted)
}
fn bounded(value: &Value, key: &OperationKey, extra: &[&str]) -> Result<()> {
    let allowed = [
        "retained_connection_ids",
        "admitted_connection_ids",
        "retired_connection_ids",
        "session_revision",
        "conversation_revision",
    ];
    if !connections::valid_scope(&key.scope)
        || serde_json::to_vec(value).map_err(|_| bad(key, true))?.len() > 32 * 1024
        || value.as_object().is_none_or(|m| {
            m.keys()
                .any(|k| !allowed.contains(&k.as_str()) && !extra.contains(&k.as_str()))
        })
    {
        return Err(bad(key, true));
    }
    Ok(())
}
fn validate_topology(
    w: &WireTopology,
    key: &OperationKey,
    stopped: Option<&ConnectionId>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut retired = BTreeSet::new();
    if w.session_revision.get() == 0
        || w.retained_connection_ids.is_empty()
        || w.admitted_connection_ids.len() != 1
        || w.retained_connection_ids.len()
            + w.admitted_connection_ids.len()
            + w.retired_connection_ids.len()
            > 100
        || w.retained_connection_ids
            .iter()
            .chain(&w.admitted_connection_ids)
            .any(|id| !ids.insert(id))
        || w.retired_connection_ids
            .iter()
            .any(|id| !retired.insert(id) || (ids.contains(id) && Some(id) != stopped))
    {
        return Err(bad(key, true));
    }
    Ok(())
}
fn positive(n: u64) -> bool {
    (1..=9_007_199_254_740_991).contains(&n)
}
fn handoff(value: &Value, key: &OperationKey) -> Result<WireHandoff> {
    bounded(value, key, &["native_handoff"])?;
    if key.command != "session.handoff" {
        return Err(bad(key, true));
    }
    let w: WireHandoff = serde_json::from_value(value.clone()).map_err(|_| bad(key, true))?;
    validate_topology(&w.topology, key, None)?;
    if let Some(change) = &w.native_handoff {
        if !positive(change.participant_revision)
            || change.source_participant == change.target_participant
            || (change.role_scope == HandoffRoleScope::Conversation)
                != w.topology.conversation_revision.is_some()
            || (change.outgoing == Outgoing::Leave) != (change.cleanup == HandoffCleanup::Pending)
        {
            return Err(bad(key, true));
        }
    }
    Ok(w)
}
fn movement(value: &Value, key: &OperationKey) -> Result<WireMove> {
    bounded(
        value,
        key,
        &[
            "connection_id",
            "replaces",
            "generation",
            "participant_id",
            "membership_epoch",
            "endpoint_id",
            "device_id",
            "readiness",
            "activation_state",
            "physical_readiness",
            "native_activation",
        ],
    )?;
    if key.command != "connection.replace" {
        return Err(bad(key, true));
    }
    let w: WireMove = serde_json::from_value(value.clone()).map_err(|_| bad(key, true))?;
    let stopped = w.activation_state == Some(MoveActivationState::Stopped);
    // These are two current producer contracts, not a version downgrade. Native
    // activation fields must appear together; telephone has neither field.
    let activation_valid = match w.activation_state {
        Some(MoveActivationState::Ready) => {
            w.readiness.is_object()
                && w.physical_readiness.as_ref() == Some(&w.readiness)
                && !w.topology.retired_connection_ids.contains(&w.connection_id)
        }
        Some(MoveActivationState::Stopped) => {
            w.readiness.is_null()
                && value.get("readiness").is_some()
                && w.physical_readiness.as_ref().is_some_and(Value::is_object)
                && w.topology.retired_connection_ids.contains(&w.connection_id)
        }
        None => {
            !value.as_object().unwrap().contains_key("activation_state")
                && !value
                    .as_object()
                    .unwrap()
                    .contains_key("physical_readiness")
                && w.readiness.is_object()
        }
    };
    if !activation_valid {
        return Err(bad(key, true));
    }
    validate_native_activation(value, &w, key)?;
    validate_topology(&w.topology, key, stopped.then_some(&w.connection_id))?;
    if !positive(w.generation)
        || !positive(w.membership_epoch)
        || w.topology.conversation_revision.is_some()
        || key.scope.connection_id.as_ref() != Some(&w.replaces)
        || w.topology.admitted_connection_ids != [w.connection_id.clone()]
        || !w.topology.retired_connection_ids.contains(&w.replaces)
    {
        return Err(bad(key, true));
    }
    Ok(w)
}
// This validates historical result metadata. The physical reader independently
// matches negotiated incarnation/handles before using any sequence membership.
fn validate_native_activation(value: &Value, w: &WireMove, key: &OperationKey) -> Result<()> {
    let proof = value.get("native_activation");
    if w.activation_state.is_none() {
        return if proof.is_none() {
            Ok(())
        } else {
            Err(bad(key, true))
        };
    }
    let proof = proof.ok_or_else(|| bad(key, true))?;
    if proof.is_null() && w.activation_state == Some(MoveActivationState::Stopped) {
        return Ok(());
    }
    let boundary: connect_protocol::replacement_activation::NativeActivationBoundary =
        serde_json::from_value(proof.clone()).map_err(|_| bad(key, true))?;
    boundary.validate().map_err(|_| bad(key, true))?;
    if boundary.operation_id != key.operation_id.as_str()
        || Some(boundary.source_connection_id.as_str())
            != key.scope.connection_id.as_ref().map(|id| id.as_str())
        || boundary.source_connection_id != w.replaces.as_str()
        || boundary.target_connection_id != w.connection_id.as_str()
        || boundary.target_generation != w.generation
        || w.physical_readiness
            .as_ref()
            .and_then(|r| r["stream_revision"].as_u64())
            != Some(boundary.stream_revision)
    {
        return Err(bad(key, true));
    }
    if let connect_protocol::replacement_activation::NativeActivationAudience::Public { streams } =
        &boundary.audience
    {
        let actual = w
            .physical_readiness
            .as_ref()
            .and_then(|r| r["streams"].as_array())
            .ok_or_else(|| bad(key, true))?;
        if streams.iter().any(|s| {
            !actual
                .iter()
                .any(|id| id.as_str() == Some(s.stream_id.as_str()))
        }) {
            return Err(bad(key, true));
        }
    }
    Ok(())
}
pub(super) fn valid_continuation(
    command: &str,
    scope: &OperationScope,
    context: Option<&Continuation>,
) -> bool {
    if !connections::valid_scope(scope) {
        return false;
    }
    match (command, context) {
        (
            "session.handoff",
            Some(Continuation::Handoff {
                retained_connection_id,
                source_participant,
                target_participant,
                candidate,
                role,
                ..
            }),
        ) => {
            scope.connection_id.as_ref() != Some(retained_connection_id)
                && source_participant != target_participant
                && candidate.as_ref().is_none_or(|c| {
                    Some(c) != scope.connection_id.as_ref() && c != retained_connection_id
                })
                && match role {
                    HandoffRole::Session => true,
                    HandoffRole::Conversation {
                        expected_conversation_revision,
                    } => expected_conversation_revision.get() < 9_007_199_254_740_991,
                }
        }
        (
            "connection.replace",
            Some(Continuation::Replacement {
                retained_connection_id,
                target,
            }),
        ) => {
            scope.connection_id.as_ref() != Some(retained_connection_id)
                && match target {
                    MoveTarget::Endpoint(_) => true,
                    MoveTarget::Candidate(c) => {
                        Some(c) != scope.connection_id.as_ref() && c != retained_connection_id
                    }
                }
        }
        _ => false,
    }
}
pub(super) fn validate_completion(
    value: &Value,
    key: &OperationKey,
    context: Option<&Continuation>,
) -> Result<()> {
    if !valid_continuation(&key.command, &key.scope, context) {
        return Err(bad(key, true));
    }
    match context {
        Some(Continuation::Handoff {
            retained_connection_id,
            source_participant,
            target_participant,
            candidate,
            role,
            outgoing,
        }) => {
            let w = handoff(value, key)?;
            if !w
                .topology
                .retained_connection_ids
                .contains(retained_connection_id)
                || candidate
                    .as_ref()
                    .is_some_and(|id| w.topology.admitted_connection_ids != [id.clone()])
                || (*outgoing == Outgoing::Leave)
                    != key
                        .scope
                        .connection_id
                        .as_ref()
                        .is_some_and(|id| w.topology.retired_connection_ids.contains(id))
            {
                return Err(bad(key, true));
            }
            let scope = match role {
                HandoffRole::Session => {
                    if w.topology.conversation_revision.is_some() {
                        return Err(bad(key, true));
                    }
                    HandoffRoleScope::Session
                }
                HandoffRole::Conversation {
                    expected_conversation_revision,
                } => {
                    if w.topology.conversation_revision.map(|v| v.get())
                        != expected_conversation_revision.get().checked_add(1)
                    {
                        return Err(bad(key, true));
                    }
                    HandoffRoleScope::Conversation
                }
            };
            if w.native_handoff.as_ref().is_some_and(|c| {
                &c.source_participant != source_participant
                    || &c.target_participant != target_participant
                    || c.role_scope != scope
                    || &c.outgoing != outgoing
            }) {
                return Err(bad(key, true));
            }
        }
        Some(Continuation::Replacement {
            retained_connection_id,
            target,
        }) => {
            let w = movement(value, key)?;
            if !w
                .topology
                .retained_connection_ids
                .contains(retained_connection_id)
                || match target {
                    MoveTarget::Endpoint(id) => &w.endpoint_id != id,
                    MoveTarget::Candidate(id) => &w.connection_id != id,
                }
            {
                return Err(bad(key, true));
            }
        }
        _ => return Err(bad(key, true)),
    }
    Ok(())
}
fn session(client: &Client, key: &OperationKey) -> Result<Session> {
    Ok(client
        .conversation_by_id(
            key.scope
                .conversation_id
                .clone()
                .ok_or_else(|| bad(key, true))?,
        )
        .session_by_id(key.scope.session_id.clone().ok_or_else(|| bad(key, true))?))
}
fn topology(w: &WireTopology, s: &Session) -> TopologyChange {
    TopologyChange {
        retained: w
            .retained_connection_ids
            .iter()
            .cloned()
            .map(|id| s.connection_by_id(id))
            .collect(),
        admitted: w
            .admitted_connection_ids
            .iter()
            .cloned()
            .map(|id| s.connection_by_id(id))
            .collect(),
        retired: w.retired_connection_ids.clone(),
        topology_revision: w.session_revision,
        conversation_revision: w.conversation_revision,
    }
}
pub(super) fn decode_handoff(
    value: Value,
    client: &Client,
    key: &OperationKey,
) -> Result<HandoffResult> {
    let w = handoff(&value, key)?;
    let s = session(client, key)?;
    Ok(HandoffResult {
        topology: topology(&w.topology, &s),
        role_change: w.native_handoff,
    })
}
pub(super) fn decode_move(value: Value, client: &Client, key: &OperationKey) -> Result<MoveResult> {
    let w = movement(&value, key)?;
    let s = session(client, key)?;
    Ok(MoveResult {
        activation_state: w.activation_state,
        connection: s.connection_by_id(w.connection_id),
        topology: topology(&w.topology, &s),
        binding: MoveBindingObservation {
            generation: w.generation,
            participant_id: w.participant_id,
            membership_epoch: w.membership_epoch,
            endpoint_id: w.endpoint_id,
            device_id: w.device_id,
        },
    })
}
impl Session {
    pub async fn handoff(
        &self,
        input: HandoffInput,
        mut options: MutationOptions,
    ) -> Result<Operation<HandoffResult>> {
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: "session.handoff".into(),
            scope: OperationScope {
                conversation_id: Some(self.conversation_id().clone()),
                session_id: Some(self.id().clone()),
                connection_id: Some(input.via.clone()),
            },
        };
        let invalid = || Error::request(ErrorKind::Validation, Some(&key), false);
        options.request.validate().map_err(|_| invalid())?;
        let (role_scope, expected_conversation_revision) = match &input.role {
            HandoffRole::Session => (RoleScope::Session, None),
            HandoffRole::Conversation {
                expected_conversation_revision,
            } => (
                RoleScope::Conversation,
                Some(expected_conversation_revision.get()),
            ),
        };
        let intent = TransitionIntent::Handoff {
            source_participant: input.from_participant.as_str().into(),
            target_participant: input.to.as_str().into(),
            retained_connid: input.retain.as_str().into(),
            candidate_connid: input.candidate.as_ref().map(|id| id.as_str().into()),
            outgoing: match input.outgoing {
                Outgoing::Leave => OutgoingDisposition::Leave,
                Outgoing::Observe => OutgoingDisposition::Observe,
            },
            role_scope,
            expected_conversation_revision,
        };
        let mut preflight = TransitionCommand {
            operation_id: key.operation_id.as_str().into(),
            conversation_id: self.conversation_id().as_str().into(),
            session_id: self.id().as_str().into(),
            connection_id: Some(input.via.as_str().into()),
            expected_revision: input.expected_topology_revision.get(),
            intent: intent.clone(),
            payload_extensions: options.extensions.clone().unwrap_or_default(),
        };
        preflight.normalize().map_err(|_| invalid())?;
        let until = tokio::time::Instant::now() + options.request.timeout;
        if let Some(reference) = input.consultation {
            if reference.key.command != "session.consult"
                || reference.key.scope.conversation_id != key.scope.conversation_id
                || reference.key.scope.session_id != key.scope.session_id
                || reference.key.operation_id == key.operation_id
                || options
                    .extensions
                    .as_ref()
                    .is_some_and(|m| m.contains_key("consultation_record_id"))
            {
                return Err(invalid());
            }
            let record = match &reference.continuation {
                Some(Continuation::Consultation { record_id, .. }) => record_id.clone(),
                _ => return Err(invalid()),
            };
            let record = if let Some(record) = record {
                record
            } else {
                Operation::new(self.client().clone(), reference)
                    .status(options.request.clone())
                    .await
                    .map_err(|mut e| {
                        e.operation = Some(key.clone());
                        e.certainty = Certainty::NotSubmitted;
                        e
                    })?
                    .record_id
                    .ok_or_else(|| {
                        Error::request(ErrorKind::ReceiptUnavailable, Some(&key), false)
                    })?
            };
            options
                .extensions
                .get_or_insert_default()
                .insert("consultation_record_id".into(), json!(record));
        }
        let remaining = until.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::request(ErrorKind::Deadline, Some(&key), false));
        }
        options.request.timeout = remaining;
        let context = Continuation::Handoff {
            retained_connection_id: input.retain,
            source_participant: input.from_participant,
            target_participant: input.to,
            candidate: input.candidate,
            role: input.role,
            outgoing: input.outgoing,
        };
        self.submit_transition(
            self.client().operation_kinds().session_handoff,
            input.via,
            input.expected_topology_revision,
            intent,
            context,
            options,
        )
        .await
    }
}
impl Connection {
    pub async fn replace_device(
        &self,
        input: MoveInput,
        options: MutationOptions,
    ) -> Result<Operation<MoveResult>> {
        let (endpoint_id, candidate_connid) = match &input.target {
            MoveTarget::Endpoint(id) => (Some(id.as_str().into()), None),
            MoveTarget::Candidate(id) => (None, Some(id.as_str().into())),
        };
        let intent = TransitionIntent::Replace {
            retained_connid: input.retain.as_str().into(),
            endpoint_id,
            candidate_connid,
            video: input.video,
        };
        let context = Continuation::Replacement {
            retained_connection_id: input.retain,
            target: input.target,
        };
        let s = self
            .client()
            .conversation_by_id(self.conversation_id().clone())
            .session_by_id(self.session_id().clone());
        s.submit_transition(
            self.client().operation_kinds().connection_replace,
            self.id().clone(),
            input.expected_topology_revision,
            intent,
            context,
            options,
        )
        .await
    }
}

#[cfg(test)]
mod activation_tests {
    use super::*;
    #[test]
    fn native_activation_proof_is_required_bound_and_never_a_media_permission() {
        let ready: Value = serde_json::from_str(include_str!(
            "../../../../sdk/tests/fixtures/native_move_activation.json"
        ))
        .unwrap();
        let key = OperationKey {
            operation_id: "original".parse().unwrap(),
            command: "connection.replace".into(),
            scope: OperationScope {
                conversation_id: Some("conversation".parse().unwrap()),
                session_id: Some("session".parse().unwrap()),
                connection_id: Some("source".parse().unwrap()),
            },
        };
        movement(&ready, &key).unwrap();
        for (path, replacement) in [
            ("/native_activation", Value::Null),
            ("/native_activation/operation_id", json!("foreign")),
            ("/native_activation/source_connection_id", json!("foreign")),
            ("/native_activation/target_connection_id", json!("foreign")),
            ("/native_activation/target_generation", json!(3)),
            ("/native_activation/stream_revision", json!(5)),
            ("/native_activation/profile", json!("unknown")),
            (
                "/native_activation/audience/streams/0/stream_id",
                json!("foreign"),
            ),
            (
                "/native_activation/audience/streams/0/physical_incarnation",
                json!("0".repeat(32)),
            ),
            (
                "/native_activation/audience/streams/0/public_sequence_floor",
                json!(5),
            ),
            (
                "/native_activation/audience/streams/0/submission_observed",
                json!(true),
            ),
            ("/native_activation/audience/streams", json!([])),
        ] {
            let mut changed = ready.clone();
            *changed.pointer_mut(path).unwrap() = replacement;
            assert!(movement(&changed, &key).is_err(), "{path}");
        }
        let mut missing = ready.clone();
        missing.as_object_mut().unwrap().remove("native_activation");
        assert!(movement(&missing, &key).is_err());
        let mut unknown = ready.clone();
        unknown["native_activation"]["permission"] = json!(true);
        assert!(movement(&unknown, &key).is_err());
        let mut duplicate = ready.clone();
        let cuts = duplicate["native_activation"]["audience"]["streams"]
            .as_array_mut()
            .unwrap();
        cuts.push(cuts[0].clone());
        assert!(movement(&duplicate, &key).is_err());
        let mut stopped = ready.clone();
        stopped["activation_state"] = json!("stopped");
        stopped["readiness"] = Value::Null;
        stopped["retired_connection_ids"] = json!(["source", "candidate"]);
        movement(&stopped, &key).unwrap();
        stopped["native_activation"] = Value::Null;
        movement(&stopped, &key).unwrap();
        stopped.as_object_mut().unwrap().remove("native_activation");
        assert!(movement(&stopped, &key).is_err());
        for audience in [
            json!({"scope":"private-consultation","original_operation":"parent"}),
            json!({"scope":"unsupported","reason":"non-native-adapter"}),
        ] {
            let mut value = ready.clone();
            value["native_activation"]["audience"] = audience;
            movement(&value, &key).unwrap();
        }
        let mut observed = json!({"state":"completed","command_type":"connection.replace","record_id":"record_original","expected_revision":3,"result":ready});
        consultations::validate_observation(&observed, &key, None).unwrap();
        observed["record_id"] = json!("foreign");
        assert!(consultations::validate_observation(&observed, &key, None).is_err());
    }
    #[test]
    fn native_activation_completion_preserves_ready_stopped_and_refuses_contradictions() {
        let ready: Value = serde_json::from_str(include_str!(
            "../../../../sdk/tests/fixtures/native_move_activation.json"
        ))
        .unwrap();
        let key = OperationKey {
            operation_id: "original".parse().unwrap(),
            command: "connection.replace".into(),
            scope: OperationScope {
                conversation_id: Some("conversation".parse().unwrap()),
                session_id: Some("session".parse().unwrap()),
                connection_id: Some("source".parse().unwrap()),
            },
        };
        let context = Continuation::Replacement {
            retained_connection_id: "remote".parse().unwrap(),
            target: MoveTarget::Endpoint("endpoint_new".parse().unwrap()),
        };
        let mut stopped = ready.clone();
        stopped["activation_state"] = json!("stopped");
        stopped["readiness"] = Value::Null;
        stopped["retired_connection_ids"] = json!(["source", "candidate"]);
        for (v, state) in [
            (&ready, MoveActivationState::Ready),
            (&stopped, MoveActivationState::Stopped),
        ] {
            validate_completion(v, &key, Some(&context)).unwrap();
            assert_eq!(movement(v, &key).unwrap().activation_state, Some(state));
        }
        let mut telephone = ready.clone();
        for field in [
            "activation_state",
            "physical_readiness",
            "native_activation",
        ] {
            telephone.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(movement(&telephone, &key).unwrap().activation_state, None);
        // No generalized topology relaxation, partial field set, invented state,
        // stale readiness or nullable-state downgrade is accepted.
        for (base, field, value) in [
            (
                &ready,
                "retired_connection_ids",
                json!(["source", "candidate"]),
            ),
            (&ready, "physical_readiness", json!({"streams":["foreign"]})),
            (&ready, "readiness", Value::Null),
            (&ready, "activation_state", Value::Null),
            (&ready, "activation_state", json!("prepared")),
            (&stopped, "readiness", ready["readiness"].clone()),
            (&stopped, "physical_readiness", Value::Null),
            (&stopped, "retired_connection_ids", json!(["source"])),
            (
                &stopped,
                "retired_connection_ids",
                json!(["source", "candidate", "candidate"]),
            ),
            (
                &stopped,
                "retired_connection_ids",
                json!(["source", "candidate", "remote"]),
            ),
            (&stopped, "retained_connection_ids", json!(["candidate"])),
            (&stopped, "admitted_connection_ids", json!(["foreign"])),
            (&stopped, "endpoint_id", json!("foreign")),
        ] {
            let mut bad = base.clone();
            bad[field] = value;
            assert!(
                validate_completion(&bad, &key, Some(&context)).is_err(),
                "{field}"
            );
        }
        for field in ["activation_state", "physical_readiness", "readiness"] {
            let mut bad = stopped.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(movement(&bad, &key).is_err(), "missing {field}");
        }
    }
}
