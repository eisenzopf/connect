//! Remote acceptance without a receipt remains uncertain across owned restarts.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Vapi;
use connect_protocol::{Operation, Outcome};
use connect_server::providers::Worker;
use connect_store::{Route, Scope};
use native_tasks::Harness;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};

/// The fake remote service commits its business effect before withholding ACK.
/// Counting accepted effects is independent of Connect's delivery/job records.
struct AcceptWithoutReceipt(Arc<AtomicUsize>);
impl Respond for AcceptWithoutReceipt {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let input: serde_json::Value = request.body_json().unwrap();
        assert_eq!(input["input"], "Accept once, even if the receipt is lost.");
        self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200)
            .set_body_json(json!({"id":"remote-chat-one","output":[{"content":"Accepted"}]}))
            .set_delay(Duration::from_secs(60))
    }
}
async fn uncertain(client: &mut ConnectClient, question: TaskCommand) -> String {
    let TaskOutcome::Tool(TaskToolResult::Uncertain { effect_id, .. }) =
        client.task(question).await.unwrap()
    else {
        panic!("uncertain outcome")
    };
    effect_id
}

#[tokio::test(flavor = "multi_thread")]
async fn accepted_task_send_without_receipt_is_not_redispatched_after_server_restart() {
    let mock = MockServer::start().await;
    let accepted = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/assistant/12345678-1234-1234-1234-123456789012"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":"12345678-1234-1234-1234-123456789012","model":{"messages":[]}}),
        ))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(AcceptWithoutReceipt(accepted.clone()))
        .expect(1)
        .mount(&mock)
        .await;
    // Complete platform HTTP initialization before opening bounded native peers.
    let driver = Arc::new(Vapi::new("offline-fixture", &mock.uri()).unwrap());
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let delegate = h
        .server
        .store
        .issue(
            &h.peer_id,
            Scope::messaging(vec![h.owner_config.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    h.server
        .store
        .configure_route(
            &h.owner_config.credential_id,
            &Route {
                id: "uncertain-vapi".into(),
                identity_id: h.peer_id.clone(),
                credential_id: delegate.id,
                conversation_id: h.cid.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    let task = h.create("task").await;
    let grant = grants::issue(
        &h,
        &task,
        "uncertain-grant",
        vec![TaskPermission::Send],
        chrono::Utc::now().timestamp() + 300,
    )
    .await;
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let worker = Worker::with_drivers(h.server.service.clone(), vec![driver.clone()]).unwrap();
    let question = grants::wrapped(
        &task.id,
        "retained-mutation",
        Operation::Send {
            request_id: "retained-mutation".into(),
            conversation_id: h.cid.clone(),
            body: "Accept once, even if the receipt is lost.".into(),
        },
    );
    assert!(matches!(
        client.task(question.clone()).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Pending { .. } | TaskToolResult::Uncertain { .. })
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while accepted.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    worker.shutdown().await.unwrap();
    let effect_id = uncertain(&mut client, question.clone()).await;
    client.close().await;
    h = h.restart().await;
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let worker = Worker::with_drivers(h.server.service.clone(), vec![driver]).unwrap();
    assert_eq!(uncertain(&mut client, question.clone()).await, effect_id);
    // Multiple worker polls and matching public replays cannot dispatch again.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(uncertain(&mut client, question).await, effect_id);
    let jobs = h.owner.provider_status(&h.cid).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, "uncertain");
    let inspection = TaskCommand::EffectGet {
        task_id: task.id.clone(),
        effect_id,
    };
    // A send-only grant may reconcile its send, but cannot gain receipt-read
    // permission. Inspect retained metadata with the separately authorized owner.
    assert_eq!(
        grants::code(client.task(inspection.clone()).await.unwrap_err()),
        "forbidden"
    );
    h.owner.negotiate_task(&task.id).await.unwrap();
    let TaskOutcome::Effect(effect) = h.owner.task(inspection).await.unwrap() else {
        panic!("effect")
    };
    assert_eq!(effect.state, EffectState::Uncertain);
    let Outcome::Accepted {
        conversation_id, ..
    } = *effect.outcome.unwrap()
    else {
        panic!("local send receipt")
    };
    assert_eq!(conversation_id, h.cid);
    assert_eq!(
        h.owner
            .history(&h.cid, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    worker.shutdown().await.unwrap();
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
    client.close().await;
    h.close().await;
}
