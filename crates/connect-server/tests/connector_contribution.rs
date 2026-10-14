//! Independently authored Prism fixture integrated after the host-schema guide repair.
//! Uses public SDK/composition only; no conversation/routing/authorization edits.
#![allow(clippy::unwrap_used)]
use connect_client::{ClientConfig, ConnectClient};
use connect_connector_api::{
    conformance::{Fixture, Injection},
    Connector,
};
use connect_protocol::Operation;
use connect_server::{cli::init, config::ServerConfig, providers::Worker, runtime::RunningServer};
use connect_store::Route;
use prism_fixture::Prism;
use std::{sync::Arc, time::Duration};

async fn fixture() -> (
    tempfile::TempDir,
    RunningServer,
    ConnectClient,
    String,
    Arc<Prism>,
) {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let agent = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let operator = ClientConfig::load(&state.join("operator.toml")).unwrap();
    let mut client = ConnectClient::open(config).await.unwrap();
    let conversation = client
        .create("conversation", vec![agent.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let provider = Arc::new(Prism::open_file(tmp.path().join("provider.json")).unwrap());
    server
        .store
        .register_connector(provider.descriptor())
        .unwrap();
    server
        .store
        .configure_route(
            &operator.credential_id,
            &Route {
                id: "contribution".into(),
                identity_id: agent.identity_id,
                credential_id: agent.credential_id,
                conversation_id: conversation.clone(),
                connector: provider.name().into(),
                target: "fixture-peer".into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    (tmp, server, client, conversation, provider)
}
async fn delivery(client: &mut ConnectClient, conversation: &str, expected: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = client
                .context_deliveries(conversation, None, 10)
                .await
                .unwrap();
            if let Some(item) = page
                .deliveries
                .iter()
                .find(|d| d.provider == "prism-fixture" && d.state == expected)
            {
                return item.id.strip_prefix("provider-job:").unwrap().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn independent_prism_context_and_contact_survive_host_and_adapter_restart() {
    let (tmp, server, mut client, cid, provider) = fixture().await;
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let send = Operation::Send {
        request_id: "contributor-first".into(),
        conversation_id: cid.clone(),
        body: "The approved color is violet.".into(),
    };
    let receipt = client.request(send.clone()).await.unwrap();
    let id = delivery(&mut client, &cid, "delivered").await;
    assert_eq!(provider.effects(&id), 1);
    assert!(client
        .history(&cid, None, 10)
        .await
        .unwrap()
        .messages
        .iter()
        .any(|m| m.body.contains("Historical context:") && m.body.contains("violet")));
    assert_eq!(client.request(send.clone()).await.unwrap(), receipt);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
    let state = tmp.path().join("state");
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let provider = Arc::new(Prism::open_file(tmp.path().join("provider.json")).unwrap());
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut client = ConnectClient::open(config).await.unwrap();
    assert_eq!(client.request(send).await.unwrap(), receipt);
    assert_eq!(provider.effects(&id), 1);
    // The remote peer independently initiates a new message on the same route.
    // Ingress authenticates the adapter response; the host maps the existing
    // route credential and journals it without invoking outbound send.
    let contact = provider
        .independent_message("remote-contact", "Independently proposed color is amber.")
        .unwrap();
    let incoming_id = Worker::receive(
        server.service.as_ref(),
        provider.as_ref(),
        "contribution",
        &contact,
    )
    .unwrap();
    assert_eq!(
        Worker::receive(
            server.service.as_ref(),
            provider.as_ref(),
            "contribution",
            &contact
        )
        .unwrap(),
        incoming_id
    );
    assert_eq!(provider.effects("remote-contact"), 1);
    let history = client.history(&cid, None, 100).await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.id == incoming_id && m.body.contains("amber"))
            .count(),
        1
    );
    // A fresh local request exports that inbound source as bounded history.
    client
        .send(
            "contact-follow-up",
            &cid,
            "What did the independent peer propose?",
        )
        .await
        .unwrap();
    let follow_up = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let history = client.history(&cid, None, 100).await.unwrap();
            if history
                .messages
                .iter()
                .any(|m| m.body.contains("amber") && m.body.contains("Historical context:"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(follow_up.is_ok());
    let deliveries = client.context_deliveries(&cid, None, 100).await.unwrap();
    let next = deliveries
        .deliveries
        .iter()
        .find_map(|delivery| {
            let job = delivery.id.strip_prefix("provider-job:")?;
            (job != id && delivery.state == "delivered").then_some(job)
        })
        .expect("new context reply committed");
    let attributed = server
        .store
        .connector_receipt(next, provider.name())
        .unwrap();
    assert!(attributed
        .reply_sources
        .iter()
        .any(|source| source.ends_with(&format!(":{incoming_id}"))));
    let mut config = ClientConfig::load(&state.join("agent.toml")).unwrap();
    config.endpoint = server.address;
    let mut app = ConnectClient::open(config).await.unwrap();
    let incoming = app.inbox(None, 100, 0).await.unwrap();
    assert!(incoming.events.iter().any(|event| event
        .message
        .as_ref()
        .is_some_and(|m| m.body.contains("amber"))));
    worker.shutdown().await.unwrap();
    drop(client);
    drop(app);
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn unknown_effect_is_not_repeated_after_worker_and_adapter_restart() {
    let (tmp, server, mut client, cid, provider) = fixture().await;
    provider.inject(Injection::UncertainMessage);
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let send = Operation::Send {
        request_id: "contributor-unknown".into(),
        conversation_id: cid.clone(),
        body: "Uncertain provider response".into(),
    };
    let receipt = client.request(send.clone()).await.unwrap();
    let id = delivery(&mut client, &cid, "uncertain").await;
    assert_eq!(provider.effects(&id), 1);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
    let state = tmp.path().join("state");
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let provider = Arc::new(Prism::open_file(tmp.path().join("provider.json")).unwrap());
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut client = ConnectClient::open(config).await.unwrap();
    assert_eq!(client.request(send).await.unwrap(), receipt);
    assert_eq!(provider.effects(&id), 1);
    assert_eq!(delivery(&mut client, &cid, "uncertain").await, id);
    // An authoritative read under the original durable job ID resolves acceptance
    // and commits the saved reply/provenance without another provider send.
    assert_eq!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &id)
            .await
            .unwrap(),
        connect_connector_api::lifecycle::Apply::Applied
    );
    assert_eq!(delivery(&mut client, &cid, "delivered").await, id);
    assert_eq!(provider.effects(&id), 1);
    let receipt = server
        .store
        .connector_receipt(&id, provider.name())
        .unwrap();
    assert_eq!(
        receipt.observation.verification,
        connect_connector_api::lifecycle::Verification::Reconciliation
    );
    assert!(!receipt.reply_sources.is_empty());
    assert_eq!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &id)
            .await
            .unwrap(),
        connect_connector_api::lifecycle::Apply::Duplicate
    );
    assert_eq!(provider.effects(&id), 1);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn duplicate_composition_is_rejected_before_worker_claims() {
    let (_tmp, server, client, _cid, provider) = fixture().await;
    assert!(
        Worker::with_drivers(server.service.clone(), vec![provider.clone(), provider]).is_err()
    );
    drop(client);
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn invalid_or_unready_composition_cannot_persist_contracts_or_claim_work() {
    use async_trait::async_trait;
    use connect_connector_api::{
        manifest::{Manifest, Readiness},
        Completion, DeliveryRequest, Failure,
    };
    struct MissingSecret;
    #[async_trait]
    impl Connector for MissingSecret {
        fn name(&self) -> &'static str {
            "missing-secret"
        }
        fn readiness(&self) -> Readiness {
            Readiness::MissingSecret
        }
        async fn deliver(&self, _: &DeliveryRequest) -> Result<Completion, Failure> {
            panic!("must not dispatch");
        }
    }
    struct Unsupported;
    #[async_trait]
    impl Connector for Unsupported {
        fn name(&self) -> &'static str {
            "unsupported"
        }
        fn manifest(&self) -> Manifest {
            let mut value = Manifest::messaging(self.descriptor());
            value.video = true;
            value
        }
        async fn deliver(&self, _: &DeliveryRequest) -> Result<Completion, Failure> {
            panic!("must not dispatch");
        }
    }
    let (tmp, server, mut client, cid, provider) = fixture().await;
    client
        .send("before-claim", &cid, "Pending work")
        .await
        .unwrap();
    assert!(Worker::with_drivers(
        server.service.clone(),
        vec![provider.clone(), Arc::new(MissingSecret)]
    )
    .is_err());
    assert!(Worker::with_drivers(
        server.service.clone(),
        vec![provider, Arc::new(Unsupported)]
    )
    .is_err());
    let db = rusqlite::Connection::open(tmp.path().join("state/state.sqlite3")).unwrap();
    let manifests: u32 = db
        .query_row("SELECT COUNT(*) FROM connector_manifests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(manifests, 0);
    let claimed: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM provider_jobs WHERE state!='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(claimed, 0);
    drop(client);
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn independent_prism_accepts_actual_task_recall_projection_through_public_sdk() {
    use connect_client::tasks::*;
    use connect_store::Scope;
    let (tmp, server, client, cid, provider) = fixture().await;
    drop(client);
    let mut config = ClientConfig::load(&tmp.path().join("state/reviews.toml")).unwrap();
    let credential = server
        .store
        .issue(
            &config.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    config.credential_id = credential.id;
    config.credential = credential.token;
    config.endpoint = server.address;
    let peer = ClientConfig::load(&tmp.path().join("state/agent.toml")).unwrap();
    let mut owner = ConnectClient::open(config).await.unwrap();
    owner
        .negotiate_tasks("connect:agent@local", Some(&cid))
        .await
        .unwrap();
    owner
        .send("task-history-seed", &cid, "Approved task color is violet.")
        .await
        .unwrap();
    let TaskOutcome::Record(task) = owner
        .task(TaskCommand::Create {
            request_id: "prism-task".into(),
            definition: TaskDefinition {
                conversation_id: cid.clone(),
                scope: TaskScope {
                    operations: vec![
                        TaskPermission::Send,
                        TaskPermission::History,
                        TaskPermission::Inbox,
                    ],
                    recipients: vec![peer.identity_id],
                    sms_enrollments: vec![],
                },
                expires_at: chrono::Utc::now().timestamp() + 600,
                initiating_event_id: None,
            },
        })
        .await
        .unwrap()
    else {
        panic!("task created");
    };
    let question = TaskCommand::Invoke {
        request_id: "prism-task-question".into(),
        task_id: task.id.clone(),
        step_id: "ask".into(),
        operation: Box::new(Operation::Send {
            request_id: "prism-task-question".into(),
            conversation_id: cid.clone(),
            body: "What was approved?".into(),
        }),
        reply_to: None,
        wait_for_reply: true,
    };
    owner.task(question.clone()).await.unwrap();
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let TaskOutcome::Inbox { events, .. } = owner
                .task(TaskCommand::Inbox {
                    task_id: task.id.clone(),
                    after: None,
                    limit: 100,
                    wait_ms: 0,
                })
                .await
                .unwrap()
            else {
                panic!("task inbox");
            };
            if let Some(reply) = events.into_iter().find_map(|e| match e.data {
                TaskEventData::Reply(reply) => Some(reply),
                _ => None,
            }) {
                break reply;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let event = owner
        .inbox(None, 100, 0)
        .await
        .unwrap()
        .events
        .into_iter()
        .find(|event| event.id == reply.event_id)
        .expect("task reply references ordinary journal event");
    let message = event.message.expect("ordinary reply message");
    assert!(message.body.contains("violet"));
    assert!(message.body.contains("conversation-journal:"));
    assert!(message.body.contains("timestamp-not-recorded"));
    assert!(message.body.contains("Historical context:"));
    assert_eq!(
        owner.task(question.clone()).await.unwrap(),
        owner.task(question).await.unwrap()
    );
    worker.shutdown().await.unwrap();
    drop(owner);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn normalized_cancelled_task_reconciliation_retains_unassigned_reply_and_sources_once() {
    use connect_client::tasks::*;
    use connect_connector_api::lifecycle::Apply;
    use connect_store::Scope;
    let (tmp, server, client, cid, provider) = fixture().await;
    drop(client);
    let state = tmp.path().join("state");
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    let key = server
        .store
        .issue(
            &config.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    config.credential_id = key.id;
    config.credential = key.token;
    config.endpoint = server.address;
    let peer = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let mut owner = ConnectClient::open(config).await.unwrap();
    owner
        .negotiate_tasks("connect:agent@local", Some(&cid))
        .await
        .unwrap();
    let TaskOutcome::Record(task) = owner
        .task(TaskCommand::Create {
            request_id: "late-task".into(),
            definition: TaskDefinition {
                conversation_id: cid.clone(),
                scope: TaskScope {
                    operations: vec![
                        TaskPermission::Send,
                        TaskPermission::History,
                        TaskPermission::Inbox,
                    ],
                    recipients: vec![peer.identity_id],
                    sms_enrollments: vec![],
                },
                expires_at: chrono::Utc::now().timestamp() + 600,
                initiating_event_id: None,
            },
        })
        .await
        .unwrap()
    else {
        panic!("created task");
    };
    provider.inject(Injection::UncertainMessage);
    owner
        .task(TaskCommand::Invoke {
            request_id: "late-question".into(),
            task_id: task.id.clone(),
            step_id: "ask".into(),
            operation: Box::new(Operation::Send {
                request_id: "late-question".into(),
                conversation_id: cid.clone(),
                body: "The approved task color is amber.".into(),
            }),
            reply_to: None,
            wait_for_reply: true,
        })
        .await
        .unwrap();
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let job = delivery(&mut owner, &cid, "uncertain").await;
    assert_eq!(provider.effects(&job), 1);
    owner
        .task(TaskCommand::Cancel {
            request_id: "cancel-uncertain-task".into(),
            task_id: task.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    assert_eq!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &job)
            .await
            .unwrap(),
        Apply::Applied
    );
    assert_eq!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &job)
            .await
            .unwrap(),
        Apply::Duplicate
    );
    assert_eq!(provider.effects(&job), 1);
    let receipt = server
        .store
        .connector_receipt(&job, provider.name())
        .unwrap();
    assert!(!receipt.reply_sources.is_empty());
    let TaskOutcome::Replies { items, .. } = owner
        .task(TaskCommand::Replies {
            conversation_id: cid.clone(),
            after: None,
            limit: 100,
        })
        .await
        .unwrap()
    else {
        panic!("ordinary replies");
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].task_id.is_none() && items[0].reply_to.is_some());
    let TaskOutcome::Record(current) = owner
        .task(TaskCommand::Get {
            task_id: task.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("retained task");
    };
    assert_eq!(current.state, TaskState::Cancelled);
    let TaskOutcome::Inbox { events, .. } = owner
        .task(TaskCommand::Inbox {
            task_id: task.id,
            after: None,
            limit: 100,
            wait_ms: 0,
        })
        .await
        .unwrap()
    else {
        panic!("task inbox");
    };
    assert!(events
        .iter()
        .all(|event| !matches!(event.data, TaskEventData::Reply(_))));
    let db = rusqlite::Connection::open(state.join("state.sqlite3")).unwrap();
    let effects: u32 = db
        .query_row("SELECT COUNT(*) FROM task_effects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(effects, 1);
    let history = owner.history(&cid, None, 100).await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.body.contains("Historical context:") && m.body.contains("amber"))
            .count(),
        1
    );
    let operator = ClientConfig::load(&state.join("operator.toml")).unwrap();
    let authority = server.store.authenticate(&operator.credential).unwrap();
    server
        .store
        .revoke(&authority, &peer.credential_id)
        .unwrap();
    assert!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &job)
            .await
            .is_err()
    );
    assert!(server
        .store
        .apply_connector_receipt(provider.name(), &receipt)
        .is_err());
    assert_eq!(provider.effects(&job), 1);
    worker.shutdown().await.unwrap();
    drop(owner);
    server.shutdown().await.unwrap();
}
