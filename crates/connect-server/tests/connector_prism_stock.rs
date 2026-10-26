#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
//! Independent adapter through current public SDK and protected stock composition.
#[path = "support/connector_stock_setup.rs"]
mod setup;
use connect_client::{
    application::{ApplicationClient, ApplicationOperation},
    canonical::CanonicalHttpClient,
    MessageDraft,
};
use connect_connector_api::{
    conformance::{Fixture, Injection},
    lifecycle::*,
    manifest::*,
    Completion, Connector, DeliveryRequest, Descriptor, DispatchPermit, Failure,
};
use connect_protocol::{messages::MessageAudience, provisioning::*, Operation, Outcome};
use connect_server::providers::Worker;
use prism_fixture::Prism;
use setup::{delivered, http};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

/// Count the actual adapter submission entry, before provider idempotency can
/// hide an accidental resubmission. This wrapper changes no service dispatch.
struct Counted {
    inner: Arc<dyn Connector>,
    submissions: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl Connector for Counted {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn descriptor(&self) -> Descriptor {
        self.inner.descriptor()
    }
    fn manifest(&self) -> Manifest {
        self.inner.manifest()
    }
    fn configuration(&self) -> BTreeMap<String, String> {
        self.inner.configuration()
    }
    fn readiness(&self) -> Readiness {
        self.inner.readiness()
    }
    fn message_extension(&self) -> Option<&dyn MessageExtension> {
        self.inner.message_extension()
    }
    async fn deliver_observed(
        &self,
        id: &str,
        request: &DeliveryRequest,
        permit: DispatchPermit,
    ) -> Result<MessageReceipt, Failure> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        let result = self.inner.deliver_observed(id, request, permit).await;
        if let Err(error) = &result {
            eprintln!(
                "Fixture submission {id}: {error:?}; context bytes {}",
                request.context.as_ref().map_or(0, |v| v.len())
            );
        }
        result
    }
    async fn deliver(
        &self,
        request: &DeliveryRequest,
        permit: DispatchPermit,
    ) -> Result<Completion, Failure> {
        self.inner.deliver(request, permit).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prism_stock_uncertain_restart_signed_callback_and_current_replay_preserve_original_submission(
) {
    let f = setup::setup("prism-fixture", "prism-offline-key").await;
    let prism = Arc::new(Prism::open_file(f.remote.clone()).unwrap());
    prism.inject(Injection::UncertainMessage);
    let count = Arc::new(AtomicUsize::new(0));
    let driver = Arc::new(Counted {
        inner: prism.clone(),
        submissions: count.clone(),
    });
    let server = f.start(driver.clone()).await;
    let mut client = CanonicalHttpClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let mut app = ApplicationClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let draft = MessageDraft::text(
        MessageAudience::Participants(vec![f.ai_pid.clone()]),
        "prism private uncertain question",
    );
    let accepted = client
        .send_message(
            "prism-original",
            &f.cid,
            "prism-original-message",
            draft.clone(),
        )
        .await
        .unwrap();
    let job = delivered(&mut app, &f.cid, "prism-fixture", "uncertain").await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(prism.effects(&job), 1);
    drop(app);
    drop(client);
    server.shutdown().await.unwrap();
    drop(driver);
    drop(prism);

    let prism = Arc::new(Prism::open_file(f.remote.clone()).unwrap());
    let driver = Arc::new(Counted {
        inner: prism.clone(),
        submissions: count.clone(),
    });
    let server = f.start(driver.clone()).await;
    let mut client = CanonicalHttpClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let mut app = ApplicationClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    assert_eq!(
        client
            .send_message("prism-original", &f.cid, "prism-original-message", draft)
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(
        delivered(&mut app, &f.cid, "prism-fixture", "uncertain").await,
        job
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // A later turn remains blocked while the original conversational outcome is
    // unknown. Original read-only reconciliation resumes it without resubmission.
    client
        .send_message(
            "prism-probe",
            &f.cid,
            "prism-probe-message",
            MessageDraft::text(
                MessageAudience::Participants(vec![f.ai_pid.clone()]),
                "fresh prism probe",
            ),
        )
        .await
        .unwrap();
    let probe = provider_job(&mut app, &f.cid, "prism-probe-message", "blocked").await;
    assert_ne!(probe, job);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        delivered(&mut app, &f.cid, "prism-fixture", "uncertain").await,
        job
    );
    let event = Observation {
        mutation_id: job.clone(),
        external_id: Some(format!("prism-message-{job}")),
        event_id: "prism-stock-delivered".into(),
        state: State::Message(MessageState::Delivered),
        verification: Verification::VerifiedCallback,
    };
    let (body, signature) = prism.signed_callback(&event);
    let mut changed = body.clone();
    changed.push(b' ');
    assert!(prism.authenticate_callback(&changed, &signature).is_err());
    assert!(prism.authenticate_callback(&body, &[0; 32]).is_err());
    let receipt = MessageReceipt {
        observation: prism.authenticate_callback(&body, &signature).unwrap(),
        reply: None,
        reply_sources: vec![],
    };
    // Callback authenticity cannot strengthen an unknown outcome. The host
    // requires read-only reconciliation of the original effect first.
    assert!(Worker::observe(server.service.as_ref(), driver.as_ref(), &receipt).is_err());
    assert_eq!(
        provider_job(&mut app, &f.cid, "prism-original-message", "uncertain").await,
        job
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    Worker::reconcile(server.service.as_ref(), driver.as_ref(), &job)
        .await
        .unwrap();
    assert_eq!(
        Worker::observe(server.service.as_ref(), driver.as_ref(), &receipt).unwrap(),
        Apply::Applied
    );
    assert_eq!(
        Worker::observe(server.service.as_ref(), driver.as_ref(), &receipt).unwrap(),
        Apply::Duplicate
    );
    assert_eq!(
        provider_job(&mut app, &f.cid, "prism-probe-message", "completed").await,
        probe
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let page = client
        .history(
            &f.cid,
            serde_json::from_value(serde_json::json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    let reply = page
        .messages
        .iter()
        .find(|m| m.body.starts_with("Historical context:"))
        .unwrap();
    assert!(reply.body.contains("public-context-primer"));
    assert!(!reply.body.contains("exclude-from-connector-secret"));
    let mut peer = CanonicalHttpClient::open(http(&server, &f.peer_token))
        .await
        .unwrap();
    let page = peer
        .history(
            &f.cid,
            serde_json::from_value(serde_json::json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert!(page
        .messages
        .iter()
        .any(|m| m.body == "public-context-primer"));
    assert!(page
        .messages
        .iter()
        .all(|m| !m.body.contains("prism private uncertain question")
            && !m.body.contains("fresh prism probe")));

    let contact = prism
        .independent_message("prism-inbound", "private prism inbound proposal")
        .unwrap();
    let incoming = Worker::receive(
        server.service.as_ref(),
        driver.as_ref(),
        "contributed-route",
        &contact,
    )
    .unwrap();
    assert_eq!(
        Worker::receive(
            server.service.as_ref(),
            driver.as_ref(),
            "contributed-route",
            &contact
        )
        .unwrap(),
        incoming
    );
    let page = client
        .history(
            &f.cid,
            serde_json::from_value(serde_json::json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .filter(|m| m.msg_id.as_str() == incoming && m.body == "private prism inbound proposal")
            .count(),
        1
    );
    let page = peer
        .history(
            &f.cid,
            serde_json::from_value(serde_json::json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert!(page
        .messages
        .iter()
        .all(|m| m.body != "private prism inbound proposal"));
    let Outcome::ContextDeliveries(page) = app
        .execute(
            ApplicationOperation::try_from(Operation::ContextDeliveries {
                conversation_id: f.cid.clone(),
                before: None,
                limit: 100,
            })
            .unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("deliveries")
    };
    assert!(page
        .deliveries
        .iter()
        .any(|v| v.id == format!("provider-job:{job}") && v.state == "delivered"));
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(prism.effects(&job), 1);
    app.provision(ProvisioningCommand::RevokeEndpoint {
        operation_id: "prism-withdraw".into(),
        endpoint_id: f.endpoint_id.clone(),
        expected_revision: 1,
    })
    .await
    .unwrap();
    assert!(Worker::observe(server.service.as_ref(), driver.as_ref(), &receipt).is_err());
    assert!(
        Worker::reconcile(server.service.as_ref(), driver.as_ref(), &job)
            .await
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    drop(peer);
    drop(client);
    drop(app);
    server.shutdown().await.unwrap();
}

async fn provider_job(
    app: &mut ApplicationClient,
    cid: &str,
    message: &str,
    state: &str,
) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let Outcome::ProviderStatus { deliveries } = app
                .execute(
                    ApplicationOperation::try_from(Operation::ProviderStatus {
                        conversation_id: cid.into(),
                    })
                    .unwrap(),
                )
                .await
                .unwrap()
            else {
                panic!("provider status")
            };
            if let Some(row) = deliveries
                .iter()
                .find(|v| v.message_id == message && v.state == state)
            {
                return row.job_id.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Provider job for {message} did not reach {state}"))
}

/// Model a provider acknowledgement lost after the real fixture submission.
/// The stock worker owns cancellation; this adapter holds only its response.
struct HeldAcknowledgement {
    inner: Arc<Counted>,
    submitted: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    active: Arc<AtomicUsize>,
}
struct ActiveDelivery(Arc<AtomicUsize>);
impl Drop for ActiveDelivery {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl Connector for HeldAcknowledgement {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn descriptor(&self) -> Descriptor {
        self.inner.descriptor()
    }
    fn manifest(&self) -> Manifest {
        self.inner.manifest()
    }
    fn configuration(&self) -> BTreeMap<String, String> {
        self.inner.configuration()
    }
    fn readiness(&self) -> Readiness {
        self.inner.readiness()
    }
    fn message_extension(&self) -> Option<&dyn MessageExtension> {
        self.inner.message_extension()
    }
    async fn deliver_observed(
        &self,
        id: &str,
        request: &DeliveryRequest,
        permit: DispatchPermit,
    ) -> Result<MessageReceipt, Failure> {
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = ActiveDelivery(self.active.clone());
        let result = self.inner.deliver_observed(id, request, permit).await;
        self.submitted.notify_one();
        self.release.notified().await;
        result
    }
    async fn deliver(
        &self,
        request: &DeliveryRequest,
        permit: DispatchPermit,
    ) -> Result<Completion, Failure> {
        self.inner.deliver(request, permit).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stock_shutdown_cancels_held_ack_and_reconciles_original_after_restart_without_resubmit() {
    let f = setup::setup("prism-fixture", "prism-offline-key").await;
    let prism = Arc::new(Prism::open_file(f.remote.clone()).unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let submitted = Arc::new(tokio::sync::Notify::new());
    let driver = Arc::new(HeldAcknowledgement {
        inner: Arc::new(Counted {
            inner: prism.clone(),
            submissions: count.clone(),
        }),
        submitted: submitted.clone(),
        release: Arc::new(tokio::sync::Notify::new()),
        active: active.clone(),
    });
    let server = f.start(driver.clone()).await;
    let mut client = CanonicalHttpClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let mut app = ApplicationClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let draft = MessageDraft::text(
        MessageAudience::Participants(vec![f.ai_pid.clone()]),
        "held private acknowledgement",
    );
    let accepted = client
        .send_message(
            "held-ack-original",
            &f.cid,
            "held-ack-message",
            draft.clone(),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), submitted.notified())
        .await
        .unwrap();
    let job = provider_job(&mut app, &f.cid, "held-ack-message", "sending").await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(prism.effects(&job), 1);
    assert_eq!(active.load(Ordering::SeqCst), 1);
    drop(client);
    drop(app);
    server.shutdown().await.unwrap();
    assert_eq!(
        active.load(Ordering::SeqCst),
        0,
        "stock shutdown must cancel and drain the outstanding response future"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(prism.effects(&job), 1);
    drop(driver);
    drop(prism);

    let prism = Arc::new(Prism::open_file(f.remote.clone()).unwrap());
    let driver = Arc::new(Counted {
        inner: prism.clone(),
        submissions: count.clone(),
    });
    let server = f.start(driver.clone()).await;
    let mut client = CanonicalHttpClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let mut app = ApplicationClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    assert_eq!(
        provider_job(&mut app, &f.cid, "held-ack-message", "uncertain").await,
        job
    );
    assert_eq!(
        client
            .send_message("held-ack-original", &f.cid, "held-ack-message", draft)
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    Worker::reconcile(server.service.as_ref(), driver.as_ref(), &job)
        .await
        .unwrap();
    assert_eq!(
        provider_job(&mut app, &f.cid, "held-ack-message", "completed").await,
        job
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(prism.effects(&job), 1);
    let page = client
        .history(
            &f.cid,
            serde_json::from_value(serde_json::json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .filter(|m| m.body.starts_with("Historical context:"))
            .count(),
        1
    );
    drop(client);
    drop(app);
    server.shutdown().await.unwrap();
    assert_eq!(active.load(Ordering::SeqCst), 0);
}
