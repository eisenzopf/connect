//! Backend worker fixtures, independent of the retired public call dialect.
#![allow(clippy::unwrap_used)]
use async_trait::async_trait;
use connect_connector_api::{
    lifecycle::*, manifest::*, Failure, InboundCall, RemoteCall, VoiceConnector, VoiceQuery,
    VoiceStart,
};
use connect_protocol::{Limits, Operation, Outcome};
use connect_server::voice::Worker;
use connect_service::Service;
use connect_store::{Scope, Store, VoiceRoute};
use rusqlite::OptionalExtension;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
struct Driver {
    discoveries: AtomicUsize,
    starts: AtomicUsize,
    ends: AtomicUsize,
    normalization_fails: bool,
    can_start: bool,
    discover: bool,
}
#[async_trait]
impl VoiceConnector for Driver {
    fn voice_manifest(&self) -> Manifest {
        let mut m = connect_connectors::provider_mapping::vapi_telephone_manifest();
        m.operations.remove(&ConnectorOperation::VoiceReconcile);
        if !self.discover {
            m.operations.remove(&ConnectorOperation::VoiceDiscover);
        }
        if !self.can_start {
            m.operations.remove(&ConnectorOperation::VoiceStart);
        }
        m
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        connect_connectors::provider_mapping::references("private-key", "FIXTURE_ONLY")
    }
    fn normalize(&self, id: &str, call: &RemoteCall) -> Result<Observation, Failure> {
        if self.normalization_fails {
            return Err(Failure::Rejected);
        }
        Ok(Observation {
            mutation_id: id.into(),
            external_id: Some(call.id.clone()),
            event_id: format!("{}:{}", call.id, call.state),
            state: State::Voice(connect_connectors::provider_mapping::vapi_voice_state(
                &call.state,
            )),
            verification: Verification::AuthenticatedResponse,
        })
    }
    async fn inbound(&self, q: &VoiceQuery) -> Result<Vec<InboundCall>, Failure> {
        assert!(self.discover, "undeclared discovery was dispatched");
        self.discoveries.fetch_add(1, Ordering::SeqCst);
        Ok(vec![InboundCall {
            provider_id: self.remote().id,
            assistant_id: q.assistant_id.clone(),
            phone_number_id: q.phone_number_id.clone(),
            caller: "+15555550100".into(),
            created_at: chrono::Utc::now().timestamp(),
            state: "ended".into(),
            control_url: None,
            ended_reason: Some("customer-ended".into()),
        }])
    }
    async fn start(&self, _: &VoiceStart) -> Result<RemoteCall, Failure> {
        assert!(self.can_start);
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(self.remote())
    }
    async fn status(&self, _: &str) -> Result<RemoteCall, Failure> {
        Ok(self.remote())
    }
    async fn end(&self, _: &str, _: Option<&str>) -> Result<RemoteCall, Failure> {
        self.ends.fetch_add(1, Ordering::SeqCst);
        Ok(self.remote())
    }
}
impl Driver {
    fn remote(&self) -> RemoteCall {
        RemoteCall {
            id: "12345678-1234-1234-1234-123456789015".into(),
            state: if self.ends.load(Ordering::SeqCst) > 0 {
                "ended"
            } else {
                "in-progress"
            }
            .into(),
            control_url: None,
            ended_reason: None,
        }
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    service: Arc<Service>,
    key: String,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let owner = store.create_identity("voice-owner").unwrap().id;
        let key = store
            .issue(
                &owner,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap()
            .id;
        let Outcome::Conversation(c) = store
            .execute(
                &key,
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        else {
            panic!("room")
        };
        store
            .configure_voice_route(
                &key,
                &VoiceRoute {
                    id: "voice".into(),
                    conversation_id: c.id.clone(),
                    identity_id: owner,
                    assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                    phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                    destinations: vec!["+15555550100".into()],
                    enabled: true,
                },
            )
            .unwrap();
        Self {
            _dir: dir,
            service: Service::new(store),
            key,
            cid: c.id,
        }
    }
    fn start(&self) -> String {
        let Outcome::Call(call) = self
            .service
            .store
            .execute(
                &self.key,
                &Operation::CallStart {
                    request_id: "logical-start".into(),
                    conversation_id: self.cid.clone(),
                    route_id: "voice".into(),
                    destination: "+15555550100".into(),
                },
            )
            .unwrap()
        else {
            panic!("call")
        };
        call.id
    }
    async fn wait(&self, id: &str, state: &str) {
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let Outcome::Call(call) = self
                .service
                .store
                .execute(
                    &self.key,
                    &Operation::CallStatus {
                        conversation_id: self.cid.clone(),
                        call_id: id.into(),
                    },
                )
                .unwrap()
            else {
                panic!("status")
            };
            if call.state == state {
                return;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "state {} expected {state}",
                call.state
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    fn projection(&self, id: &str) -> Projection {
        let c = rusqlite::Connection::open(self.service.store.path()).unwrap();
        let s: String = c
            .query_row(
                "SELECT projection_json FROM connector_voice_observations WHERE call_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&s).unwrap()
    }
}
#[tokio::test]
async fn worker_uses_declared_receipts_for_start_status_end_and_replays_once() {
    let f = Fixture::new();
    let d = Arc::new(Driver {
        discoveries: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        ends: AtomicUsize::new(0),
        normalization_fails: false,
        can_start: true,
        discover: false,
    });
    let id = f.start();
    let w = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    f.wait(&id, "in-progress").await;
    assert_eq!(f.start(), id);
    assert_eq!(f.projection(&id).state, State::Voice(VoiceState::Active));
    f.service
        .store
        .execute(
            &f.key,
            &Operation::CallEnd {
                request_id: "logical-end".into(),
                conversation_id: f.cid.clone(),
                call_id: id.clone(),
            },
        )
        .unwrap();
    f.wait(&id, "ended").await;
    assert_eq!(f.projection(&id).state, State::Voice(VoiceState::Ended));
    w.shutdown().await.unwrap();
    assert_eq!(d.starts.load(Ordering::SeqCst), 1);
    assert_eq!(d.ends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn failed_normalization_after_start_is_uncertain_and_restart_never_redials() {
    let f = Fixture::new();
    let d = Arc::new(Driver {
        discoveries: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        ends: AtomicUsize::new(0),
        normalization_fails: true,
        can_start: true,
        discover: false,
    });
    let id = f.start();
    let w = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    f.wait(&id, "uncertain").await;
    assert_eq!(f.projection(&id).state, State::Voice(VoiceState::Uncertain));
    w.shutdown().await.unwrap();
    let w = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    w.shutdown().await.unwrap();
    assert_eq!(d.starts.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn missing_start_capability_refuses_before_io_and_records_typed_failure() {
    let f = Fixture::new();
    let d = Arc::new(Driver {
        discoveries: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        ends: AtomicUsize::new(0),
        normalization_fails: false,
        can_start: false,
        discover: false,
    });
    let id = f.start();
    let w = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    f.wait(&id, "failed").await;
    assert_eq!(f.projection(&id).state, State::Voice(VoiceState::Failed));
    w.shutdown().await.unwrap();
    assert_eq!(d.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn discovery_imports_terminal_typed_receipt_once_without_dial_and_replays_on_restart() {
    let f = Fixture::new();
    f.service
        .store
        .configure_voice_ingress(&f.key, "voice", "12345678-1234-1234-1234-123456789013")
        .unwrap();
    let d = Arc::new(Driver {
        discoveries: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        ends: AtomicUsize::new(1),
        normalization_fails: false,
        can_start: false,
        discover: true,
    });
    let w = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let db = rusqlite::Connection::open(f.service.store.path()).unwrap();
    let id = loop {
        let id: Option<String> = db
            .query_row(
                "SELECT id FROM voice_calls WHERE direction='inbound'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        if let Some(id) = id {
            break id;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "no inbound receipt imported"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(f.projection(&id).state, State::Voice(VoiceState::Ended));
    w.shutdown().await.unwrap();
    let w = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    w.shutdown().await.unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM voice_calls", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(d.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn discovery_decoder_failure_keeps_scan_cursor_without_import_or_dial() {
    let f = Fixture::new();
    f.service
        .store
        .configure_voice_ingress(&f.key, "voice", "12345678-1234-1234-1234-123456789013")
        .unwrap();
    let before = f.service.store.voice_ingresses().unwrap()[0].scanned_until;
    let d = Arc::new(Driver {
        discoveries: AtomicUsize::new(0),
        starts: AtomicUsize::new(0),
        ends: AtomicUsize::new(1),
        normalization_fails: true,
        can_start: false,
        discover: true,
    });
    let w = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    while d.discoveries.load(Ordering::SeqCst) == 0 {
        assert!(tokio::time::Instant::now() < until, "discovery did not run");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    w.shutdown().await.unwrap();
    assert_eq!(
        f.service.store.voice_ingresses().unwrap()[0].scanned_until,
        before
    );
    let db = rusqlite::Connection::open(f.service.store.path()).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM voice_calls", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(d.starts.load(Ordering::SeqCst), 0);
}

struct PendingDriver {
    inner: Driver,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl PendingDriver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Driver {
                discoveries: AtomicUsize::new(0),
                starts: AtomicUsize::new(0),
                ends: AtomicUsize::new(0),
                normalization_fails: false,
                can_start: true,
                discover: false,
            },
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }
}
#[async_trait]
impl VoiceConnector for PendingDriver {
    fn voice_manifest(&self) -> Manifest {
        self.inner.voice_manifest()
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        self.inner.voice_configuration()
    }
    fn normalize(&self, id: &str, call: &RemoteCall) -> Result<Observation, Failure> {
        self.inner.normalize(id, call)
    }
    async fn inbound(&self, q: &VoiceQuery) -> Result<Vec<InboundCall>, Failure> {
        self.inner.inbound(q).await
    }
    async fn start(&self, _: &VoiceStart) -> Result<RemoteCall, Failure> {
        self.inner.starts.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(self.inner.remote())
    }
    async fn status(&self, id: &str) -> Result<RemoteCall, Failure> {
        self.inner.status(id).await
    }
    async fn end(&self, id: &str, control: Option<&str>) -> Result<RemoteCall, Failure> {
        self.inner.end(id, control).await
    }
}
async fn raw_state(f: &Fixture, id: &str, expected: &str) {
    let until = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let c = rusqlite::Connection::open(f.service.store.path()).unwrap();
        let state: String = c
            .query_row("SELECT state FROM voice_calls WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        if state == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "{state} expected {expected}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test]
async fn pending_voice_io_retains_authority_then_records_actual_result_after_queued_revoke() {
    let f = Fixture::new();
    let d = PendingDriver::new();
    let id = f.start();
    let worker = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), d.entered.notified())
        .await
        .unwrap();
    let store = f.service.store.clone();
    let actor = store.revalidate(&f.key).unwrap();
    let key = f.key.clone();
    let mut writer = tokio::task::spawn_blocking(move || store.revoke(&actor, &key));
    assert!(
        tokio::time::timeout(Duration::from_millis(150), &mut writer)
            .await
            .is_err()
    );
    d.release.notify_one();
    writer.await.unwrap().unwrap();
    assert!(f.service.store.revalidate(&f.key).is_err());
    raw_state(&f, &id, "in-progress").await;
    assert_eq!(
        f.projection(&id).external_id.as_deref(),
        Some("12345678-1234-1234-1234-123456789015")
    );
    worker.shutdown().await.unwrap();
    assert_eq!(d.inner.starts.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn pending_voice_request_expires_with_original_credential_and_restart_never_redials() {
    let mut f = Fixture::new();
    let actor = f.service.store.revalidate(&f.key).unwrap();
    f.key = f
        .service
        .store
        .issue(
            &actor.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 2,
        )
        .unwrap()
        .id;
    let id = f.start();
    let d = PendingDriver::new();
    let worker = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), d.entered.notified())
        .await
        .unwrap();
    let began = tokio::time::Instant::now();
    raw_state(&f, &id, "uncertain").await;
    assert!(began.elapsed() < Duration::from_secs(3));
    assert_eq!(f.projection(&id).state, State::Voice(VoiceState::Uncertain));
    worker.shutdown().await.unwrap();
    let restarted = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    restarted.shutdown().await.unwrap();
    assert_eq!(d.inner.starts.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn cancelling_pending_voice_request_records_uncertainty_and_joins_before_revoke_finishes() {
    let f = Fixture::new();
    let d = PendingDriver::new();
    let id = f.start();
    let worker = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), d.entered.notified())
        .await
        .unwrap();
    let store = f.service.store.clone();
    let actor = store.revalidate(&f.key).unwrap();
    let key = f.key.clone();
    let writer = tokio::task::spawn_blocking(move || store.revoke(&actor, &key));
    worker.shutdown().await.unwrap();
    writer.await.unwrap().unwrap();
    raw_state(&f, &id, "uncertain").await;
    assert_eq!(d.inner.starts.load(Ordering::SeqCst), 1);
    assert!(f.projection(&id).external_id.is_none());
}
#[tokio::test]
async fn captured_voice_worker_retries_only_metadata_through_record_outage_and_revoke() {
    let f = Fixture::new();
    let id = f.start();
    let c = rusqlite::Connection::open(f.service.store.path()).unwrap();
    c.execute_batch("CREATE TRIGGER fail_capture BEFORE INSERT ON connector_voice_observations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let d = PendingDriver::new();
    let worker = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), d.entered.notified())
        .await
        .unwrap();
    d.release.notify_one();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let store = f.service.store.clone();
    let actor = store.revalidate(&f.key).unwrap();
    let key = f.key.clone();
    tokio::task::spawn_blocking(move || store.revoke(&actor, &key))
        .await
        .unwrap()
        .unwrap();
    c.execute_batch("DROP TRIGGER fail_capture").unwrap();
    raw_state(&f, &id, "in-progress").await;
    worker.shutdown().await.unwrap();
    assert_eq!(d.inner.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        serde_json::to_value(f.projection(&id)).unwrap()["events"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn overlapping_voice_worker_cannot_recover_a_live_dispatch_and_restart_waits_for_drain() {
    let f = Fixture::new();
    let d = PendingDriver::new();
    let id = f.start();
    let first = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), d.entered.notified())
        .await
        .unwrap();
    let error = match Worker::with_driver(f.service.clone(), d.clone()) {
        Ok(_) => panic!("overlapping worker accepted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("already owns this runtime"));
    raw_state(&f, &id, "starting").await;
    d.release.notify_one();
    raw_state(&f, &id, "in-progress").await;
    first.shutdown().await.unwrap();
    let next = Worker::with_driver(f.service.clone(), d.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    next.shutdown().await.unwrap();
    assert_eq!(d.inner.starts.load(Ordering::SeqCst), 1);
}
