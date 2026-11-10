//! One original control, then bounded current observations of that SAME record.
//! The adapter uses only public client APIs; the private seam lets tests drive
//! this exact post-verification exchange without another mutation or media IO.
use super::*;

trait Peer {
    async fn send(&mut self, kind: &str, value: Value, scope: CommandScope) -> Result<String>;
    async fn receive(&mut self) -> Result<Envelope>;
    async fn answer(
        &mut self,
        request: &Envelope,
        answer: payloads::ConnectionAnswer,
        scope: &CommandScope,
    ) -> Result<()>;
}
impl Peer for CanonicalClient {
    async fn send(&mut self, kind: &str, value: Value, scope: CommandScope) -> Result<String> {
        Ok(self.send_value(kind, value, scope).await?)
    }
    async fn receive(&mut self) -> Result<Envelope> {
        Ok(CanonicalClient::receive(self).await?)
    }
    async fn answer(
        &mut self,
        request: &Envelope,
        answer: payloads::ConnectionAnswer,
        scope: &CommandScope,
    ) -> Result<()> {
        self.answer_connection_offer(request, answer, scope).await?;
        Ok(())
    }
}

fn resource_scope(frame: &Envelope, original: &CommandScope, reading: bool) -> bool {
    [
        (Scope::Conversation, original.conversation_id.as_deref()),
        (Scope::Session, original.session_id.as_deref()),
        (Scope::Connection, original.connection_id.as_deref()),
    ]
    .iter()
    .all(|(key, expected)| expected.is_some() && frame.scope(*key) == *expected)
        && frame.scope(Scope::Operation)
            == if reading {
                None
            } else {
                original.operation_id.as_deref()
            }
}

#[derive(Default)]
struct Receipt {
    last: Option<Value>,
}
impl Receipt {
    fn observe(&mut self, value: &Value, scope: &CommandScope, kind: &str) -> Result<()> {
        let revision = value["revision"].as_u64().filter(|r| *r > 0);
        if value["operation_id"].as_str() != scope.operation_id.as_deref()
            || value["command_type"] != kind
            || value["expected_revision"].as_u64() != scope.expected_revision
            || value["record_id"].as_str().is_none_or(|id| !video::id(id))
            || revision.is_none()
        {
            return Err("original control receipt changed its operation identity".into());
        }
        if let Some(previous) = &self.last {
            if previous["record_id"] != value["record_id"]
                || revision < previous["revision"].as_u64()
                || (revision == previous["revision"].as_u64() && previous != value)
            {
                return Err("original control receipt changed its record or revision".into());
            }
        }
        self.last = Some(value.clone());
        Ok(())
    }
}

pub(super) async fn run(
    client: &mut CanonicalClient,
    scope: &CommandScope,
    kind: &str,
    input: Value,
    capabilities: &Value,
    own: &[selected::Binding],
    incoming: Option<&video::Video>,
    until: tokio::time::Instant,
) -> (
    Result<(Value, Option<Vec<selected::Binding>>)>,
    Option<Value>,
) {
    exchange(
        client,
        scope,
        kind,
        input,
        capabilities,
        own,
        incoming,
        until,
    )
    .await
}

async fn exchange(
    peer: &mut impl Peer,
    scope: &CommandScope,
    kind: &str,
    input: Value,
    capabilities: &Value,
    own: &[selected::Binding],
    incoming: Option<&video::Video>,
    until: tokio::time::Instant,
) -> (
    Result<(Value, Option<Vec<selected::Binding>>)>,
    Option<Value>,
) {
    let mut receipt = Receipt::default();
    let work = async {
        let operation = scope
            .operation_id
            .as_deref()
            .ok_or("original control operation missing")?;
        let mut request = peer.send(kind, input, scope.clone()).await?;
        let mut reading = false;
        let mut received = None;
        loop {
            let mut seen = false;
            let mut next_read = false;
            for _ in 0..128 {
                let frame = peer.receive().await?;
                if frame.scope(Scope::Reply).is_none()
                    && matches!(
                        frame.kind(),
                        "operation.updated"
                            | "connection.ready"
                            | "stream.opened"
                            | "stream.closed"
                            | "connection.quality"
                            | "session.started"
                            | "session.ended"
                            | "session.participant.joined"
                            | "session.participant.left"
                            | "recording.vcon-ready"
                    )
                {
                    continue;
                }
                if frame.kind() == "connection.offer"
                    && !reading
                    && incoming.is_some()
                    && received.is_none()
                    && scope_matches(&frame, scope)
                {
                    let payload = &frame.original()["payload"];
                    let expected = incoming.unwrap();
                    let bindings = selected::subscription(
                        payload,
                        own,
                        &expected.receive_video_stream_id,
                        expected.receive_data_stream_id.as_deref(),
                    )?;
                    peer.answer(&frame, selected::answer(payload, capabilities)?, scope)
                        .await?;
                    received = Some(bindings);
                    continue;
                }
                if frame.scope(Scope::Reply) != Some(request.as_str())
                    || !resource_scope(&frame, scope, reading)
                {
                    return Err(
                        "control response changed its original request or resource scope".into(),
                    );
                }
                match frame.kind() {
                    "operation.updated" if !seen => {
                        receipt.observe(&frame.original()["payload"], scope, kind)?;
                        seen = true;
                    }
                    "ack" if seen => {
                        let value = receipt
                            .last
                            .as_ref()
                            .ok_or("original control receipt missing")?;
                        match value["state"].as_str() {
                            Some("completed") => {
                                if incoming.is_some() && received.is_none() {
                                    return Err(
                                        "video subscription omitted actual negotiation".into()
                                    );
                                }
                                return Ok((value.clone(), received));
                            }
                            Some("accepted" | "preparing" | "ready" | "committing") => {
                                // Drain this actual ACK before a new read. The original
                                // mutation and negotiation are NEVER repeated. This sleep,
                                // send and response share the SAME original cutoff.
                                tokio::time::sleep(Duration::from_millis(20)).await;
                                let mut read_scope = scope.clone();
                                read_scope.operation_id = None;
                                read_scope.expected_revision = None;
                                request = peer
                                    .send(
                                        "operation.get",
                                        json!({"operation_id":operation}),
                                        read_scope,
                                    )
                                    .await?;
                                reading = true;
                                next_read = true;
                                break;
                            }
                            Some("failed" | "cancelled" | "interrupted" | "unknown") => {
                                return Err(
                                    "original control has a terminal noncompleted receipt".into()
                                );
                            }
                            _ => {
                                return Err("original control returned an unsupported state".into())
                            }
                        }
                    }
                    _ => return Err("unexpected control response; original is not retried".into()),
                }
            }
            // A pending ACK starts another bounded response. Exhausting a batch
            // without it is a protocol failure, not permission to send a read.
            if !next_read {
                return Err("original control response frame bound exceeded".into());
            }
        }
    };
    let result = match tokio::time::timeout_at(until, work).await {
        Ok(result) => result,
        Err(_) => Err("original control settlement deadline ended".into()),
    };
    (result, receipt.last)
}

#[cfg(test)]
mod tests;
