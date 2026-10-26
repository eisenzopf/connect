//! Signed incoming invitation hints; refresh and decide under current authority.
//!
//! ```no_run
//! # async fn receive(client: connect_client::sdk::Client, saved_id: connect_client::sdk::OperationId) -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::{MutationOptions, RequestOptions};
//! use std::time::Duration;
//! let wait = RequestOptions::new(Duration::from_secs(10))?;
//! let mut incoming = client.incoming_invitations(wait.clone());
//! if let Some(hint) = incoming.next().await? {
//!     let current = hint.session.refresh(wait.clone()).await?;
//!     // The application durably saved this separate decision ID before IO.
//!     let decision = hint.session.reject(486, "Unavailable".into(),
//!         current.topology_revision, MutationOptions {
//!             operation_id: saved_id, request: wait.clone(), extensions: None,
//!         }).await?;
//!     decision.wait(wait).await?;
//! }
//! incoming.close().await?;
//! # Ok(()) }
//! ```
use super::*;
use crate::canonical::{Envelope, Scope};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use tokio::sync::Notify;

/// Original server invitation metadata. It grants no Session or native custody.
pub struct IncomingInvitation {
    pub session: Session,
    pub from_participant: ParticipantId,
    pub to_participant: ParticipantId,
    pub operation_id: OperationId,
    pub source: EventSource,
    pub medium: SessionMedium,
    pub intent: String,
    pub capabilities_offer: Value,
}
impl std::fmt::Debug for IncomingInvitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingInvitation")
            .field("conversation_id", self.session.conversation_id())
            .field("session_id", self.session.id())
            .field("operation_id", &self.operation_id)
            .finish_non_exhaustive()
    }
}
type Key = (ConversationId, SessionId, OperationId, ParticipantId);
struct Hint {
    key: Key,
    from: ParticipantId,
    medium: SessionMedium,
    intent: String,
    capabilities: Value,
    bytes: usize,
}
struct Seen {
    digest: [u8; 32],
    wires: VecDeque<String>,
}
#[derive(Default)]
struct State {
    queue: VecDeque<Hint>,
    bytes: usize,
    seen: BTreeMap<Key, Seen>,
    order: VecDeque<Key>,
    reader: bool,
    failure: Option<Error>,
}
#[derive(Default)]
pub(in crate::sdk) struct Inbox {
    state: std::sync::Mutex<State>,
    changed: Notify,
}
fn failure(kind: ErrorKind) -> Error {
    Error::request(kind, None, false)
}
fn bad() -> Error {
    failure(ErrorKind::Protocol)
}
fn parse(frame: &Envelope) -> Result<(Hint, [u8; 32])> {
    let scope = |kind| frame.scope(kind).ok_or_else(bad);
    let body = &frame.original()["payload"];
    if frame.expected_revision().is_some()
        || frame.scope(Scope::Connection).is_some()
        || frame.scope(Scope::Reply).is_some()
        || frame.original().get("signature").is_none()
    {
        return Err(bad());
    }
    let from: ParticipantId = body["from"]
        .as_str()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let to = body["to"]
        .as_array()
        .filter(|v| v.len() == 1)
        .ok_or_else(bad)?;
    let to: ParticipantId = to[0].as_str().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    if from == to {
        return Err(bad());
    }
    let medium = match body["medium"].as_str() {
        Some("voice") => SessionMedium::Voice,
        Some("video") => SessionMedium::Video,
        Some("voice+video") => SessionMedium::VoiceVideo,
        Some("screen-share") => SessionMedium::ScreenShare,
        Some("text-chat") => SessionMedium::TextChat,
        Some("mixed") => SessionMedium::Mixed,
        _ => return Err(bad()),
    };
    let intent = body["intent"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .ok_or_else(bad)?
        .to_owned();
    let capabilities = body["capabilities_offer"].clone();
    connect_protocol::session_control::validate_capabilities(&capabilities).map_err(|_| bad())?;
    let encoded = serde_json::to_vec(body).map_err(|_| bad())?;
    Ok((
        Hint {
            key: (
                scope(Scope::Conversation)?.parse().map_err(|_| bad())?,
                scope(Scope::Session)?.parse().map_err(|_| bad())?,
                scope(Scope::Operation)?.parse().map_err(|_| bad())?,
                to,
            ),
            from,
            medium,
            intent,
            capabilities,
            bytes: encoded.len(),
        },
        Sha256::digest(encoded).into(),
    ))
}
impl Inbox {
    fn fail(&self, error: Error) {
        if let Ok(mut state) = self.state.lock() {
            state.queue.clear();
            state.bytes = 0;
            if state.failure.is_none() {
                state.failure = Some(error);
            }
        }
        self.changed.notify_waiters();
    }
    pub(in crate::sdk) fn end(&self, kind: ErrorKind) {
        self.fail(failure(kind));
    }
    pub(in crate::sdk) fn frame(&self, frame: &Envelope) -> bool {
        if frame.kind() == "session.invite" && frame.scope(Scope::Reply).is_none() {
            let result =
                parse(frame).and_then(|(hint, digest)| self.accept(hint, digest, frame.id()));
            if let Err(error) = result {
                self.fail(error);
            }
            return true;
        }
        if frame.kind() != "session.cancel" {
            return false;
        }
        let Some(reply) = frame.scope(Scope::Reply) else {
            return false;
        };
        let result = (|| {
            let mut state = self.state.lock().map_err(|_| bad())?;
            let key = state
                .seen
                .iter()
                .find(|(_, s)| s.wires.iter().any(|w| w == reply))
                .map(|(k, _)| k.clone());
            let Some(key) = key else {
                return Ok(false);
            };
            if frame.scope(Scope::Conversation) != Some(key.0.as_str())
                || frame.scope(Scope::Session) != Some(key.1.as_str())
                || frame.scope(Scope::Operation) != Some(key.2.as_str())
                || frame.scope(Scope::Connection).is_some()
                || frame.expected_revision().is_some()
            {
                return Err(bad());
            }
            if let Some(i) = state.queue.iter().position(|h| h.key == key) {
                let hint = state.queue.remove(i).ok_or_else(bad)?;
                state.bytes = state.bytes.saturating_sub(hint.bytes);
            }
            Ok(true)
        })();
        match result {
            Ok(handled) => handled,
            Err(error) => {
                self.fail(error);
                true
            }
        }
    }
    fn accept(&self, hint: Hint, digest: [u8; 32], wire: &str) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| bad())?;
        if state.failure.is_some() {
            return Ok(());
        }
        if let Some(seen) = state.seen.get_mut(&hint.key) {
            if seen.digest != digest {
                return Err(bad());
            }
            if !seen.wires.iter().any(|w| w == wire) {
                if seen.wires.len() == 4 {
                    seen.wires.pop_front();
                }
                seen.wires.push_back(wire.into());
            }
            return Ok(());
        }
        if state.queue.len() >= 32 || state.bytes.saturating_add(hint.bytes) > 1024 * 1024 {
            return Err(failure(ErrorKind::ResyncRequired));
        }
        if state.order.len() == 256 {
            if let Some(key) = state.order.pop_front() {
                state.seen.remove(&key);
            }
        }
        state.seen.insert(
            hint.key.clone(),
            Seen {
                digest,
                wires: VecDeque::from([wire.into()]),
            },
        );
        state.order.push_back(hint.key.clone());
        state.bytes += hint.bytes;
        state.queue.push_back(hint);
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }
    fn claim(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| bad())?;
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if state.reader {
            return Err(failure(ErrorKind::Conflict));
        }
        state.reader = true;
        Ok(())
    }
    fn release(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.reader = false;
        }
    }
    async fn receive(&self, options: &RequestOptions) -> Result<Hint> {
        let until = tokio::time::Instant::now() + options.timeout;
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.state.lock().map_err(|_| bad())?;
                if options.signal.is_cancelled() {
                    return Err(failure(ErrorKind::Cancelled));
                }
                if let Some(error) = &state.failure {
                    return Err(error.clone());
                }
                if tokio::time::Instant::now() >= until {
                    return Err(failure(ErrorKind::Deadline));
                }
                if let Some(hint) = state.queue.pop_front() {
                    state.bytes = state.bytes.saturating_sub(hint.bytes);
                    return Ok(hint);
                }
            }
            tokio::select! {biased;_=options.signal.cancelled()=>return Err(failure(ErrorKind::Cancelled)),_=tokio::time::sleep_until(until)=>return Err(failure(ErrorKind::Deadline)),_=&mut changed=>()}
        }
    }
}
/// One lazy reader per Client. Dropping/timing out a wait consumes no hint.
pub struct InvitationStream {
    client: Option<Client>,
    inbox: Arc<Inbox>,
    options: RequestOptions,
    active: bool,
}
impl std::fmt::Debug for InvitationStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvitationStream").finish_non_exhaustive()
    }
}
impl Drop for InvitationStream {
    fn drop(&mut self) {
        if self.active {
            self.inbox.release();
        }
    }
}
impl Client {
    /// Signed metadata hints. Always refresh the Session before deciding; delayed
    /// invitations can be cancelled/revoked and never grant media authority.
    pub fn incoming_invitations(&self, options: RequestOptions) -> InvitationStream {
        InvitationStream {
            client: Some(self.clone()),
            inbox: self.shared.invitations.clone(),
            options,
            active: false,
        }
    }
}
impl InvitationStream {
    pub async fn next(&mut self) -> Result<Option<IncomingInvitation>> {
        let Some(client) = self.client.as_ref() else {
            return Ok(None);
        };
        self.options.validate()?;
        if self.options.signal.is_cancelled() {
            return Err(failure(ErrorKind::Cancelled));
        }
        if !self.active {
            self.inbox.claim()?;
            self.active = true;
        }
        let hint = self.inbox.receive(&self.options).await?;
        let (cid, sid, operation_id, to_participant) = hint.key;
        Ok(Some(IncomingInvitation {
            session: client.conversation_by_id(cid).session_by_id(sid),
            from_participant: hint.from,
            to_participant,
            source: EventSource {
                kind: "session.invite".into(),
                correlation_id: operation_id.as_str().into(),
            },
            operation_id,
            medium: hint.medium,
            intent: hint.intent,
            capabilities_offer: hint.capabilities,
        }))
    }
    /// Detach only this local reader. Pending hints remain bounded for a later
    /// reader and retain their original logical IDs; there is no network retry.
    pub async fn close(&mut self) -> Result<()> {
        if self.active {
            self.inbox.release();
            self.active = false;
        }
        self.client.take();
        Ok(())
    }
}
