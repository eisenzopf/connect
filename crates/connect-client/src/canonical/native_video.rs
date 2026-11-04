//! Private provenance for a manual replacement media owner. Only the existing
//! signature-verifying readers may feed this state; callers cannot install a
//! notice, selected allocation, callback or activation epoch from JSON.
use super::*;
use crate::sdk::{
    native_negotiation::Pending as Negotiating,
    video_feedback::epochs::{Notice, Role},
    NativeNegotiation,
};
use std::{collections::BTreeMap, sync::Mutex};
use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct Shared {
    state: Mutex<Option<State>>,
    pub changed: Notify,
}
struct State {
    request: String,
    scope: CommandScope,
    negotiating: Negotiating,
    // Kept as the complete verified frame: an equal id with altered fields is
    // never interchangeable with the actual callback on this physical peer.
    callback: Option<Value>,
    answer: Option<Value>,
    selected: Option<NativeNegotiation>,
    notices: BTreeMap<(u16, bool), Notice>,
    active: BTreeMap<(u16, bool), Notice>,
    suspended: bool,
    ended: bool,
    version: u64,
}
fn protocol<T>(value: crate::sdk::Result<T>) -> Result<T> {
    value.map_err(|_| ClientError::Protocol)
}
impl Shared {
    /// Registered before the original replacement media command is submitted. This
    /// can happen once on a physical peer and only with an explicit profile.
    pub fn begin(&self, request: String, scope: CommandScope, payload: Value) -> Result<()> {
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        if payload["capabilities"]["video_feedback"] != "rtcp-pli-epoch/1" {
            return if guard.is_none() {
                Ok(())
            } else {
                Err(ClientError::Protocol)
            };
        }
        if guard.is_some() || self_scope(&scope).iter().any(|v| v.is_none()) {
            return Err(ClientError::Protocol);
        }
        *guard = Some(State {
            request,
            negotiating: Negotiating::replacement(payload, scope.clone()),
            scope,
            callback: None,
            answer: None,
            selected: None,
            notices: BTreeMap::new(),
            active: BTreeMap::new(),
            suspended: false,
            ended: false,
            version: 0,
        });
        Ok(())
    }
    /// Invoked immediately after signature verification, before generic inbox
    /// delivery. Consumed epoch notices never crowd out command replies/Data.
    pub fn verified(&self, frame: &Envelope) -> Result<bool> {
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_mut() else {
            return Ok(false);
        };
        if frame.kind() == "stream.epoch" {
            let n = protocol(Notice::read(frame))?;
            let root = &n.key;
            if s.ended
                || s.scope.conversation_id.as_deref() != Some(root.conversation_id.as_str())
                || s.scope.session_id.as_deref() != Some(root.session_id.as_str())
                || s.scope.connection_id.as_deref() != Some(root.connection_id.as_str())
            {
                return Ok(true);
            }
            let selected = match &s.selected {
                Some(v) => v,
                None => protocol(s.negotiating.selected())?,
            };
            if !n.matches(selected) {
                return Ok(true);
            }
            let key = (n.handle, n.role == Role::Publisher);
            if s.notices
                .get(&key)
                .is_some_and(|old| old.history.revision >= n.history.revision)
            {
                return Ok(true);
            }
            if s.notices.len() >= 64 && !s.notices.contains_key(&key) {
                return Err(ClientError::Protocol);
            }
            s.notices.insert(key, n.clone());
            if s.selected.is_some() && !s.suspended {
                s.active.insert(key, n);
            }
            drop(guard);
            self.changed.notify_waiters();
            return Ok(true);
        }
        let own_root = frame.scope(Scope::Conversation) == s.scope.conversation_id.as_deref()
            && frame.scope(Scope::Session) == s.scope.session_id.as_deref();
        if own_root
            && (frame.kind() == "session.ended"
                || (frame.kind() == "connection.end"
                    && frame.scope(Scope::Connection) == s.scope.connection_id.as_deref()))
        {
            s.ended = true;
            s.active.clear();
            s.version = s.version.wrapping_add(1);
            drop(guard);
            self.changed.notify_waiters();
            return Ok(false);
        }
        if s.ended
            || !own_root
            || frame.scope(Scope::Connection) != s.scope.connection_id.as_deref()
            || frame.scope(Scope::Operation) != s.scope.operation_id.as_deref()
        {
            return Ok(false);
        }
        if frame.kind() == "connection.answer" && s.selected.is_none() {
            protocol(s.negotiating.answer(frame, &s.request))?;
        } else if frame.kind() == "connection.offer" && s.selected.is_none() {
            if s.callback.is_some() {
                return Err(ClientError::Protocol);
            }
            let answer = protocol(s.negotiating.confirmation(frame))?;
            s.answer = Some(serde_json::to_value(answer).map_err(|_| ClientError::Protocol)?);
            s.callback = Some(frame.original().clone());
        }
        Ok(false)
    }
    /// Before signing the original parameter answer, require the exact callback
    /// observed on this physical reader and the independently validated answer.
    pub fn check_confirmation(
        &self,
        frame: &Envelope,
        answer: &payloads::ConnectionAnswer,
        root: &CommandScope,
    ) -> Result<bool> {
        let guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_ref() else {
            return Ok(false);
        };
        if root.connection_id != s.scope.connection_id || root.operation_id != s.scope.operation_id
        {
            return Ok(false);
        }
        if s.ended
            || s.selected.is_some()
            || self_scope(root) != self_scope(&s.scope)
            || s.callback.as_ref() != Some(frame.original())
            || s.answer.as_ref()
                != Some(&serde_json::to_value(answer).map_err(|_| ClientError::Protocol)?)
        {
            return Err(ClientError::Protocol);
        }
        Ok(true)
    }
    /// A successful signed write seals resources, never media readiness. The
    /// server still gates every packet and the original replacement commit.
    pub fn confirmed(&self) -> Result<()> {
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let s = guard.as_mut().ok_or(ClientError::Protocol)?;
        if s.ended || s.selected.is_some() || s.callback.is_none() {
            return Err(ClientError::Protocol);
        }
        s.selected = Some(protocol(s.negotiating.selected())?.clone());
        if !s.suspended {
            s.active = s.notices.clone();
        }
        drop(guard);
        self.changed.notify_waiters();
        Ok(())
    }
    // A new local control invalidates already captured hints before any await.
    // Mutations are not retried here. Resume cannot resurrect a retained epoch;
    // only a strictly newer signed notice may establish subsequent history.
    pub fn outgoing(&self, frame: &Value) -> Result<()> {
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_mut() else {
            return Ok(());
        };
        if frame["cid"].as_str() != s.scope.conversation_id.as_deref()
            || frame["sid"].as_str() != s.scope.session_id.as_deref()
            || frame["connid"].as_str() != s.scope.connection_id.as_deref()
        {
            return Ok(());
        }
        match frame["type"].as_str() {
            Some("connection.end") => {
                s.ended = true;
                s.active.clear();
                s.version = s.version.wrapping_add(1);
            }
            Some("connection.update") => {
                s.version = s.version.wrapping_add(1);
                if matches!(
                    frame["payload"]["action"].as_str(),
                    Some("hold" | "mute" | "resume" | "unmute" | "codec-renegotiate")
                ) {
                    // Until signed control/resource adoption exists for this raw
                    // owner, a gate mutation suspends it conservatively.
                    s.suspended = true;
                    s.active.clear();
                }
            }
            Some("stream.unsubscribe") => {
                s.version = s.version.wrapping_add(1);
                let ids = frame["payload"]["strm_ids"]
                    .as_array()
                    .ok_or(ClientError::Protocol)?;
                s.active.retain(|(_, publisher), n| {
                    *publisher
                        || !ids
                            .iter()
                            .any(|id| id.as_str() == Some(n.key.stream_id.as_str()))
                });
            }
            Some("stream.subscribe") => {
                s.version = s.version.wrapping_add(1);
            }
            _ => (),
        }
        Ok(())
    }
}
fn self_scope(scope: &CommandScope) -> [Option<&str>; 4] {
    [
        scope.conversation_id.as_deref(),
        scope.session_id.as_deref(),
        scope.connection_id.as_deref(),
        scope.operation_id.as_deref(),
    ]
}

#[cfg(test)]
pub(crate) mod tests;

pub(super) mod io;
pub use io::{NativeVideoDatagram, NativeVideoDatagrams};
