//! Drive the exact exchange with scripted, already-verified public Envelopes.
//! Signature/authentication stays covered by the retained real signed SDK tests;
//! these controls prove original command/ACK/read ownership, not authentication.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::sync::mpsc;

type Sent = (String, String, Value, CommandScope);
struct Scripted {
    requests: mpsc::UnboundedSender<Sent>,
    responses: mpsc::UnboundedReceiver<Envelope>,
    next: u64,
}
impl Peer for Scripted {
    async fn send(&mut self, kind: &str, value: Value, scope: CommandScope) -> Result<String> {
        self.next += 1;
        let id = format!("request_{}", self.next);
        self.requests
            .send((id.clone(), kind.into(), value, scope))?;
        Ok(id)
    }
    async fn receive(&mut self) -> Result<Envelope> {
        self.responses
            .recv()
            .await
            .ok_or_else(|| "scripted original peer ended".into())
    }
    async fn answer(
        &mut self,
        _: &Envelope,
        _: payloads::ConnectionAnswer,
        _: &CommandScope,
    ) -> Result<()> {
        Err("Data gate must not renegotiate".into())
    }
}
fn pair() -> (
    Scripted,
    mpsc::UnboundedReceiver<Sent>,
    mpsc::UnboundedSender<Envelope>,
) {
    let (requests, read) = mpsc::unbounded_channel();
    let (write, responses) = mpsc::unbounded_channel();
    (
        Scripted {
            requests,
            responses,
            next: 0,
        },
        read,
        write,
    )
}
fn scope() -> CommandScope {
    CommandScope {
        conversation_id: Some("conversation_one".into()),
        session_id: Some("session_one".into()),
        connection_id: Some("connection_one".into()),
        operation_id: Some("operation_one".into()),
        expected_revision: Some(4),
    }
}
fn update(state: &str, revision: u64) -> Value {
    let mut value = json!({"operation_id":"operation_one","record_id":"record_one",
        "command_type":"connection.update","expected_revision":4,"state":state,"revision":revision});
    if state == "completed" {
        value["result"] = json!({"original-result":true});
    }
    if matches!(state, "failed" | "unknown") {
        value["problem"] = json!({"code":"original-refused"});
    }
    value
}
fn frame(sent: &Sent, kind: &str, payload: Value) -> Envelope {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut value = json!({"v":2,"type":kind,"id":format!("response_{}",NEXT.fetch_add(1,Ordering::Relaxed)),
        "ts":"2026-11-10T00:00:00Z","in_reply_to":sent.0,"cid":sent.3.conversation_id,
        "sid":sent.3.session_id,"connid":sent.3.connection_id,"payload":payload});
    if let Some(operation) = &sent.3.operation_id {
        value["operation_id"] = json!(operation);
    }
    Envelope::from_value(value).unwrap()
}
fn validate_reads(sent: &[Sent]) {
    assert_eq!(
        sent.iter().filter(|r| r.1 == "connection.update").count(),
        1
    );
    assert_eq!(sent[0].2, json!({"action":"unmute","streams":["data_one"]}));
    assert_eq!(sent[0].3.operation_id.as_deref(), Some("operation_one"));
    assert_eq!(sent[0].3.expected_revision, Some(4));
    for row in &sent[1..] {
        assert_eq!(row.1, "operation.get");
        assert_eq!(row.2, json!({"operation_id":"operation_one"}));
        assert!(row.3.operation_id.is_none() && row.3.expected_revision.is_none());
        assert_eq!(row.3.conversation_id.as_deref(), Some("conversation_one"));
        assert_eq!(row.3.session_id.as_deref(), Some("session_one"));
        assert_eq!(row.3.connection_id.as_deref(), Some("connection_one"));
    }
    let ids = sent
        .iter()
        .map(|r| &r.0)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        ids.len(),
        sent.len(),
        "fresh read envelopes must not change the logical mutation ID"
    );
}
async fn run_peer(
    mut peer: Scripted,
    until: tokio::time::Instant,
) -> (
    Result<(Value, Option<Vec<selected::Binding>>)>,
    Option<Value>,
) {
    exchange(
        &mut peer,
        &scope(),
        "connection.update",
        json!({"action":"unmute","streams":["data_one"]}),
        &json!({}),
        &[],
        None,
        until,
    )
    .await
}

#[tokio::test]
async fn pending_control_drains_both_acks_and_observes_same_original_once() {
    let (peer, mut requests, responses) = pair();
    let finished = Arc::new(AtomicBool::new(false));
    let done = finished.clone();
    let client = async move {
        let result = run_peer(peer, tokio::time::Instant::now() + Duration::from_secs(1)).await;
        done.store(true, Ordering::Release);
        result
    };
    let server = tokio::time::timeout(Duration::from_secs(2), async move {
        let original = requests.recv().await.ok_or("original mutation missing")?;
        responses.send(frame(
            &original,
            "operation.updated",
            update("preparing", 2),
        ))?;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let premature_read = requests.try_recv().ok();
        let early_first = finished.load(Ordering::Acquire);
        responses.send(frame(&original, "ack", json!({})))?;
        let read = requests
            .recv()
            .await
            .ok_or("same original status read missing")?;
        responses.send(frame(&read, "operation.updated", update("completed", 3)))?;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let early_final = finished.load(Ordering::Acquire);
        responses.send(frame(&read, "ack", json!({})))?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
            vec![original, read],
            premature_read,
            early_first,
            early_final,
        ))
    });
    let ((result, observed), script) = tokio::join!(client, server);
    let (sent, premature, early_first, early_final) = script.unwrap().unwrap();
    validate_reads(&sent);
    assert!(
        premature.is_none() && !early_first && !early_final,
        "a pending or completed receipt cannot replace its original final ACK"
    );
    let (value, incoming) = result.unwrap();
    assert_eq!(value, update("completed", 3));
    assert!(incoming.is_none());
    assert_eq!(observed, Some(value));
}

#[tokio::test]
async fn pending_control_preserves_failed_or_unknown_original_without_resubmission() {
    for state in ["failed", "unknown"] {
        let (peer, mut requests, responses) = pair();
        let client = run_peer(peer, tokio::time::Instant::now() + Duration::from_secs(1));
        let server = tokio::time::timeout(Duration::from_secs(2), async move {
            let original = requests.recv().await.ok_or("original mutation missing")?;
            responses.send(frame(
                &original,
                "operation.updated",
                update("preparing", 2),
            ))?;
            responses.send(frame(&original, "ack", json!({})))?;
            let read = requests.recv().await.ok_or("same original read missing")?;
            responses.send(frame(&read, "operation.updated", update(state, 3)))?;
            responses.send(frame(&read, "ack", json!({})))?;
            let next = requests.recv().await;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((vec![original, read], next))
        });
        let ((result, observed), script) = tokio::join!(client, server);
        let (sent, next) = script.unwrap().unwrap();
        validate_reads(&sent);
        assert!(
            next.is_none(),
            "terminal original must never create another command"
        );
        assert!(result.is_err());
        assert_eq!(observed, Some(update(state, 3)));
    }
}

#[tokio::test]
async fn pending_status_reads_keep_original_cutoff_and_do_not_publish_or_replay() {
    let (peer, mut requests, responses) = pair();
    let started = tokio::time::Instant::now();
    let until = started + Duration::from_millis(100);
    let client = run_peer(peer, until);
    let server = tokio::time::timeout(Duration::from_secs(1), async move {
        let mut sent = vec![];
        while let Some(request) = requests.recv().await {
            let updated = frame(&request, "operation.updated", update("preparing", 2));
            let ack = frame(&request, "ack", json!({}));
            sent.push(request);
            if responses.send(updated).is_err() || responses.send(ack).is_err() {
                break; // The original cutoff may close the peer at this boundary.
            }
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(sent)
    });
    let ((result, observed), script) = tokio::join!(client, server);
    let sent = script.unwrap().unwrap();
    validate_reads(&sent);
    assert!(
        sent.len() >= 2,
        "must actually observe the original pending record again"
    );
    assert!(result.unwrap_err().to_string().contains("deadline"));
    assert_eq!(observed, Some(update("preparing", 2)));
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "status reads must not renew the cap"
    );
}

#[tokio::test]
async fn status_read_cannot_substitute_record_precondition_or_revision() {
    let mut cases = vec![];
    for (field, value) in [
        ("record_id", json!("other_record")),
        ("operation_id", json!("other_operation")),
        ("command_type", json!("stream.subscribe")),
        ("expected_revision", json!(5)),
        ("revision", json!(1)),
    ] {
        let mut changed = update("completed", 3);
        changed[field] = value;
        cases.push(changed);
    }
    cases.push(update("completed", 2)); // Same revision cannot change state/result.
    for changed in cases {
        let (peer, mut requests, responses) = pair();
        let client = run_peer(peer, tokio::time::Instant::now() + Duration::from_secs(1));
        let server = tokio::time::timeout(Duration::from_secs(2), async move {
            let original = requests.recv().await.ok_or("original mutation missing")?;
            responses.send(frame(
                &original,
                "operation.updated",
                update("preparing", 2),
            ))?;
            responses.send(frame(&original, "ack", json!({})))?;
            let read = requests.recv().await.ok_or("same original read missing")?;
            responses.send(frame(&read, "operation.updated", changed))?;
            let _ = responses.send(frame(&read, "ack", json!({})));
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(vec![original, read])
        });
        let ((result, observed), script) = tokio::join!(client, server);
        validate_reads(&script.unwrap().unwrap());
        assert!(result.is_err());
        assert_eq!(observed, Some(update("preparing", 2)));
    }
}
