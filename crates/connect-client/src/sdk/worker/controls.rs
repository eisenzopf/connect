//! Apply only newer, exact-binding control observations on this owned peer.
use super::*;
use connect_protocol::native_gates::GateStream;
#[derive(Clone)]
pub(super) struct LocalGate {
    active: bool,
    muted: bool,
    held: Option<Direction>,
    revision: StreamRevision,
}
impl LocalGate {
    pub fn direction(&self, base: Direction) -> Direction {
        let sends = matches!(base, Direction::Sendonly | Direction::Sendrecv)
            && self.active
            && !self.muted
            && self
                .held
                .is_none_or(|d| matches!(d, Direction::Sendonly | Direction::Sendrecv));
        let receives = matches!(base, Direction::Recvonly | Direction::Sendrecv)
            && self.active
            && self
                .held
                .is_none_or(|d| matches!(d, Direction::Recvonly | Direction::Sendrecv));
        match (sends, receives) {
            (true, true) => Direction::Sendrecv,
            (true, false) => Direction::Sendonly,
            (false, true) => Direction::Recvonly,
            _ => Direction::Inactive,
        }
    }
}
pub(super) fn observe(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    key: &OperationKey,
    intent: &Value,
    expected: u64,
    frame: &crate::canonical::Envelope,
) -> Result<()> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let crate::canonical::Payload::OperationUpdated(update) = frame.payload() else {
        return Err(fail());
    };
    if update.operation_id.as_str() != key.operation_id.as_str()
        || update.revision > 9_007_199_254_740_991
    {
        return Err(fail());
    }
    let state = serde_json::to_value(update.state).map_err(|_| fail())?;
    if state != serde_json::json!("completed") {
        if state == "failed" || state == "cancelled" {
            codecs::failed(cache, key)?;
        }
        return Ok(());
    }
    let observed = connection_updates::parse(update.result.as_ref().ok_or_else(fail)?, key)?;
    connection_updates::validate_intent(&observed, intent, expected, key)?;
    record(cache, media, key, &observed)
}
pub(super) fn record(
    cache: &std::sync::Mutex<NativeCache>,
    media: &media::Inbox,
    key: &OperationKey,
    observed: &connection_updates::Observed,
) -> Result<()> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    if observed.codecs.is_some() {
        return codecs::record(cache, media, key, observed);
    }
    if observed.quality.is_some() {
        return Ok(());
    }
    let mut cache = cache.lock().map_err(|_| fail())?;
    let conn = key.scope.connection_id.as_ref().ok_or_else(fail)?;
    let Some(entry) = cache.get_mut(conn) else {
        return Ok(());
    };
    if Some(&entry.session) != key.scope.session_id.as_ref()
        || Some(&entry.conversation) != key.scope.conversation_id.as_ref()
    {
        return Err(fail());
    }
    if entry.ended {
        return Ok(());
    }
    let old_matches = observed.gates.iter().all(|g| matches_binding(entry, g));
    let mut changes = Vec::new();
    for gate in &observed.gates {
        let id: StreamId = gate.binding.stream_id.parse().map_err(|_| fail())?;
        let held = gate
            .held_direction
            .as_ref()
            .map(|d| {
                serde_json::from_value::<Direction>(serde_json::json!(d))
                    .map(Direction::reverse)
                    .map_err(|_| fail())
            })
            .transpose()?;
        changes.push((
            id,
            LocalGate {
                active: gate.state == "active",
                muted: gate.source_muted,
                held,
                revision: observed.revision,
            },
        ));
    }
    let mut new_matches = false;
    if let Some(prepared) = entry.prepared.as_mut() {
        new_matches = observed
            .gates
            .iter()
            .all(|g| prepared.matches_gate(&g.binding));
        if new_matches || (old_matches && prepared.inherits(observed.revision)) {
            for (id, gate) in &changes {
                if prepared
                    .gates
                    .get(id)
                    .is_none_or(|old| old.revision.get() < gate.revision.get())
                {
                    prepared.gates.insert(id.clone(), gate.clone());
                }
            }
            prepared.observed_revision = StreamRevision::try_from(
                prepared
                    .observed_revision
                    .get()
                    .max(observed.revision.get()),
            )
            .map_err(|_| fail())?;
        }
    }
    if old_matches || new_matches {
        entry.known_revision =
            StreamRevision::try_from(entry.known_revision.get().max(observed.revision.get()))
                .map_err(|_| fail())?;
    }
    if !old_matches {
        return Ok(());
    }
    let mut directions = Vec::new();
    let mut gates = Vec::new();
    for (id, gate) in changes {
        if entry
            .gates
            .get(&id)
            .is_some_and(|old| old.revision.get() >= gate.revision.get())
        {
            continue;
        }
        let current = entry
            .selected
            .streams
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(fail)?;
        if current.stream_local_id.is_some() {
            directions.push((
                streams::StreamKey {
                    conversation_id: entry.conversation.clone(),
                    session_id: entry.session.clone(),
                    connection_id: conn.clone(),
                    stream_id: id.clone(),
                },
                gate.direction(current.direction),
            ));
        }
        gates.push((id, gate));
    }
    media.apply_directions(&directions)?;
    entry.gates.extend(gates);
    if entry.prepared.is_none() {
        entry.selected.stream_revision = entry.known_revision;
    }
    Ok(())
}
fn matches_binding(entry: &NativeConnection, gate: &GateStream) -> bool {
    let b = &gate.binding;
    entry.selected.streams.iter().any(|s| {
        s.id.as_str() == b.stream_id
            && serde_json::to_value(s.medium).ok().as_ref() == Some(&serde_json::json!(b.medium))
            && serde_json::to_value(s.direction.reverse()).ok().as_ref()
                == Some(&serde_json::json!(b.direction))
            && entry.wire_handles.get(&s.id) == Some(&b.stream_local_id)
            && s.codec == b.codec
            && entry.generations.get(&s.id) == Some(&b.stream_generation)
            && b.connection_generation == 1
    })
}
