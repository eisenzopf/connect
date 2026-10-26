//! A real app crash and lost provider ACK keep one retained uncertain effect.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used)]
#[path = "support/task_fixture.rs"]
mod fixture;
use connect_client::{tasks::*, Outcome};
use connect_connectors::Vapi;
use connect_server::providers::Worker;
use connect_store::Route;
use fixture::{exited, stop, Fixture};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::{
    path::Path,
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

/// Independent provider state records acceptance before withholding its receipt.
struct AcceptWithoutReceipt(Arc<AtomicUsize>);
impl Respond for AcceptWithoutReceipt {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let input: serde_json::Value = request.body_json().unwrap();
        assert_eq!(
            input["input"],
            "Please review draft-002 and reply to this question."
        );
        self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200)
            .set_body_json(json!({"id":"accepted-once","output":[{"content":"Accepted"}]}))
            .set_delay(Duration::from_secs(60))
    }
}

/// This is the application's own outbox, never the Connect server database.
fn saved_question(state: &Path) -> (TaskCommand, Option<TaskOutcome>) {
    let db = Connection::open_with_flags(state, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (command, outcome): (String, Option<String>) = db
        .query_row(
            "SELECT command,outcome FROM runner_commands WHERE key='question'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    (
        serde_json::from_str(&command).unwrap(),
        outcome.map(|s| serde_json::from_str(&s).unwrap()),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn app_and_server_restarts_retain_actual_provider_uncertainty_without_redispatch() {
    let remote = MockServer::start().await;
    let accepted = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/assistant/12345678-1234-1234-1234-123456789012"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":"12345678-1234-1234-1234-123456789012","model":{"messages":[{"role":"system","content":"{{connect_context}}"}]}}),
        ))
        .expect(1)
        .mount(&remote)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(AcceptWithoutReceipt(accepted.clone()))
        .expect(1)
        .mount(&remote)
        .await;
    let driver = Arc::new(Vapi::new("offline-fixture", &remote.uri()).unwrap());
    let mut f = Fixture::start().await;
    f.server
        .as_ref()
        .unwrap()
        .store
        .configure_route(
            &f.owner_config.credential_id,
            &Route {
                id: "runner-uncertain-vapi".into(),
                identity_id: f.peer_config.identity_id.clone(),
                credential_id: f.peer_config.credential_id.clone(),
                conversation_id: f.task.conversation_id.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let provider = Worker::with_drivers(
        f.server.as_ref().unwrap().service.clone(),
        vec![driver.clone()],
    )
    .await
    .unwrap();
    let mut app = f.spawn(Some("after-invocation-ack"), None);
    exited(&mut app, 86).await;
    let original = saved_question(&f.state());
    assert!(
        original.1.is_none(),
        "Crash must precede app receipt commit"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while accepted.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    provider.shutdown().await.unwrap();

    // Neither the old grant nor the durable app state is replaced on restart.
    f.restart().await;
    let provider = Worker::with_drivers(f.server.as_ref().unwrap().service.clone(), vec![driver])
        .await
        .unwrap();
    app = f.spawn(None, None);
    let stats = f.settled(0).await;
    assert_eq!(stats.task_id, f.task.id);
    stop(&mut app).await;
    let recovered = saved_question(&f.state());
    assert_eq!(recovered.0, original.0);
    let Some(TaskOutcome::Tool(TaskToolResult::Uncertain { effect_id, .. })) = &recovered.1 else {
        panic!("Actual provider uncertainty must remain distinct from completion");
    };
    let TaskOutcome::Effect(effect) = f
        .owner
        .as_mut()
        .unwrap()
        .task(TaskCommand::EffectGet {
            task_id: f.task.id.clone(),
            effect_id: effect_id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("effect")
    };
    assert_eq!(effect.state, EffectState::Uncertain);
    assert_eq!(effect.task_id, f.task.id);
    let TaskCommand::Invoke { request_id, .. } = &original.0 else {
        panic!("Persisted invocation")
    };
    assert_eq!(&effect.request_id, request_id);
    let Some(outcome) = effect.outcome else {
        panic!("Local send receipt")
    };
    let Outcome::Accepted {
        conversation_id, ..
    } = *outcome
    else {
        panic!("Local send receipt is distinct from remote completion")
    };
    assert_eq!(conversation_id, f.task.conversation_id);

    // Public diagnostic journal retains the actual uncertainty and original
    // reconciliation identities without reading outcomes or replaying work.
    let snapshot =
        connect_store::Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap();
    let diagnostic = connect_review_loop::task_runner::diagnostics::inspect(
        &f.diagnostic_config("invocation"),
        &f.task.id,
        Some(&f.state()),
        100,
    )
    .await
    .unwrap();
    let connect_client::task_diagnostics::DiagnosticSection::Available(journal) =
        diagnostic.task.journal
    else {
        panic!("diagnostic journal");
    };
    assert!(journal.items.iter().any(|item| matches!(&item.observation, connect_client::task_diagnostics::DiagnosticObservation::Effect { effect_id: id, request_id: request, state: EffectState::Uncertain, .. } if id == effect_id && request == request_id)));
    assert_eq!(saved_question(&f.state()), recovered);
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap(),
        snapshot
    );

    // A second app restart catches up the same journal without treating the
    // uncertain provider result as a review response or a reason to send again.
    app = f.spawn(None, None);
    f.settled(0).await;
    stop(&mut app).await;
    assert_eq!(saved_question(&f.state()), recovered);
    let jobs = f
        .owner
        .as_mut()
        .unwrap()
        .provider_status(&f.task.conversation_id)
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, "uncertain");
    let history = f
        .peer
        .as_mut()
        .unwrap()
        .history(&f.task.conversation_id, None, 100)
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].body,
        "Please review draft-002 and reply to this question."
    );
    provider.shutdown().await.unwrap();
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
    f.finish().await;
}
