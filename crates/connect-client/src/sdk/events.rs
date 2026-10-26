//! Durable journal facts over an owned, bounded live subscription.
//!
//! ```no_run
//! # async fn journal(room: connect_client::sdk::Conversation) -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::RequestOptions;
//! use std::time::Duration;
//! let mut events = room.events(None, RequestOptions::new(Duration::from_secs(10))?);
//! let initial = events.snapshot().await?; // Complete authorized metadata boundary.
//! if let Some(event) = events.next().await? {
//!     // Apply by event_id, then durably save event.cursor after processing.
//!     let _ = (initial, event.event_id, event.cursor);
//! }
//! events.close().await?;
//! # Ok(()) }
//! ```
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use tokio::sync::Notify;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Cursor(String);
impl Cursor {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Cursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cursor(redacted)")
    }
}
impl TryFrom<String> for Cursor {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty() || value.len() > 2048 || !value.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(Error::local(ErrorKind::Validation));
        }
        Ok(Self(value))
    }
}
impl std::str::FromStr for Cursor {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        Self::try_from(value.to_owned())
    }
}
impl From<Cursor> for String {
    fn from(value: Cursor) -> Self {
        value.0
    }
}
#[derive(Clone, Debug)]
pub struct EventSource {
    pub kind: String,
    pub correlation_id: String,
}
#[derive(Clone, Debug)]
pub struct AudienceMember {
    pub participant_id: ParticipantId,
    pub membership_epoch: u64,
}
#[derive(Clone)]
pub struct JournalEvent {
    pub event_id: String,
    pub cursor: Cursor,
    pub conversation_id: ConversationId,
    pub session_id: Option<SessionId>,
    pub connection_id: Option<ConnectionId>,
    pub operation_id: Option<OperationId>,
    pub participant_id: Option<ParticipantId>,
    pub audience: Vec<AudienceMember>,
    pub kind: String,
    pub revision: u64,
    pub source: EventSource,
    pub observed_at: i64,
    pub data: Value,
}
impl std::fmt::Debug for JournalEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalEvent")
            .field("event_id", &self.event_id)
            .field("conversation_id", &self.conversation_id)
            .field("kind", &self.kind)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
pub use connect_protocol::events::SnapshotItem as SnapshotFact;
#[derive(Clone)]
pub struct EventSnapshot {
    pub conversation: ConversationSnapshot,
    pub sessions: Vec<SessionSnapshot>,
    pub connections: Vec<ConnectionSnapshot>,
    pub streams: Vec<StreamSnapshot>,
    pub facts: Vec<SnapshotFact>,
}
impl std::fmt::Debug for EventSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventSnapshot")
            .field("conversation_id", &self.conversation.id)
            .field("facts", &self.facts.len())
            .finish_non_exhaustive()
    }
}
#[derive(Default)]
struct State {
    queue: VecDeque<(JournalEvent, usize)>,
    pending: Option<(JournalEvent, usize)>,
    bytes: usize,
    closed: bool,
    failure: Option<Error>,
    delivered: Option<Cursor>,
    seen: BTreeMap<String, [u8; 32]>,
    order: VecDeque<String>,
}
pub(in crate::sdk) struct Inbox {
    pub(in crate::sdk) cid: ConversationId,
    pub(in crate::sdk) signal: CancellationToken,
    stop: CancellationToken,
    state: std::sync::Mutex<State>,
    changed: Notify,
}
pub(in crate::sdk) struct Parsed {
    pub event: JournalEvent,
    pub bytes: usize,
    pub digest: [u8; 32],
}
#[track_caller]
fn bad() -> Error {
    let at = std::panic::Location::caller();
    tracing::debug!(target:"connect_client::sdk::events",file=at.file(),line=at.line(),"Journal fact validation refused");
    Error::request(ErrorKind::Protocol, None, false)
}
pub(in crate::sdk) fn parse(
    frame: &crate::canonical::Envelope,
    cid: &ConversationId,
) -> Result<Parsed> {
    use crate::canonical::Scope;
    use sha2::{Digest, Sha256};
    if frame.expected_revision().is_some() {
        return Err(bad());
    }
    let p = &frame.original()["payload"];
    let raw = &p["data"];
    let event: connect_protocol::events::JournalEvent =
        serde_json::from_value(raw.clone()).map_err(|_| bad())?;
    if event.conversation_id != cid.as_str()
        || frame.scope(Scope::Conversation) != Some(cid.as_str())
        || frame.scope(Scope::Session) != event.session_id.as_deref()
        || frame.scope(Scope::Connection) != event.connection_id.as_deref()
        || frame.scope(Scope::Operation) != event.operation_id.as_deref()
        || p["event_id"] != event.event_id
        || p["cursor"] != event.cursor
        || p["kind"] != event.kind
        || p["revision"] != event.revision
        || event.revision > 9_007_199_254_740_991
        || event.kind.is_empty()
        || event.kind.len() > 128
        || event.source.kind.is_empty()
        || event.source.kind.len() > 128
        || event.source.correlation_id.is_empty()
        || event.source.correlation_id.len() > 2048
        || event.audience.len() > 256
    {
        return Err(bad());
    }
    rvoip_uctp::v2::Identifier::try_from(event.event_id.clone()).map_err(|_| bad())?;
    let body = &raw["data"]["data"];
    for nested in [
        body.get("conversation_id"),
        body.get("session").and_then(|v| v.get("conversation_id")),
    ]
    .into_iter()
    .flatten()
    {
        if nested.as_str() != Some(cid.as_str()) {
            return Err(bad());
        }
    }
    for (field, expected) in [
        ("session_id", event.session_id.as_deref()),
        ("connection_id", event.connection_id.as_deref()),
        ("operation_id", event.operation_id.as_deref()),
    ] {
        if let (Some(actual), Some(expected)) = (body.get(field).and_then(Value::as_str), expected)
        {
            if actual != expected {
                return Err(bad());
            }
        }
    }
    if let (Some(id), Some(expected)) = (
        body.get("session")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str),
        event.session_id.as_deref(),
    ) {
        if id != expected {
            return Err(bad());
        }
    }
    if let (Some(kind), Some(id)) = (
        body.get("resource_kind").and_then(Value::as_str),
        body.get("resource_id").and_then(Value::as_str),
    ) {
        let expected = match kind {
            "conversation" => Some(cid.as_str()),
            "session" => event.session_id.as_deref(),
            "connection" => event.connection_id.as_deref(),
            "operation" => event.operation_id.as_deref(),
            _ => None,
        };
        if expected.is_some_and(|expected| expected != id) {
            return Err(bad());
        }
    }
    let mut audience = Vec::new();
    let mut unique = std::collections::BTreeSet::new();
    for member in event.audience {
        if member.membership_epoch == 0
            || member.membership_epoch > 9_007_199_254_740_991
            || !unique.insert(member.participant_id.clone())
        {
            return Err(bad());
        }
        audience.push(AudienceMember {
            participant_id: member.participant_id.parse().map_err(|_| bad())?,
            membership_epoch: member.membership_epoch,
        });
    }
    let mut immutable = raw.clone();
    immutable.as_object_mut().ok_or_else(bad)?.remove("cursor");
    let digest = Sha256::digest(serde_json::to_vec(&immutable).map_err(|_| bad())?).into();
    let bytes = serde_json::to_vec(raw).map_err(|_| bad())?.len();
    Ok(Parsed {
        event: JournalEvent {
            event_id: event.event_id,
            cursor: event.cursor.parse().map_err(|_| bad())?,
            conversation_id: cid.clone(),
            session_id: event
                .session_id
                .map(|v| v.parse().map_err(|_| bad()))
                .transpose()?,
            connection_id: event
                .connection_id
                .map(|v| v.parse().map_err(|_| bad()))
                .transpose()?,
            operation_id: event
                .operation_id
                .map(|v| v.parse().map_err(|_| bad()))
                .transpose()?,
            participant_id: event
                .participant_id
                .map(|v| v.parse().map_err(|_| bad()))
                .transpose()?,
            audience,
            kind: event.kind,
            revision: event.revision,
            source: EventSource {
                kind: event.source.kind,
                correlation_id: event.source.correlation_id,
            },
            observed_at: event.observed_at,
            data: raw["data"].clone(),
        },
        bytes,
        digest,
    })
}
impl Inbox {
    fn new(cid: ConversationId, after: Option<Cursor>, signal: CancellationToken) -> Self {
        Self {
            cid,
            signal,
            stop: CancellationToken::new(),
            state: std::sync::Mutex::new(State {
                delivered: after,
                ..State::default()
            }),
            changed: Notify::new(),
        }
    }
    fn check(&self) -> Result<()> {
        let state = self.state.lock().map_err(|_| bad())?;
        if state.closed || self.signal.is_cancelled() {
            return Err(Error::request(ErrorKind::Cancelled, None, false));
        }
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        Ok(())
    }
    pub(in crate::sdk) fn active(&self) -> bool {
        !self.signal.is_cancelled()
            && !self.stop.is_cancelled()
            && self
                .state
                .lock()
                .is_ok_and(|s| !s.closed && s.failure.is_none())
    }
    pub(in crate::sdk) fn cursor(&self) -> Option<Cursor> {
        self.state.lock().ok().and_then(|s| s.delivered.clone())
    }
    fn baseline(&self, cursor: Cursor) -> Result<()> {
        self.state.lock().map_err(|_| bad())?.delivered = Some(cursor);
        Ok(())
    }
    pub(in crate::sdk) fn fail(&self, error: Error) {
        if let Ok(mut s) = self.state.lock() {
            s.queue.clear();
            s.pending.take();
            s.bytes = 0;
            if !s.closed && s.failure.is_none() {
                s.failure = Some(error);
            }
        }
        self.changed.notify_waiters();
    }
    fn close(&self) {
        self.stop.cancel();
        if let Ok(mut s) = self.state.lock() {
            s.closed = true;
            s.queue.clear();
            s.pending.take();
            s.bytes = 0;
        }
        self.changed.notify_waiters();
    }
    pub(in crate::sdk) fn accept(&self, events: Vec<Parsed>) -> Result<()> {
        let mut s = self.state.lock().map_err(|_| bad())?;
        if s.closed || s.failure.is_some() || self.signal.is_cancelled() {
            return Ok(());
        }
        let mut fresh = Vec::new();
        let mut seen = BTreeMap::new();
        let mut bytes = 0usize;
        for item in events {
            if let Some(digest) = s
                .seen
                .get(&item.event.event_id)
                .or_else(|| seen.get(&item.event.event_id))
            {
                if *digest != item.digest {
                    return Err(bad());
                }
                continue;
            }
            bytes = bytes.saturating_add(item.bytes);
            seen.insert(item.event.event_id.clone(), item.digest);
            fresh.push(item);
        }
        if s.queue.len() + usize::from(s.pending.is_some()) + fresh.len() > 256
            || s.bytes.saturating_add(bytes) > 2 * 1024 * 1024
        {
            return Err(Error::request(ErrorKind::ResyncRequired, None, false));
        }
        for item in fresh {
            if s.order.len() == 256 {
                if let Some(id) = s.order.pop_front() {
                    s.seen.remove(&id);
                }
            }
            s.order.push_back(item.event.event_id.clone());
            s.seen.insert(item.event.event_id.clone(), item.digest);
            s.bytes += item.bytes;
            s.queue.push_back((item.event, item.bytes));
        }
        drop(s);
        self.changed.notify_waiters();
        Ok(())
    }
    async fn receive(&self, until: tokio::time::Instant) -> Result<Option<JournalEvent>> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut s = self.state.lock().map_err(|_| bad())?;
                if s.closed {
                    return Ok(None);
                }
                if self.signal.is_cancelled() {
                    return Err(Error::request(ErrorKind::Cancelled, None, false));
                }
                if let Some(error) = &s.failure {
                    return Err(error.clone());
                }
                if tokio::time::Instant::now() >= until {
                    return Err(Error::request(ErrorKind::Deadline, None, false));
                }
                if s.pending.is_none() {
                    s.pending = s.queue.pop_front();
                }
                if let Some((event, _)) = &s.pending {
                    return Ok(Some(event.clone()));
                }
            }
            tokio::select! {biased;
                _=self.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,None,false)),
                _=tokio::time::sleep_until(until)=>return Err(Error::request(ErrorKind::Deadline,None,false)),
                _=&mut changed=>(),
            }
        }
    }
    fn delivered(&self, event: &JournalEvent) -> Result<()> {
        self.check()?;
        let mut s = self.state.lock().map_err(|_| bad())?;
        let (pending, bytes) = s.pending.as_ref().ok_or_else(bad)?;
        if pending.event_id != event.event_id || pending.cursor != event.cursor {
            return Err(bad());
        }
        s.bytes = s.bytes.saturating_sub(*bytes);
        s.pending.take();
        s.delivered = Some(event.cursor.clone());
        Ok(())
    }
}
pub struct EventStream {
    client: Option<Client>,
    inbox: Arc<Inbox>,
    options: RequestOptions,
    opened: bool,
    opening: Option<tokio::task::JoinHandle<Result<Option<EventSnapshot>>>>,
    snapshot: Option<EventSnapshot>,
}
impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventStream")
            .field("conversation_id", &self.inbox.cid)
            .finish_non_exhaustive()
    }
}
impl Drop for EventStream {
    fn drop(&mut self) {
        self.inbox.close();
        if let Some(job) = self.opening.take() {
            job.abort();
        }
    }
}
impl Conversation {
    /// A lazy real live subscription. None obtains a complete current snapshot
    /// first. Supplied cursors resume their original identity/epoch-bound journal.
    pub fn events(&self, after: Option<Cursor>, options: RequestOptions) -> EventStream {
        EventStream {
            client: Some(self.client().clone()),
            inbox: Arc::new(Inbox::new(self.id().clone(), after, options.signal.clone())),
            options,
            opened: false,
            opening: None,
            snapshot: None,
        }
    }
}
impl EventStream {
    /// Last returned fact or explicit snapshot/resume boundary. Persist each
    /// event cursor only after your application has actually processed it.
    pub fn cursor(&self) -> Option<Cursor> {
        self.inbox.cursor()
    }
    async fn open(&mut self, until: tokio::time::Instant) -> Result<()> {
        self.inbox.check()?;
        if self.opened {
            return Ok(());
        }
        if self.opening.is_none() {
            let client = self
                .client
                .as_ref()
                .ok_or_else(|| Error::request(ErrorKind::Cancelled, None, false))?
                .clone();
            let inbox = self.inbox.clone();
            self.opening = Some(tokio::spawn(async move {
                let work = async {
                    let mut snapshot = None;
                    if inbox.cursor().is_none() {
                        let remaining =
                            until.saturating_duration_since(tokio::time::Instant::now());
                        if remaining.is_zero() {
                            return Err(Error::request(ErrorKind::Deadline, None, false));
                        }
                        let current = snapshot::read(
                            &client,
                            &inbox.cid,
                            RequestOptions {
                                timeout: remaining,
                                signal: inbox.stop.clone(),
                            },
                        )
                        .await?;
                        inbox.baseline(current.conversation.cursor.parse().map_err(|_| bad())?)?;
                        snapshot = Some(EventSnapshot {
                            conversation: current.conversation,
                            sessions: current.sessions.into_values().collect(),
                            connections: current.connections.into_values().collect(),
                            streams: current.streams.into_values().collect(),
                            facts: current.facts,
                        });
                    }
                    let remaining = until.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        return Err(Error::request(ErrorKind::Deadline, None, false));
                    }
                    client
                        .shared
                        .subscribe_events(
                            inbox.clone(),
                            RequestOptions {
                                timeout: remaining,
                                signal: inbox.stop.clone(),
                            },
                        )
                        .await?;
                    Ok(snapshot)
                };
                let result = tokio::select! {biased;
                    _=inbox.signal.cancelled()=>Err(Error::request(ErrorKind::Cancelled,None,false)),
                    _=inbox.stop.cancelled()=>Err(Error::request(ErrorKind::Cancelled,None,false)),
                    result=work=>result,
                };
                if let Err(failure) = &result {
                    inbox.fail(failure.clone());
                }
                result
            }));
        }
        let result = tokio::time::timeout_at(
            until,
            self.opening.as_mut().expect("owned event initialization"),
        )
        .await
        .map_err(|_| Error::request(ErrorKind::Deadline, None, false))?;
        self.opened = true;
        self.opening.take();
        match result {
            Ok(Ok(snapshot)) => {
                self.snapshot = snapshot;
                Ok(())
            }
            Ok(Err(failure)) => Err(failure),
            Err(_) => {
                let failure = bad();
                self.inbox.fail(failure.clone());
                Err(failure)
            }
        }
    }
    /// Complete bootstrap metadata, or None when resuming an explicit cursor.
    /// Snapshot observations never grant native transport or Stream ownership.
    pub async fn snapshot(&mut self) -> Result<Option<EventSnapshot>> {
        self.options.validate()?;
        let until = tokio::time::Instant::now() + self.options.timeout;
        self.open(until).await?;
        Ok(self.snapshot.clone())
    }
    /// Each next has its configured finite timeout. A timed-out established
    /// wait consumes nothing; closing/cancelling this stream leaves control live.
    /// Before returning a completed native routing fact, read its original
    /// journal under current authority. Interrupted catch-up retains the same
    /// charged fact and previous cursor; it never replays a mutation.
    pub async fn next(&mut self) -> Result<Option<JournalEvent>> {
        self.options.validate()?;
        let until = tokio::time::Instant::now() + self.options.timeout;
        if self.client.is_none() {
            return Ok(None);
        }
        self.open(until).await?;
        let event = self.inbox.receive(until).await?;
        if let Some(event) = &event {
            self.client
                .as_ref()
                .expect("opened event owner")
                .catch_up_native_event(event, &self.options, until)
                .await?;
            self.inbox.delivered(event)?;
        }
        Ok(event)
    }
    /// Local detach stops delivery/renewal immediately. The bounded existing
    /// server lease expires within10 seconds; no unsupported unsubscribe is sent.
    pub async fn close(&mut self) -> Result<()> {
        self.inbox.close();
        if let Some(job) = self.opening.take() {
            job.abort();
            let _ = job.await;
        }
        self.client.take();
        Ok(())
    }
}
