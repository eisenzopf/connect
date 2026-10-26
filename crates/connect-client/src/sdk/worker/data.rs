//! Bounded per-binding reliable queues never hold up the single control reader.
use super::super::data as api;
use super::*;
use crate::canonical::{Envelope, Scope};
use std::collections::{BTreeMap, VecDeque};
use tokio::sync::Notify;
const PER_STREAM: usize = 32;
const TOTAL_MESSAGES: usize = 256;
const TOTAL_BYTES: usize = 1024 * 1024;
#[derive(Clone, PartialEq)]
struct Binding {
    codec: Value,
    generation: u64,
}
fn binding(cache: &NativeCache, key: &streams::StreamKey, send: bool) -> Result<Binding> {
    let bad = || Error::local(ErrorKind::Unsupported);
    let entry = cache
        .get(&key.connection_id)
        .filter(|e| {
            !e.ended && e.session == key.session_id && e.conversation == key.conversation_id
        })
        .ok_or_else(bad)?;
    let stream = entry
        .selected
        .streams
        .iter()
        .find(|s| s.id == key.stream_id && s.medium == Medium::Data)
        .ok_or_else(bad)?;
    let direction = entry
        .gates
        .get(&key.stream_id)
        .map_or(stream.direction, |g| g.direction(stream.direction));
    if if send {
        !matches!(direction, Direction::Sendonly | Direction::Sendrecv)
    } else {
        !matches!(direction, Direction::Recvonly | Direction::Sendrecv)
    } {
        return Err(bad());
    }
    if !matches!(
        stream.codec["name"].as_str(),
        Some("json" | "text" | "binary")
    ) {
        return Err(bad());
    }
    Ok(Binding {
        codec: stream.codec.clone(),
        generation: *entry.generations.get(&key.stream_id).ok_or_else(bad)?,
    })
}
pub(super) fn send_key(
    kind: &str,
    scope: &CommandScope,
    payload: &Value,
) -> Result<Option<streams::StreamKey>> {
    if kind != "message.send" || scope.session_id.is_none() && scope.connection_id.is_none() {
        return Ok(None);
    }
    let bad = || Error::local(ErrorKind::Validation);
    if scope.expected_revision.is_some() || scope.operation_id.is_none() {
        return Err(bad());
    }
    Ok(Some(streams::StreamKey {
        conversation_id: scope
            .conversation_id
            .as_deref()
            .ok_or_else(bad)?
            .parse()
            .map_err(|_| bad())?,
        session_id: scope
            .session_id
            .as_deref()
            .ok_or_else(bad)?
            .parse()
            .map_err(|_| bad())?,
        connection_id: scope
            .connection_id
            .as_deref()
            .ok_or_else(bad)?
            .parse()
            .map_err(|_| bad())?,
        stream_id: payload["label"]
            .as_str()
            .ok_or_else(bad)?
            .parse()
            .map_err(|_| bad())?,
    }))
}
pub(super) fn check_send(
    cache: &std::sync::Mutex<NativeCache>,
    key: &streams::StreamKey,
    payload: &Value,
) -> Result<()> {
    let cache = cache
        .lock()
        .map_err(|_| Error::local(ErrorKind::Protocol))?;
    let binding = binding(&cache, key, true)?;
    if payload.get("from").is_some()
        || payload.get("in_reply_to_msg").is_some()
        || payload.get("attachments").is_some()
        || payload["reliability"] != serde_json::json!({"mode":"reliable_ordered"})
    {
        return Err(Error::local(ErrorKind::Validation));
    }
    api::body(
        payload,
        binding.codec["name"]
            .as_str()
            .ok_or_else(|| Error::local(ErrorKind::Protocol))?,
    )?;
    Ok(())
}
struct Queue {
    binding: Binding,
    items: VecDeque<(ReceivedData, usize)>,
    failed: bool,
}
#[derive(Default)]
struct State {
    queues: BTreeMap<streams::StreamKey, Queue>,
    messages: usize,
    bytes: usize,
    ended: bool,
    stats: DataStats,
}
#[derive(Default)]
pub(super) struct Inbox {
    state: std::sync::Mutex<State>,
    changed: Notify,
}
fn prune(state: &mut State, cache: &NativeCache) {
    state.queues.retain(|key, queue| {
        if binding(cache, key, false).is_ok_and(|b| b == queue.binding) {
            return true;
        }
        state.messages = state.messages.saturating_sub(queue.items.len());
        state.bytes = state
            .bytes
            .saturating_sub(queue.items.iter().map(|(_, n)| *n).sum::<usize>());
        state.stats.binding_change_drops = state
            .stats
            .binding_change_drops
            .saturating_add(queue.items.len() as u64);
        false
    });
}
impl Inbox {
    pub(super) fn frame(
        &self,
        cache: &std::sync::Mutex<NativeCache>,
        frame: &Envelope,
    ) -> Result<bool> {
        if frame.kind() != "message.send" || frame.scope(Scope::Connection).is_none() {
            return Ok(false);
        }
        let cache = cache
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        state.stats.received_frames = state.stats.received_frames.saturating_add(1);
        if state.ended {
            return Ok(true);
        }
        prune(&mut state, &cache);
        let decode = || -> Result<(streams::StreamKey, Binding, ReceivedData, usize)> {
            let bad = || Error::local(ErrorKind::Validation);
            if frame.scope(Scope::Reply).is_some() || frame.expected_revision().is_some() {
                return Err(bad());
            }
            let key = streams::StreamKey {
                conversation_id: frame.scope(Scope::Conversation).ok_or_else(bad)?.parse()?,
                session_id: frame.scope(Scope::Session).ok_or_else(bad)?.parse()?,
                connection_id: frame.scope(Scope::Connection).ok_or_else(bad)?.parse()?,
                stream_id: frame.original()["payload"]["label"]
                    .as_str()
                    .ok_or_else(bad)?
                    .parse()?,
            };
            let binding = binding(&cache, &key, false)?;
            let p = &frame.original()["payload"];
            let local = &cache.get(&key.connection_id).ok_or_else(bad)?.participant;
            if p.get("attachments")
                .is_some_and(|a| a.as_array().is_none_or(|a| !a.is_empty()))
                || p.get("in_reply_to_msg").is_some()
                || p.get("reliability")
                    .is_some_and(|r| *r != serde_json::json!({"mode":"reliable_ordered"}))
            {
                return Err(bad());
            }
            if p["to"] != "all"
                && !p["to"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(local.as_str())))
            {
                return Err(bad());
            }
            let (payload, size) = api::body(p, binding.codec["name"].as_str().ok_or_else(bad)?)?;
            let data = ReceivedData {
                message_id: p["msg_id"].as_str().ok_or_else(bad)?.parse()?,
                operation_id: frame.scope(Scope::Operation).ok_or_else(bad)?.parse()?,
                from: p["from"].as_str().ok_or_else(bad)?.parse()?,
                payload,
            };
            Ok((key, binding, data, size))
        };
        let (key, binding, data, size) = match decode() {
            Ok(value) => value,
            Err(error) => {
                if error.kind == ErrorKind::Unsupported {
                    state.stats.unowned_frames = state.stats.unowned_frames.saturating_add(1);
                } else {
                    state.stats.malformed_frames = state.stats.malformed_frames.saturating_add(1);
                }
                return Ok(true);
            }
        };
        // There are at most64 owned Connections with32 negotiated Streams each.
        if state.queues.len() >= 2048 && !state.queues.contains_key(&key) {
            return Err(Error::local(ErrorKind::Protocol));
        }
        let full =
            state.messages >= TOTAL_MESSAGES || state.bytes.saturating_add(size) > TOTAL_BYTES;
        let queue = state.queues.entry(key).or_insert_with(|| Queue {
            binding,
            items: VecDeque::new(),
            failed: false,
        });
        if queue.failed || queue.items.len() >= PER_STREAM || full {
            queue.failed = true;
            let count = queue.items.len();
            let bytes = queue.items.iter().map(|(_, n)| *n).sum::<usize>();
            queue.items.clear();
            state.messages = state.messages.saturating_sub(count);
            state.bytes = state.bytes.saturating_sub(bytes);
            state.stats.overflow_drops =
                state.stats.overflow_drops.saturating_add(count as u64 + 1);
        } else {
            queue.items.push_back((data, size));
            state.messages += 1;
            state.bytes += size;
            state.stats.accepted_messages = state.stats.accepted_messages.saturating_add(1);
        }
        drop(state);
        self.changed.notify_waiters();
        Ok(true)
    }
    pub(super) fn end(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.ended = true;
        state.queues.clear();
        state.messages = 0;
        state.bytes = 0;
        drop(state);
        self.changed.notify_waiters();
    }
    async fn receive(
        &self,
        cache: &std::sync::Mutex<NativeCache>,
        key: &streams::StreamKey,
        options: RequestOptions,
    ) -> Result<ReceivedData> {
        options.validate()?;
        let until = tokio::time::Instant::now() + options.timeout;
        loop {
            if tokio::time::Instant::now() >= until {
                return Err(Error::request(ErrorKind::Deadline, None, false));
            }
            if options.signal.is_cancelled() {
                return Err(Error::request(ErrorKind::Cancelled, None, false));
            }
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let cache = cache
                    .lock()
                    .map_err(|_| Error::local(ErrorKind::Protocol))?;
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| Error::local(ErrorKind::Protocol))?;
                if state.ended {
                    return Err(Error::request(ErrorKind::Cancelled, None, false));
                }
                prune(&mut state, &cache);
                let binding =
                    binding(&cache, key, false).map_err(|e| Error::request(e.kind, None, false))?;
                let queue = state.queues.entry(key.clone()).or_insert_with(|| Queue {
                    binding,
                    items: VecDeque::new(),
                    failed: false,
                });
                if queue.failed {
                    return Err(Error::request(ErrorKind::ResyncRequired, None, false));
                }
                if let Some((data, size)) = queue.items.pop_front() {
                    state.messages = state.messages.saturating_sub(1);
                    state.bytes = state.bytes.saturating_sub(size);
                    return Ok(data);
                }
            }
            tokio::select! {biased;
                _=options.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,None,false)),
                _=tokio::time::sleep_until(until)=>return Err(Error::request(ErrorKind::Deadline,None,false)),
                _=&mut notified=>(),_=tokio::time::sleep(Duration::from_millis(100))=>(),
            }
        }
    }
}
impl Shared {
    pub(in crate::sdk) async fn receive_data(
        &self,
        key: &streams::StreamKey,
        options: RequestOptions,
    ) -> Result<ReceivedData> {
        self.data.receive(&self.native, key, options).await
    }
    pub(in crate::sdk) fn data_stats(&self) -> Result<DataStats> {
        let cache = self
            .native
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        let mut state = self
            .data
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        prune(&mut state, &cache);
        Ok(DataStats {
            queued_messages: state.messages,
            queued_bytes: state.bytes,
            ..state.stats
        })
    }
}
