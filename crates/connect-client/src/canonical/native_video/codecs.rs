//! Resource adoption requires the original intent, exact signed callback,
//! successful confirmation write and matching completed operation together.
use super::*;
use crate::sdk::{
    connection_updates, native_codec_negotiation as negotiation, OperationKey, OperationScope,
    ParticipantId, StreamId,
};
use connect_protocol::native_streams::{CodecBinding, Command, Intent};
use std::collections::BTreeSet;

pub(super) struct Owner {
    participant: ParticipantId,
    local: Value,
    remote: Value,
    handles: BTreeMap<StreamId, u16>,
    generations: BTreeMap<StreamId, u64>,
    reserved: BTreeSet<u16>,
    generation: u64,
    pending: Option<Pending>,
    finished: BTreeMap<String, (u64, Value)>,
}
struct Pending {
    key: OperationKey,
    payload: Value,
    expected: u64,
    request: String,
    callback: Option<Value>,
    candidate: Option<negotiation::Candidate>,
    written: bool,
    receipt: Option<connection_updates::Observed>,
    before: BTreeMap<(u16, bool), Notice>,
    highwater: BTreeMap<(u16, bool), u64>,
    notices: BTreeMap<(u16, bool), Notice>,
}
impl Owner {
    pub fn new(s: &State) -> Result<Self> {
        let (participant, local, remote, handles) = protocol(s.negotiating.context())?;
        let selected = s.selected.as_ref().ok_or(ClientError::Protocol)?;
        let generation = selected
            .video_feedback
            .values()
            .next()
            .ok_or(ClientError::Protocol)?
            .connection_generation;
        let generations = handles
            .keys()
            .map(|id| {
                (
                    id.clone(),
                    selected
                        .video_feedback
                        .get(id)
                        .map_or(1, |v| v.stream_generation),
                )
            })
            .collect();
        let reserved = handles.values().copied().collect();
        Ok(Self {
            participant,
            local,
            remote,
            handles,
            generations,
            reserved,
            generation,
            pending: None,
            finished: BTreeMap::new(),
        })
    }
    pub fn has_operation(&self, operation: &str) -> bool {
        self.finished.contains_key(operation)
            || self
                .pending
                .as_ref()
                .is_some_and(|p| p.key.operation_id.as_str() == operation)
    }
}
pub(super) fn matches(s: &State, b: &CodecBinding) -> bool {
    let (Some(owner), Some(selected)) = (&s.codecs, &s.selected) else {
        return false;
    };
    negotiation::matches(&selected.streams, &owner.handles, b)
        && b.connection_generation == owner.generation
        && b.stream_id
            .parse::<StreamId>()
            .ok()
            .and_then(|id| owner.generations.get(&id))
            .copied()
            == Some(b.stream_generation)
}
pub(super) fn outgoing(s: &mut State, frame: &Value) -> Result<()> {
    let operation = frame["operation_id"]
        .as_str()
        .ok_or(ClientError::Protocol)?;
    if s.controls.has_operation(operation) {
        return Err(ClientError::Protocol);
    }
    let expected = frame["expected_revision"]
        .as_u64()
        .filter(|n| (1..9_007_199_254_740_991).contains(n))
        .ok_or(ClientError::Protocol)?;
    let payload = frame["payload"].clone();
    let owner = s.codecs.as_mut().ok_or(ClientError::Protocol)?;
    if let Some((revision, original)) = owner.finished.get(operation) {
        return if *revision == expected && *original == payload {
            Ok(())
        } else {
            Err(ClientError::Protocol)
        };
    }
    if let Some(p) = &owner.pending {
        return if p.key.operation_id.as_str() == operation
            && p.expected == expected
            && p.payload == payload
        {
            Ok(())
        } else {
            Err(ClientError::Protocol)
        };
    }
    if s.ended
        || s.suspended
        || expected
            < s.selected
                .as_ref()
                .ok_or(ClientError::Protocol)?
                .stream_revision
                .get()
    {
        return Err(ClientError::Protocol);
    }
    let intent = Intent {
        operation_id: operation.into(),
        conversation_id: s
            .scope
            .conversation_id
            .clone()
            .ok_or(ClientError::Protocol)?,
        session_id: s.scope.session_id.clone().ok_or(ClientError::Protocol)?,
        connection_id: s.scope.connection_id.clone().ok_or(ClientError::Protocol)?,
        expected_revision: Some(expected),
        by_participant: None,
        command: Command::Update,
        payload: payload.clone(),
    };
    intent.validate().map_err(|_| ClientError::Protocol)?;
    let key = OperationKey {
        operation_id: operation.parse().map_err(|_| ClientError::Protocol)?,
        command: "connection.update".into(),
        scope: OperationScope {
            conversation_id: Some(
                intent
                    .conversation_id
                    .parse()
                    .map_err(|_| ClientError::Protocol)?,
            ),
            session_id: Some(
                intent
                    .session_id
                    .parse()
                    .map_err(|_| ClientError::Protocol)?,
            ),
            connection_id: Some(
                intent
                    .connection_id
                    .parse()
                    .map_err(|_| ClientError::Protocol)?,
            ),
        },
    };
    owner.pending = Some(Pending {
        key,
        payload,
        expected,
        request: frame["id"].as_str().ok_or(ClientError::Protocol)?.into(),
        callback: None,
        candidate: None,
        written: false,
        receipt: None,
        before: s.active.clone(),
        highwater: s
            .notices
            .iter()
            .map(|(k, n)| (*k, n.history.revision))
            .collect(),
        notices: BTreeMap::new(),
    });
    s.suspended = true;
    s.active.clear();
    Ok(())
}
pub(super) fn offer(s: &mut State, frame: &Envelope) -> Result<()> {
    let Some(owner) = s.codecs.as_mut() else {
        return Ok(());
    };
    let Some(p) = owner.pending.as_mut() else {
        return Ok(());
    };
    if frame.scope(Scope::Operation) != Some(p.key.operation_id.as_str()) {
        return Ok(());
    }
    if p.callback.is_some() {
        return Err(ClientError::Protocol);
    }
    let candidate = protocol(negotiation::prepare(
        negotiation::Base {
            selected: s.selected.as_ref().ok_or(ClientError::Protocol)?,
            participant: &owner.participant,
            local_capabilities: &owner.local,
            server_capabilities: &owner.remote,
            wire_handles: &owner.handles,
            generations: &owner.generations,
            reserved_handles: &owner.reserved,
            connection_generation: owner.generation,
        },
        &p.key,
        &p.payload,
        p.expected,
        &p.request,
        frame,
    ))?;
    // Never reuse a handle already observed even if later confirmation fails.
    owner
        .reserved
        .extend(candidate.wire_handles.values().copied());
    if owner.reserved.len() > 4096 {
        return Err(ClientError::Protocol);
    }
    p.callback = Some(frame.original().clone());
    p.candidate = Some(candidate);
    Ok(())
}
pub(super) fn check_confirmation(
    s: &State,
    frame: &Envelope,
    answer: &payloads::ConnectionAnswer,
    root: &CommandScope,
) -> Result<bool> {
    let Some(p) = s.codecs.as_ref().and_then(|o| o.pending.as_ref()) else {
        return Ok(false);
    };
    if root.operation_id.as_deref() != Some(p.key.operation_id.as_str()) {
        return Ok(false);
    }
    let candidate = p.candidate.as_ref().ok_or(ClientError::Protocol)?;
    if s.ended
        || p.written
        || root.connection_id != s.scope.connection_id
        || root.session_id != s.scope.session_id
        || root.conversation_id != s.scope.conversation_id
        || root.expected_revision != Some(p.expected)
        || p.callback.as_ref() != Some(frame.original())
        || serde_json::to_value(answer).map_err(|_| ClientError::Protocol)?
            != serde_json::to_value(&candidate.answer).map_err(|_| ClientError::Protocol)?
    {
        return Err(ClientError::Protocol);
    }
    Ok(true)
}
pub(super) fn written(s: &mut State) -> Result<bool> {
    let Some(p) = s.codecs.as_mut().and_then(|o| o.pending.as_mut()) else {
        return Ok(false);
    };
    if s.ended || p.written || p.candidate.is_none() {
        return Err(ClientError::Protocol);
    }
    p.written = true;
    adopt(s)?;
    Ok(true)
}
pub(super) fn notice(s: &mut State, n: &Notice) -> Result<bool> {
    let Some(p) = s.codecs.as_mut().and_then(|o| o.pending.as_mut()) else {
        return Ok(false);
    };
    if p.candidate.as_ref().is_none_or(|c| !n.matches(&c.selected)) {
        return Ok(false);
    }
    let key = (n.handle, n.role == Role::Publisher);
    if p.notices
        .get(&key)
        .is_none_or(|old| old.history.revision < n.history.revision)
    {
        if p.notices.len() >= 64 && !p.notices.contains_key(&key) {
            return Err(ClientError::Protocol);
        }
        p.notices.insert(key, n.clone());
    }
    Ok(true)
}
pub(super) fn unselect(s: &mut State, ids: &[Value]) {
    if let Some(p) = s.codecs.as_mut().and_then(|o| o.pending.as_mut()) {
        p.before.retain(|(_, publisher), n| {
            *publisher
                || !ids
                    .iter()
                    .any(|id| id.as_str() == Some(n.key.stream_id.as_str()))
        });
    }
}
pub(super) fn observe(s: &mut State, frame: &Envelope) -> Result<()> {
    let Payload::OperationUpdated(update) = frame.payload() else {
        return Ok(());
    };
    let Some(owner) = s.codecs.as_mut() else {
        return Ok(());
    };
    let Some(p) = owner
        .pending
        .as_mut()
        .filter(|p| p.key.operation_id.as_str() == update.operation_id.as_str())
    else {
        return Ok(());
    };
    if frame
        .scope(Scope::Operation)
        .is_some_and(|v| v != p.key.operation_id.as_str())
        || update
            .command_type
            .as_deref()
            .is_some_and(|v| v != "connection.update")
        || update.expected_revision.is_some_and(|v| v != p.expected)
    {
        return Err(ClientError::Protocol);
    }
    let state = serde_json::to_value(update.state).map_err(|_| ClientError::Protocol)?;
    if state == "failed" || state == "cancelled" {
        let p = owner.pending.take().ok_or(ClientError::Protocol)?;
        remember(owner, &p);
        // A settled refusal leaves the last sealed resources in place. Retired
        // subscriptions/gates remain retired; no candidate epoch is installed.
        let selected = s.selected.as_ref().ok_or(ClientError::Protocol)?;
        for (key, n) in p.notices {
            if n.matches(selected)
                && s.notices
                    .get(&key)
                    .is_none_or(|old| old.history.revision < n.history.revision)
            {
                s.notices.insert(key, n);
            }
        }
        s.active = s
            .notices
            .iter()
            .filter(|(key, n)| {
                n.matches(selected)
                    && s.controls.admits(n)
                    && (p.before.contains_key(key)
                        || p.highwater.get(key).is_none_or(|r| n.history.revision > *r))
            })
            .map(|(k, n)| (*k, n.clone()))
            .collect();
        s.suspended = false;
        return Ok(());
    }
    if state != "completed" {
        return Ok(());
    }
    let observed = protocol(connection_updates::parse(
        update.result.as_ref().ok_or(ClientError::Protocol)?,
        &p.key,
    ))?;
    protocol(connection_updates::validate_intent(
        &observed, &p.payload, p.expected, &p.key,
    ))?;
    let c = p.candidate.as_ref().ok_or(ClientError::Protocol)?;
    let bindings = observed.codecs.as_ref().ok_or(ClientError::Protocol)?;
    if observed.generation != owner.generation
        || bindings.len() != c.selected.streams.len()
        || bindings.iter().any(|b| {
            !negotiation::matches(&c.selected.streams, &c.wire_handles, b)
                || b.stream_id
                    .parse::<StreamId>()
                    .ok()
                    .and_then(|id| c.generations.get(&id))
                    .copied()
                    != Some(b.stream_generation)
        })
    {
        return Err(ClientError::Protocol);
    }
    p.receipt = Some(observed);
    adopt(s)
}
fn remember(owner: &mut Owner, p: &Pending) {
    if owner.finished.len() >= 64 {
        if let Some(key) = owner.finished.keys().next().cloned() {
            owner.finished.remove(&key);
        }
    }
    owner.finished.insert(
        p.key.operation_id.as_str().into(),
        (p.expected, p.payload.clone()),
    );
}
fn adopt(s: &mut State) -> Result<()> {
    let owner = s.codecs.as_mut().ok_or(ClientError::Protocol)?;
    if owner
        .pending
        .as_ref()
        .is_none_or(|p| !p.written || p.receipt.is_none())
    {
        return Ok(());
    }
    if s.ended {
        return Err(ClientError::Protocol);
    }
    let mut p = owner.pending.take().ok_or(ClientError::Protocol)?;
    remember(owner, &p);
    let c = p.candidate.take().ok_or(ClientError::Protocol)?;
    // A signed notice on the same exact retained allocation remains eligible
    // only if it was active before this command or is strictly newer history.
    s.notices.retain(|_, n| n.matches(&c.selected));
    for (key, n) in p.notices {
        if s.notices
            .get(&key)
            .is_none_or(|old| old.history.revision < n.history.revision)
        {
            s.notices.insert(key, n);
        }
    }
    s.active = s
        .notices
        .iter()
        .filter(|(key, n)| {
            s.controls.admits(n)
                && (p.before.contains_key(key)
                    || p.highwater.get(key).is_none_or(|r| n.history.revision > *r))
        })
        .map(|(k, n)| (*k, n.clone()))
        .collect();
    owner.handles = c.wire_handles;
    owner.generations = c.generations;
    s.selected = Some(c.selected);
    s.suspended = false;
    s.version = s.version.wrapping_add(1);
    Ok(())
}
