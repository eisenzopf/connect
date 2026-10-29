//! One SDK pending fact remains charged until its exact local acknowledgement.
use super::*;
use tokio::time::{timeout_at, Instant};
enum Observed {
    Ready(Cursor),
    Fact(Option<JournalEvent>, Option<Cursor>),
}
pub(super) struct Reader {
    client: Client,
    cid: ConversationId,
    after: Option<Cursor>,
    baseline: Option<Cursor>,
    acknowledged: Option<EventAcknowledgement>,
    exposed: Option<EventAcknowledgement>,
    stream: Arc<tokio::sync::Mutex<EventStream>>,
    health: Arc<crate::sdk::events::Inbox>,
    work: Option<tokio::task::JoinHandle<Result<Observed>>>,
}
impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(work) = &self.work {
            work.abort();
        }
    }
}
impl Reader {
    pub(super) fn new(client: &Client, cid: ConversationId, after: Option<Cursor>) -> Result<Self> {
        let stream = client
            .conversation_by_id(cid.clone())
            .events(after.clone(), RequestOptions::new(Duration::from_secs(30))?);
        let health = stream.helper_inbox();
        Ok(Self {
            client: client.clone(),
            cid,
            after,
            baseline: None,
            acknowledged: None,
            exposed: None,
            stream: Arc::new(tokio::sync::Mutex::new(stream)),
            health,
            work: None,
        })
    }
    pub(super) fn conversation_id(&self) -> &ConversationId {
        &self.cid
    }
    pub(super) async fn poll(
        &mut self,
        reader: &OperationId,
        cid: &ConversationId,
        after: Option<&Cursor>,
        ack: Option<EventAcknowledgement>,
        ready: bool,
        until: Instant,
    ) -> Result<Value> {
        if cid != &self.cid || after != self.after.as_ref() {
            return Err(failure(ErrorKind::Validation));
        }
        self.health.check()?;
        if let Some(ack) = ack {
            if self.acknowledged.as_ref() != Some(&ack) {
                if self.work.is_some() || self.exposed.as_ref() != Some(&ack) {
                    return Err(failure(ErrorKind::Validation));
                }
                let stream = timeout_at(until, self.stream.lock())
                    .await
                    .map_err(|_| failure(ErrorKind::Deadline))?;
                stream.helper_acknowledge(&ack.event_id, &ack.cursor)?;
                self.acknowledged = Some(ack);
                self.exposed = None;
            }
        }
        if ready && self.baseline.is_some() {
            return Ok(self.reply(reader, None, false));
        }
        if self.work.is_none() {
            let stream = self.stream.clone();
            self.work = Some(tokio::spawn(async move {
                let mut stream = stream.lock().await;
                if ready {
                    stream.snapshot().await?;
                    Ok(Observed::Ready(
                        stream
                            .cursor()
                            .ok_or_else(|| failure(ErrorKind::Protocol))?,
                    ))
                } else {
                    let event = stream.helper_peek().await?;
                    Ok(Observed::Fact(event, stream.cursor()))
                }
            }));
        }
        let result = timeout_at(
            until.min(Instant::now() + Duration::from_millis(25)),
            self.work.as_mut().unwrap(),
        )
        .await;
        let result = match result {
            Err(_) => return Ok(self.reply(reader, None, false)),
            Ok(result) => {
                self.work.take();
                result.map_err(|_| failure(ErrorKind::Protocol))??
            }
        };
        self.health.check()?;
        match result {
            Observed::Ready(cursor) => {
                if self.baseline.is_none() {
                    self.baseline = Some(cursor);
                }
                Ok(self.reply(reader, None, false))
            }
            Observed::Fact(event, cursor) => {
                if self.baseline.is_none() {
                    self.baseline = cursor;
                }
                let done = event.is_none();
                if let Some(event) = &event {
                    if !ready {
                        let remaining = until.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            return Err(failure(ErrorKind::Deadline));
                        }
                        // Work may have completed before a cancelled local wait.
                        // Local capability support/health is not a current grant.
                        self.client
                            .reauthorize_journal(
                                &self.cid,
                                &event.cursor,
                                RequestOptions::new(remaining)?,
                            )
                            .await?;
                        self.health.check()?;
                        self.exposed = Some(EventAcknowledgement {
                            event_id: event.event_id.clone(),
                            cursor: event.cursor.clone(),
                        });
                    }
                }
                let value = if ready {
                    None
                } else {
                    event.as_ref().map(event_json).transpose()?
                };
                Ok(self.reply(reader, value, done))
            }
        }
    }
    fn reply(&self, reader: &OperationId, event: Option<Value>, done: bool) -> Value {
        json!({"reader_id":reader,"conversation_id":self.cid,"ready":self.baseline.is_some(),"cursor":self.baseline,"event":event,"done":done})
    }
    pub(super) async fn close(mut self, until: Instant) -> Result<()> {
        if let Some(work) = self.work.take() {
            work.abort();
            let _ = timeout_at(until, work).await;
        }
        let mut stream = timeout_at(until, self.stream.lock())
            .await
            .map_err(|_| failure(ErrorKind::CleanupIncomplete))?;
        timeout_at(until, stream.close())
            .await
            .map_err(|_| failure(ErrorKind::CleanupIncomplete))?
    }
}
fn event_json(event: &JournalEvent) -> Result<Value> {
    let value = json!({"eventId":event.event_id,"cursor":event.cursor,"conversationId":event.conversation_id,
        "sessionId":event.session_id,"connectionId":event.connection_id,"operationId":event.operation_id,
        "participantId":event.participant_id,"audience":event.audience.iter().map(|member|json!({"participantId":member.participant_id,"membershipEpoch":member.membership_epoch})).collect::<Vec<_>>(),
        "kind":event.kind,"revision":event.revision,"source":{"kind":event.source.kind,"correlationId":event.source.correlation_id},"observedAt":event.observed_at,"data":event.data});
    if serde_json::to_vec(&value)
        .map_err(|_| failure(ErrorKind::Protocol))?
        .len()
        > 2 * 1024 * 1024 - 1024
    {
        return Err(failure(ErrorKind::Protocol));
    }
    Ok(value)
}
