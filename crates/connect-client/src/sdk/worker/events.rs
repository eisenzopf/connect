//! Live event sources share the one signed reader and explicit control capacity.
use super::super::events as api;
use super::*;
use crate::canonical::{exchange::Exchange, Envelope, Scope};
use std::collections::VecDeque;
use tokio::time::Instant;
const LEASE_SECONDS: u64 = 10;
const SOURCE_LIMIT: usize = 8;
const RETIRED_LIMIT: usize = 64;
struct Source {
    inbox: Arc<api::Inbox>,
    exchange: Exchange,
    cursor: Cursor,
    deadline: Instant,
    renew: Instant,
    startup: Option<oneshot::Sender<Result<ReplyBatch>>>,
    slot: Option<OwnedSemaphorePermit>,
}
#[derive(Default)]
pub(super) struct Driver {
    sources: Vec<Source>,
    retired: VecDeque<(String, Instant)>,
}
fn error(kind: ErrorKind) -> Error {
    Error::request(kind, None, false)
}
struct Page {
    events: Vec<api::Parsed>,
    cursor: Cursor,
    deadline: Instant,
    renew: Instant,
}
fn page(batch: &ReplyBatch, inbox: &api::Inbox) -> Result<Page> {
    super::super::check_batch(batch, None)?;
    let mut events = Vec::new();
    for frame in &batch.frames[..batch.frames.len().saturating_sub(1)] {
        match frame.kind() {
            "conversation.event" => events.push(api::parse(frame, &inbox.cid)?),
            "conversation.snapshot" => return Err(error(ErrorKind::ResyncRequired)),
            "session.started"
            | "session.ended"
            | "session.participant.joined"
            | "session.participant.left"
            | "conversation.closed"
            | "recording.vcon-ready"
            | "message.read"
            | "message.delivered" => (),
            _ => return Err(error(ErrorKind::Protocol)),
        }
    }
    if events.len() > 32 {
        return Err(error(ErrorKind::Protocol));
    }
    let details = batch
        .ack()
        .map_err(|e| Error::transport(e, None))?
        .details
        .as_ref()
        .ok_or_else(|| error(ErrorKind::Protocol))?;
    if details["resync_required"] == true || details["live"] == false {
        return Err(error(ErrorKind::ResyncRequired));
    }
    if details["resync_required"] != false
        || details["live"] != true
        || details["has_more"].as_bool().is_none()
    {
        return Err(error(ErrorKind::Protocol));
    }
    let cursor: Cursor = serde_json::from_value(details["cursor"].clone())
        .map_err(|_| error(ErrorKind::Protocol))?;
    let lease = details["lease"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 2048 && s.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or_else(|| error(ErrorKind::Protocol))?;
    let _ = lease;
    let expires = details["expires_at"]
        .as_i64()
        .ok_or_else(|| error(ErrorKind::Protocol))?;
    let remaining = (chrono::DateTime::from_timestamp(expires, 0)
        .ok_or_else(|| error(ErrorKind::Protocol))?
        - chrono::Utc::now())
    .to_std()
    .map_err(|_| error(ErrorKind::ResyncRequired))?;
    if remaining > Duration::from_secs(LEASE_SECONDS + 1) {
        return Err(error(ErrorKind::Protocol));
    }
    let deadline = Instant::now() + remaining;
    let renew = deadline - Duration::from_millis(500).min(remaining);
    Ok(Page {
        events,
        cursor,
        deadline,
        renew,
    })
}
fn payload(cursor: &Cursor) -> Value {
    serde_json::json!({"after":cursor.as_str(),"live":true,"limit":32,"lease_seconds":LEASE_SECONDS})
}
impl Driver {
    fn retire(&mut self, source: Source, failure: Option<Error>) {
        if let Some(failure) = &failure {
            source.inbox.fail(failure.clone());
        }
        if let Some(startup) = source.startup {
            let _ = startup.send(Err(failure.unwrap_or_else(|| error(ErrorKind::Cancelled))));
        }
        self.retired.push_back((
            source.exchange.id.clone(),
            Instant::now() + Duration::from_secs(LEASE_SECONDS + 2),
        ));
    }
    pub(super) fn can_start(&mut self, inbox: &api::Inbox) -> Result<()> {
        self.prune();
        if !inbox.active() {
            return Err(error(ErrorKind::Cancelled));
        }
        if self.sources.iter().any(|s| s.inbox.cid == inbox.cid) {
            return Err(error(ErrorKind::Conflict));
        }
        if self.sources.len() >= SOURCE_LIMIT
            || self.retired.len() + self.sources.len() >= RETIRED_LIMIT
        {
            return Err(error(ErrorKind::Configuration));
        }
        if inbox.cursor().is_none() {
            return Err(error(ErrorKind::Protocol));
        }
        Ok(())
    }
    fn prune(&mut self) {
        self.retired.retain(|(_, end)| *end > Instant::now());
        for index in (0..self.sources.len()).rev() {
            if !self.sources[index].inbox.active() {
                let source = self.sources.remove(index);
                let failure = source
                    .inbox
                    .signal
                    .is_cancelled()
                    .then(|| error(ErrorKind::Cancelled));
                self.retire(source, failure);
            }
        }
    }
    pub(super) fn start(
        &mut self,
        exchange: Exchange,
        inbox: Arc<api::Inbox>,
        startup: oneshot::Sender<Result<ReplyBatch>>,
        slot: OwnedSemaphorePermit,
    ) {
        let cursor = inbox.cursor().expect("validated subscription cursor");
        let deadline = exchange.deadline;
        self.sources.push(Source {
            inbox,
            exchange,
            cursor,
            deadline,
            renew: deadline,
            startup: Some(startup),
            slot: Some(slot),
        });
    }
    pub(super) fn wakeup(&self) -> Option<Instant> {
        (!self.sources.is_empty()).then(|| Instant::now() + Duration::from_millis(100))
    }
    pub(super) async fn tick(
        &mut self,
        peer: &mut CanonicalClient,
        slots: &Arc<Semaphore>,
    ) -> Result<()> {
        self.prune();
        for index in (0..self.sources.len()).rev() {
            if Instant::now() >= self.sources[index].deadline {
                let source = self.sources.remove(index);
                let kind = if source.startup.is_some() {
                    ErrorKind::Deadline
                } else {
                    ErrorKind::ResyncRequired
                };
                self.retire(source, Some(error(kind)));
            }
        }
        let ready = self
            .sources
            .iter()
            .position(|s| s.slot.is_none() && s.renew <= Instant::now() && s.exchange.is_empty());
        if let Some(index) = ready {
            if slots.available_permits() <= 1
                || self.retired.len() + self.sources.len() >= RETIRED_LIMIT
            {
                return Ok(());
            }
            let Ok(slot) = slots.clone().try_acquire_owned() else {
                return Ok(());
            };
            let mut source = self.sources.remove(index);
            let scope = CommandScope {
                conversation_id: Some(source.inbox.cid.as_str().into()),
                ..Default::default()
            };
            let exchange = peer
                .begin_exchange("conversation.subscribe", payload(&source.cursor), scope)
                .await
                .map_err(|e| Error::transport(peer.with_owned_signaling_cause(e), None))?;
            self.retired.push_back((
                source.exchange.id.clone(),
                Instant::now() + Duration::from_secs(LEASE_SECONDS + 2),
            ));
            source.deadline = exchange.deadline;
            source.exchange = exchange;
            source.slot = Some(slot);
            self.sources.push(source);
        }
        Ok(())
    }
    pub(super) fn frame(&mut self, peer: &mut CanonicalClient, frame: &Envelope) -> Result<bool> {
        let Some(reply) = frame.scope(Scope::Reply) else {
            return Ok(false);
        };
        if self
            .retired
            .iter()
            .any(|(id, end)| id == reply && *end > Instant::now())
        {
            return Ok(true);
        }
        let Some(index) = self.sources.iter().position(|s| s.exchange.id == reply) else {
            return Ok(false);
        };
        let accepted = peer.accept_exchange_frame(&mut self.sources[index].exchange, frame.clone());
        match accepted {
            Ok(false) => (),
            Err(_) => {
                tracing::debug!(target:"connect_client::sdk::events",kind=frame.kind(),"Live batch reply scope refused");
                let source = self.sources.remove(index);
                self.retire(source, Some(error(ErrorKind::Protocol)));
            }
            Ok(true) => {
                let mut source = self.sources.remove(index);
                let next = source.exchange.continuation(source.deadline);
                let batch = std::mem::replace(&mut source.exchange, next).finish();
                match page(&batch, &source.inbox).and_then(|mut page| {
                    source.inbox.accept(std::mem::take(&mut page.events))?;
                    Ok(page)
                }) {
                    Ok(page) => {
                        source.cursor = page.cursor;
                        source.deadline = page.deadline;
                        source.renew = page.renew;
                        source.exchange.deadline = page.deadline;
                        source.slot.take();
                        if let Some(startup) = source.startup.take() {
                            let _ = startup.send(Ok(batch));
                        }
                        self.sources.push(source);
                    }
                    Err(failure) => {
                        tracing::debug!(target:"connect_client::sdk::events",?failure,"Live event batch refused");
                        self.retire(source, Some(failure));
                    }
                }
            }
        }
        Ok(true)
    }
    pub(super) fn end(&mut self, kind: ErrorKind) {
        while let Some(source) = self.sources.pop() {
            self.retire(source, Some(error(kind)));
        }
    }
}
impl Shared {
    pub(in crate::sdk) async fn subscribe_events(
        &self,
        inbox: Arc<api::Inbox>,
        options: RequestOptions,
    ) -> Result<ReplyBatch> {
        let cursor = inbox.cursor().ok_or_else(|| error(ErrorKind::Protocol))?;
        let scope = CommandScope {
            conversation_id: Some(inbox.cid.as_str().into()),
            ..Default::default()
        };
        self.request_inner(
            false,
            Some(inbox),
            "conversation.subscribe",
            payload(&cursor),
            scope,
            None,
            options,
        )
        .await
    }
}
