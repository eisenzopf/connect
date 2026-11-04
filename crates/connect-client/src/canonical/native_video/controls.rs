//! Signed gate results join the original local intent and sealed allocation.
//! A historical receipt never supplies a media epoch or replaces resources.
use super::*;
use crate::sdk::{connection_updates, OperationKey, OperationScope};
use connect_protocol::native_gates::GateStream;
use std::collections::BTreeSet;

struct Pending {
    key: OperationKey,
    payload: Value,
    expected: u64,
    affected: BTreeSet<(u16, bool)>,
    terminal: bool,
}
#[derive(Default)]
pub(super) struct Controls {
    operations: BTreeMap<String, Pending>,
    gates: BTreeMap<String, (u64, GateStream)>,
    // Original history high water at local invalidation, never reset by a
    // receipt. A new notice may arrive before or after the command result.
    floors: BTreeMap<(u16, bool), u64>,
}
fn permits(g: &GateStream, publisher: bool) -> bool {
    g.state == "active"
        && (!publisher || !g.source_muted)
        && g.held_direction
            .as_deref()
            .is_none_or(|d| d == "sendrecv" || d == if publisher { "recvonly" } else { "sendonly" })
}
impl Controls {
    pub fn admits(&self, notice: &Notice) -> bool {
        let key = (notice.handle, notice.role == Role::Publisher);
        self.floors
            .get(&key)
            .is_none_or(|r| notice.history.revision > *r)
            && !self
                .operations
                .values()
                .any(|p| !p.terminal && p.affected.contains(&key))
            && self
                .gates
                .get(notice.key.stream_id.as_str())
                .is_none_or(|(_, g)| permits(g, key.1))
    }
}
pub(super) fn outgoing(s: &mut State, frame: &Value) -> Result<()> {
    let payload = &frame["payload"];
    connect_protocol::native_gates::validate(payload).map_err(|_| ClientError::Protocol)?;
    let operation = frame["operation_id"]
        .as_str()
        .ok_or(ClientError::Protocol)?;
    let expected = frame["expected_revision"]
        .as_u64()
        .filter(|r| (1..9_007_199_254_740_991).contains(r))
        .ok_or(ClientError::Protocol)?;
    if let Some(old) = s.controls.operations.get(operation) {
        return if old.payload == *payload && old.expected == expected {
            Ok(())
        } else {
            Err(ClientError::Protocol)
        };
    }
    let selected = s.selected.as_ref().ok_or(ClientError::Protocol)?;
    if s.ended || s.suspended || expected < selected.stream_revision.get() {
        return Err(ClientError::Protocol);
    }
    let targets: BTreeSet<String> = match payload["streams"].as_array() {
        Some(ids) => ids
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or(ClientError::Protocol))
            .collect::<Result<_>>()?,
        None => selected
            .streams
            .iter()
            .filter(|v| v.medium != crate::sdk::Medium::Data)
            .map(|v| v.id.as_str().to_owned())
            .collect(),
    };
    if targets.is_empty()
        || targets.iter().any(|id| {
            !selected
                .streams
                .iter()
                .any(|v| v.id.as_str() == id && v.medium != crate::sdk::Medium::Data)
        })
    {
        return Err(ClientError::Protocol);
    }
    // Bound both unresolved work and replay memory. Evicted historical results
    // have no local intent and cannot install anything; old revisions cannot
    // start a new local mutation against a newer adopted binding.
    if s.controls.operations.len() >= 64 {
        let retired = s
            .controls
            .operations
            .iter()
            .find(|(_, p)| p.terminal)
            .map(|(id, _)| id.clone())
            .ok_or(ClientError::Protocol)?;
        s.controls.operations.remove(&retired);
    }
    let key = OperationKey {
        operation_id: operation.parse().map_err(|_| ClientError::Protocol)?,
        command: "connection.update".into(),
        scope: OperationScope {
            conversation_id: s
                .scope
                .conversation_id
                .as_ref()
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| ClientError::Protocol)?,
            session_id: s
                .scope
                .session_id
                .as_ref()
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| ClientError::Protocol)?,
            connection_id: s
                .scope
                .connection_id
                .as_ref()
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| ClientError::Protocol)?,
        },
    };
    let mut affected = BTreeSet::new();
    for stream in &selected.streams {
        if !targets.contains(stream.id.as_str())
            || !selected.video_feedback.contains_key(&stream.id)
        {
            continue;
        }
        let handle = stream.stream_local_id.ok_or(ClientError::Protocol)?;
        for publisher in [false, true] {
            let invalidates = match payload["action"].as_str() {
                Some("mute" | "unmute") => publisher,
                Some("hold") => match payload["details"]["direction"].as_str() {
                    Some("inactive") => true,
                    Some("sendonly") => !publisher,
                    _ => publisher,
                },
                Some("resume") => {
                    s.controls
                        .gates
                        .get(stream.id.as_str())
                        .is_some_and(|(_, g)| !permits(g, publisher))
                        || s.controls
                            .operations
                            .values()
                            .any(|p| !p.terminal && p.affected.contains(&(handle, publisher)))
                }
                _ => false,
            };
            if invalidates {
                let key = (handle, publisher);
                affected.insert(key);
                let floor = s.notices.get(&key).map_or(0, |n| n.history.revision);
                s.controls
                    .floors
                    .entry(key)
                    .and_modify(|v| *v = (*v).max(floor))
                    .or_insert(floor);
                s.active.remove(&key);
            }
        }
    }
    s.controls.operations.insert(
        operation.into(),
        Pending {
            key,
            payload: payload.clone(),
            expected,
            affected,
            terminal: false,
        },
    );
    Ok(())
}
fn matches(s: &State, gate: &GateStream) -> bool {
    let b = &gate.binding;
    let Some(selected) = &s.selected else {
        return false;
    };
    let Some(stream) = selected
        .streams
        .iter()
        .find(|v| v.id.as_str() == b.stream_id)
    else {
        return false;
    };
    let original =
        &s.callback.as_ref().unwrap_or(&Value::Null)["payload"]["substrate_setup"]["streams"];
    let Some(wire) = original.as_array().and_then(|v| {
        v.iter()
            .find(|v| v["strm_id"].as_str() == Some(b.stream_id.as_str()))
    }) else {
        return false;
    };
    // Initial sealed resources have stream generation one. Profiled video
    // carries its generation explicitly; all bindings share the connection.
    let generation = selected
        .video_feedback
        .values()
        .next()
        .map(|v| v.connection_generation);
    generation == Some(b.connection_generation)
        && b.stream_generation
            == selected
                .video_feedback
                .get(&stream.id)
                .map_or(1, |v| v.stream_generation)
        && wire["kind"] == b.medium
        && wire["direction"] == b.direction
        && wire["stream_local_id"].as_u64() == Some(u64::from(b.stream_local_id))
        && stream.codec == b.codec
}
pub(super) fn observe(s: &mut State, frame: &Envelope) -> Result<()> {
    let Payload::OperationUpdated(update) = frame.payload() else {
        return Err(ClientError::Protocol);
    };
    let operation = update.operation_id.as_str();
    let Some(pending) = s.controls.operations.get(operation) else {
        return Ok(());
    };
    if frame
        .scope(Scope::Operation)
        .is_some_and(|v| v != operation)
        || update
            .command_type
            .as_deref()
            .is_some_and(|v| v != "connection.update")
        || update
            .expected_revision
            .is_some_and(|v| v != pending.expected)
    {
        return Err(ClientError::Protocol);
    }
    if pending.terminal {
        return Ok(());
    }
    let state = serde_json::to_value(update.state).map_err(|_| ClientError::Protocol)?;
    if state != "completed" {
        if state == "failed" || state == "cancelled" {
            // No rollback of history: uncertainty/failure cannot restore a
            // consumed hint or a notice retired by the original local command.
            s.controls
                .operations
                .get_mut(operation)
                .ok_or(ClientError::Protocol)?
                .terminal = true;
        }
        return Ok(());
    }
    let observed = protocol(connection_updates::parse(
        update.result.as_ref().ok_or(ClientError::Protocol)?,
        &pending.key,
    ))?;
    protocol(connection_updates::validate_intent(
        &observed,
        &pending.payload,
        pending.expected,
        &pending.key,
    ))?;
    if observed.gates.iter().any(|g| !matches(s, g)) {
        return Err(ClientError::Protocol);
    }
    // Omitted target lists mean every negotiated media stream, not any subset
    // the result happens to contain.
    if pending.payload.get("streams").is_none() {
        let expected: BTreeSet<_> = s
            .selected
            .as_ref()
            .ok_or(ClientError::Protocol)?
            .streams
            .iter()
            .filter(|v| v.medium != crate::sdk::Medium::Data)
            .map(|v| v.id.as_str())
            .collect();
        let actual = observed
            .gates
            .iter()
            .map(|g| g.binding.stream_id.as_str())
            .collect();
        if expected != actual {
            return Err(ClientError::Protocol);
        }
    }
    let affected = pending.affected.clone();
    let revision = observed.revision.get();
    for gate in observed.gates {
        if s.controls
            .gates
            .get(&gate.binding.stream_id)
            .is_none_or(|(old, _)| *old < revision)
        {
            s.controls
                .gates
                .insert(gate.binding.stream_id.clone(), (revision, gate));
        }
    }
    let selected = s.selected.as_mut().ok_or(ClientError::Protocol)?;
    if selected.stream_revision.get() < revision {
        selected.stream_revision = observed.revision;
    }
    s.controls
        .operations
        .get_mut(operation)
        .ok_or(ClientError::Protocol)?
        .terminal = true;
    if !s.suspended {
        s.active.retain(|_, n| s.controls.admits(n));
        for key in affected {
            if let Some(n) = s.notices.get(&key).filter(|n| s.controls.admits(n)) {
                s.active.insert(key, n.clone());
            }
        }
    }
    Ok(())
}
