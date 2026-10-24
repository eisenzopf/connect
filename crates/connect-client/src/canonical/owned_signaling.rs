//! One physical signaling reader survives cancellation of atomic audio reads.
use super::*;
use std::sync::Mutex;
use tokio::sync::{watch, Notify};

#[derive(Default)]
struct Inbox {
    frames: VecDeque<Envelope>,
    bytes: usize,
    failure: Option<ClientError>,
    ended: bool,
}
impl Inbox {
    fn push(&mut self, frame: Envelope) -> Result<()> {
        let passive = |e: &Envelope| {
            matches!(
                e.kind(),
                "connection.ready"
                    | "connection.quality"
                    | "stream.opened"
                    | "stream.closed"
                    | "capability.advertise"
            ) && e.scope(Scope::Reply).is_none()
                && e.scope(Scope::Operation).is_none()
        };
        // Only advisory facts for the same exact resource and kind coalesce.
        // Replies, data, DTMF and unknown future signaling stay ordered and
        // lossless within the explicit inbox bound.
        if passive(&frame) {
            if let Some(index) = self.frames.iter().position(|old| {
                passive(old)
                    && old.kind() == frame.kind()
                    && ["cid", "sid", "connid"]
                        .iter()
                        .all(|key| old.original()[key] == frame.original()[key])
                    && old.original()["payload"]["strm_id"]
                        == frame.original()["payload"]["strm_id"]
                    && old.original()["payload"]["by_participant"]
                        == frame.original()["payload"]["by_participant"]
            }) {
                if let Some(old) = self.frames.remove(index) {
                    self.bytes = self.bytes.saturating_sub(encoded_size(&old)?);
                }
            }
        }
        let size = encoded_size(&frame)?;
        if self.frames.len() >= MAX_BATCH_FRAMES
            || self.bytes.saturating_add(size) > MAX_BATCH_BYTES
        {
            return Err(ClientError::Protocol);
        }
        self.bytes += size;
        self.frames.push_back(frame);
        Ok(())
    }
    fn next(&mut self) -> Result<Option<Envelope>> {
        if let Some(frame) = self.frames.pop_front() {
            self.bytes = self.bytes.saturating_sub(encoded_size(&frame)?);
            return Ok(Some(frame));
        }
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        if self.ended {
            return Err(ClientError::Transport);
        }
        Ok(None)
    }
}

struct ExpectedStepUp {
    response: String,
    previous: String,
    required: String,
    scope: CommandScope,
    deadline: tokio::time::Instant,
    observed: bool,
}
/// A bounded exception to terminal assurance handling, not new authority. The
/// foreground still validates session, scope, identity, token and transition
/// before renewing the reader's credential deadline.
pub(super) struct StepUpReaderPermit {
    expected: Arc<Mutex<Option<ExpectedStepUp>>>,
    response: String,
    connection: quinn::Connection,
    closed: Arc<AtomicBool>,
    verified: bool,
}
impl StepUpReaderPermit {
    pub(super) fn verified(&mut self) -> Result<()> {
        let expected = self.expected.lock().map_err(|_| ClientError::Protocol)?;
        if !expected.as_ref().is_some_and(|e| {
            e.response == self.response && e.observed && tokio::time::Instant::now() < e.deadline
        }) {
            return Err(ClientError::Authentication);
        }
        self.verified = true;
        Ok(())
    }
}
impl Drop for StepUpReaderPermit {
    fn drop(&mut self) {
        let mut expected = self.expected.lock().unwrap_or_else(|p| p.into_inner());
        if expected
            .as_ref()
            .is_some_and(|e| e.response == self.response)
        {
            let observed = expected.take().is_some_and(|e| e.observed);
            if observed && !self.verified {
                self.closed.store(true, Ordering::Release);
                self.connection
                    .close(0u32.into(), b"unvalidated owned assurance transition");
            }
        }
    }
}
fn observe_expected_step_up(frame: &Envelope, expected: &Mutex<Option<ExpectedStepUp>>) -> bool {
    let Ok(mut expected) = expected.lock() else {
        return false;
    };
    let Some(e) = expected.as_mut() else {
        return false;
    };
    let Payload::IdentityAssuranceChanged(change) = frame.payload() else {
        return false;
    };
    if e.observed
        || tokio::time::Instant::now() >= e.deadline
        || frame.scope(Scope::Reply) != Some(e.response.as_str())
        || check_reply_scope(frame, &e.scope, false).is_err()
        || change.previous != e.previous
        || change.current != e.required
        || change.reason != "verified-step-up"
    {
        return false;
    }
    e.observed = true;
    true
}

pub(super) struct OwnedSignaling {
    inbox: Arc<Mutex<Inbox>>,
    changed: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
    authority: watch::Sender<chrono::DateTime<chrono::Utc>>,
    pin_until: chrono::DateTime<chrono::Utc>,
    expected_step_up: Arc<Mutex<Option<ExpectedStepUp>>>,
    connection: quinn::Connection,
    closed: Arc<AtomicBool>,
}
impl Drop for OwnedSignaling {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl OwnedSignaling {
    pub(super) fn start(
        mut reader: quinn::RecvStream,
        verifier: Arc<Sig9421Verifier>,
        verification: SignatureVerificationContext,
        connection: quinn::Connection,
        closed: Arc<AtomicBool>,
        until: chrono::DateTime<chrono::Utc>,
        pin_until: chrono::DateTime<chrono::Utc>,
        deadline: Duration,
    ) -> Self {
        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let changed = Arc::new(Notify::new());
        let received = inbox.clone();
        let notify = changed.clone();
        let (authority, mut changes) = watch::channel(until.min(pin_until));
        let current_authority = authority.subscribe();
        let expected_step_up = Arc::new(Mutex::new(None));
        let step_up = expected_step_up.clone();
        let peer = connection.clone();
        let retired = closed.clone();
        let task = tokio::spawn(async move {
            // Updating the timer must never cancel an unfinished framed read.
            // Only a fully validated refresh may publish a successor deadline.
            let expiration = async {
                loop {
                    let until = *changes.borrow_and_update();
                    let remaining = (until - chrono::Utc::now()).to_std().unwrap_or_default();
                    tokio::select! {
                        _ = tokio::time::sleep(remaining) => {
                            if chrono::Utc::now() >= *changes.borrow() {
                                return Err(ClientError::Authentication);
                            }
                        }
                        changed = changes.changed() => {
                            if changed.is_err() { return Err(ClientError::Transport); }
                        }
                    }
                }
            };
            let work = async {
                loop {
                    if chrono::Utc::now() >= *current_authority.borrow() {
                        return Err(ClientError::Authentication);
                    }
                    // Idle audio is healthy. The frame deadline starts only
                    // when its first byte arrives; credential/key expiry still
                    // bounds this entire owned read, including an idle peer.
                    let mut prefix = [0; 4];
                    reader
                        .read_exact(&mut prefix[..1])
                        .await
                        .map_err(|_| ClientError::Transport)?;
                    let frame = tokio::time::timeout(deadline, async {
                        reader
                            .read_exact(&mut prefix[1..])
                            .await
                            .map_err(|_| ClientError::Transport)?;
                        let size = u32::from_be_bytes(prefix) as usize;
                        if size == 0 || size > MAX_ENVELOPE_BYTES {
                            return Err(ClientError::Protocol);
                        }
                        let mut bytes = vec![0; size];
                        reader
                            .read_exact(&mut bytes)
                            .await
                            .map_err(|_| ClientError::Transport)?;
                        let frame = Envelope::parse(&bytes).map_err(|_| ClientError::Protocol)?;
                        verifier
                            .verify_with_context(frame.original(), &verification)
                            .await
                            .map_err(|_| ClientError::Authentication)?;
                        Ok(frame)
                    })
                    .await
                    .map_err(|_| ClientError::Timeout)??;
                    if chrono::Utc::now() >= *current_authority.borrow() {
                        return Err(ClientError::Authentication);
                    }
                    let terminal = frame.kind() == "auth.bye"
                        || (frame.kind() == "identity.assurance-changed"
                            && !observe_expected_step_up(&frame, &step_up));
                    received
                        .lock()
                        .map_err(|_| ClientError::Protocol)?
                        .push(frame)?;
                    notify.notify_one();
                    if terminal {
                        return Err(ClientError::Transport);
                    }
                    tokio::task::consume_budget().await;
                }
            };
            let result: Result<()> =
                tokio::select! { result = work => result, result = expiration => result };
            {
                let mut inbox = received.lock().unwrap_or_else(|p| p.into_inner());
                inbox.failure = Some(result.err().unwrap_or(ClientError::Transport));
                inbox.ended = true;
            }
            closed.store(true, Ordering::Release);
            // There is no continuing background read after owner retirement,
            // partial-frame failure, expiry or a verified terminal auth fact.
            connection.close(0u32.into(), b"owned native signaling retired");
            notify.notify_waiters();
        });
        Self {
            inbox,
            changed,
            task,
            authority,
            pin_until,
            expected_step_up,
            connection: peer,
            closed: retired,
        }
    }
    pub(super) fn expect_step_up(
        &self,
        response: &str,
        previous: &str,
        required: &str,
        scope: CommandScope,
        deadline: tokio::time::Instant,
    ) -> Result<StepUpReaderPermit> {
        let inbox = self.inbox.lock().map_err(|_| ClientError::Protocol)?;
        if inbox.ended
            || self.closed.load(Ordering::Acquire)
            || chrono::Utc::now() >= *self.authority.borrow()
            || tokio::time::Instant::now() >= deadline
        {
            return Err(ClientError::Authentication);
        }
        let mut expected = self
            .expected_step_up
            .lock()
            .map_err(|_| ClientError::Protocol)?;
        if expected.is_some() {
            return Err(ClientError::Protocol);
        }
        *expected = Some(ExpectedStepUp {
            response: response.into(),
            previous: previous.into(),
            required: required.into(),
            scope,
            deadline,
            observed: false,
        });
        Ok(StepUpReaderPermit {
            expected: self.expected_step_up.clone(),
            response: response.into(),
            connection: self.connection.clone(),
            closed: self.closed.clone(),
            verified: false,
        })
    }
    pub(super) fn refresh_until(&self, until: chrono::DateTime<chrono::Utc>) -> Result<()> {
        let inbox = self.inbox.lock().map_err(|_| ClientError::Protocol)?;
        let now = chrono::Utc::now();
        let until = until.min(self.pin_until);
        if inbox.ended || now >= *self.authority.borrow() || until <= now {
            return Err(ClientError::Authentication);
        }
        self.authority.send_replace(until);
        Ok(())
    }
    pub(super) async fn receive(&self) -> Result<Envelope> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(frame) = self
                .inbox
                .lock()
                .map_err(|_| ClientError::Protocol)?
                .next()?
            {
                return Ok(frame);
            }
            changed.await;
        }
    }
}

impl CanonicalClient {
    /// Start a bounded, signature-verified signaling inbox for manual native
    /// media loops. Cancelling a subsequent `receive()` wait leaves this single
    /// reader active, so an application can service media on a timer. Replies,
    /// data and DTMF stay ordered; advisory facts for the same resource may
    /// coalesce. Audio helpers already enable this reader. Repeated calls reuse
    /// it while the peer's current authentication remains valid.
    pub fn start_signaling_reader(&mut self) -> Result<()> {
        self.own_signaling()
    }

    pub(super) fn own_signaling(&mut self) -> Result<()> {
        let mut current = self.pending()?;
        if self.owned_signaling.is_some() {
            current.complete = true;
            return Ok(());
        }
        let reader = self.reader.take().ok_or(ClientError::Protocol)?;
        self.owned_signaling = Some(OwnedSignaling::start(
            reader,
            self.verifier.clone(),
            self.verification.clone(),
            self.connection.clone(),
            self.closed.clone(),
            self.authenticated_until.min(self.server_key_expires_at),
            self.server_key_expires_at,
            self.deadline,
        ));
        current.complete = true;
        Ok(())
    }
}
