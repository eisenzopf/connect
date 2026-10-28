//! Owned helper readers. A bounded poll never acknowledges a pending delivery.
use super::*;

#[derive(Default)]
struct Delivery {
    reader: Option<OperationId>,
    acknowledged: Option<OperationId>,
    pending: Option<(OperationId, Value)>,
}
fn failure(kind: ErrorKind) -> Error {
    Error::request(kind, None, false)
}
impl Delivery {
    fn claim(&mut self, reader: &OperationId, acknowledged: Option<&OperationId>) -> Result<()> {
        if self
            .reader
            .as_ref()
            .is_some_and(|current| current != reader)
        {
            return Err(failure(ErrorKind::Conflict));
        }
        if self.reader.is_none() {
            if acknowledged.is_some() {
                return Err(failure(ErrorKind::Validation));
            }
            self.reader = Some(reader.clone());
        }
        if let Some(acknowledged) = acknowledged {
            if self.acknowledged.as_ref() != Some(acknowledged) {
                if self.pending.as_ref().map(|(id, _)| id) != Some(acknowledged) {
                    return Err(failure(ErrorKind::Validation));
                }
                self.pending.take();
                self.acknowledged = Some(acknowledged.clone());
            }
        }
        Ok(())
    }
    fn detach(&mut self) {
        self.reader.take();
        self.acknowledged.take();
        // An unacknowledged hint stays available to the next local reader.
    }
}
#[derive(Default)]
struct Incoming {
    delivery: Delivery,
    stream: Option<InvitationStream>,
}
pub struct ManagedClient {
    client: Client,
    incoming: tokio::sync::Mutex<Incoming>,
}
impl ManagedClient {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            incoming: Default::default(),
        }
    }
    pub async fn execute(&self, request: Request) -> Result<Value> {
        let options = RequestOptions::new(Duration::from_millis(request.timeout_ms))?;
        let until = tokio::time::Instant::now() + options.timeout;
        match request.action {
            Action::IncomingPoll {
                reader_id,
                acknowledged,
            } => {
                // Local runtime closure/expiry still fences retained metadata.
                self.client.capabilities().refresh(options.clone()).await?;
                self.client.shared.invitations.check()?;
                let mut state = tokio::time::timeout_at(until, self.incoming.lock())
                    .await
                    .map_err(|_| failure(ErrorKind::Deadline))?;
                state.delivery.claim(&reader_id, acknowledged.as_ref())?;
                if state.delivery.pending.is_none() {
                    if state.stream.is_none() {
                        state.stream =
                            Some(self.client.incoming_invitations(RequestOptions::new(
                                Duration::from_secs(30),
                            )?));
                    }
                    let received = tokio::time::timeout_at(
                        until.min(tokio::time::Instant::now() + Duration::from_millis(25)),
                        state.stream.as_mut().unwrap().next(),
                    )
                    .await;
                    match received {
                        Err(_) => {}
                        Ok(Err(error)) => return Err(error),
                        Ok(Ok(None)) => return Err(failure(ErrorKind::Cancelled)),
                        Ok(Ok(Some(hint))) => {
                            let medium = match hint.medium {
                                SessionMedium::Voice => "voice",
                                SessionMedium::Video => "video",
                                SessionMedium::VoiceVideo => "voice+video",
                                SessionMedium::ScreenShare => "screen-share",
                                SessionMedium::TextChat => "text-chat",
                                SessionMedium::Mixed => "mixed",
                            };
                            let value = json!({
                                "conversation_id":hint.session.conversation_id(),"session_id":hint.session.id(),
                                "from_participant":hint.from_participant,"to_participant":hint.to_participant,
                                "operation_id":hint.operation_id,"source":{"kind":hint.source.kind,"correlation_id":hint.source.correlation_id},
                                "medium":medium,"intent":hint.intent,"capabilities":hint.capabilities_offer
                            });
                            if serde_json::to_vec(&value)
                                .map_err(|_| failure(ErrorKind::Protocol))?
                                .len()
                                > 63 * 1024
                            {
                                return Err(failure(ErrorKind::Protocol));
                            }
                            let id = format!("invitation-{}", uuid::Uuid::new_v4())
                                .parse()
                                .unwrap();
                            state.delivery.pending = Some((id, value));
                        }
                    }
                }
                Ok(match &state.delivery.pending {
                    Some((id, value)) => {
                        json!({"reader_id":reader_id,"delivery_id":id,"invitation":value})
                    }
                    None => json!({"reader_id":reader_id,"delivery_id":null,"invitation":null}),
                })
            }
            Action::IncomingClose {
                reader_id,
                acknowledged,
            } => {
                let mut state = tokio::time::timeout_at(until, self.incoming.lock())
                    .await
                    .map_err(|_| failure(ErrorKind::Deadline))?;
                state.delivery.claim(&reader_id, acknowledged.as_ref())?;
                if let Some(mut stream) = state.stream.take() {
                    stream.close().await?;
                }
                state.delivery.detach();
                Ok(json!({"reader_id":reader_id,"closed":true}))
            }
            action => {
                super::execute(
                    &self.client,
                    Request {
                        timeout_ms: request.timeout_ms,
                        action,
                    },
                )
                .await
            }
        }
    }
    pub async fn close(self, options: RequestOptions) -> Result<()> {
        let mut incoming = self.incoming.into_inner();
        if let Some(mut stream) = incoming.stream.take() {
            stream.close().await?;
        }
        self.client.close(options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(value: &str) -> OperationId {
        value.parse().unwrap()
    }
    #[test]
    fn incoming_inbox_check_preserves_current_failure() {
        let inbox = super::super::super::invitations::Inbox::default();
        inbox.check().unwrap();
        inbox.end(ErrorKind::Authentication);
        let error = inbox.check().unwrap_err();
        assert_eq!(error.kind, ErrorKind::Authentication);
        assert_eq!(error.certainty, Certainty::ReadOnly);
    }
    #[test]
    fn pending_hint_requires_exact_owner_and_acknowledgement() {
        let mut delivery = Delivery::default();
        delivery.claim(&id("reader"), None).unwrap();
        delivery.pending = Some((id("original"), json!({"operation_id":"invite"})));
        assert!(delivery
            .claim(&id("foreign"), Some(&id("original")))
            .is_err());
        assert!(delivery.claim(&id("reader"), Some(&id("foreign"))).is_err());
        assert!(delivery.pending.is_some());
        delivery.claim(&id("reader"), None).unwrap();
        assert!(delivery.pending.is_some());
        delivery
            .claim(&id("reader"), Some(&id("original")))
            .unwrap();
        assert!(delivery.pending.is_none());
        delivery
            .claim(&id("reader"), Some(&id("original")))
            .unwrap();
    }
    #[test]
    fn detach_preserves_unacknowledged_hint_but_no_old_reader_authority() {
        let mut delivery = Delivery::default();
        delivery.claim(&id("reader"), None).unwrap();
        delivery.pending = Some((id("original"), json!({"operation_id":"invite"})));
        delivery.detach();
        assert!(delivery.claim(&id("new"), Some(&id("original"))).is_err());
        delivery.claim(&id("new"), None).unwrap();
        assert!(delivery.pending.is_some());
        assert!(delivery.claim(&id("reader"), None).is_err());
        delivery.claim(&id("new"), Some(&id("original"))).unwrap();
        assert!(delivery.pending.is_none());
    }
}
