#![allow(clippy::unwrap_used)]
use async_trait::async_trait;
use connect_client::{ClientConfig, ConnectClient};
use connect_connector_api::{VoiceQuery, VoiceStart};
use connect_connectors::{
    voice::{RemoteCall, VoiceConnector},
    Failure,
};
use connect_protocol::{Operation, Outcome, Response};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer, voice::Worker};
use connect_store::{Scope, VoiceRoute};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
struct Driver {
    starts: AtomicUsize,
    ended: AtomicBool,
    uncertain: bool,
    incoming: bool,
    normalized: bool,
}
#[async_trait]
impl VoiceConnector for Driver {
    fn voice_manifest(&self) -> Option<connect_connector_api::manifest::Manifest> {
        self.normalized
            .then(connect_connectors::provider_mapping::vapi_telephone_manifest)
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        connect_connectors::provider_mapping::references("private-key", "OFFLINE_FIXTURE")
    }
    fn normalize(
        &self,
        id: &str,
        call: &RemoteCall,
    ) -> Result<connect_connector_api::lifecycle::Observation, Failure> {
        use connect_connector_api::lifecycle::*;
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
    async fn inbound(&self, b: &VoiceQuery) -> Result<Vec<connect_store::InboundCall>, Failure> {
        if !self.incoming {
            return Ok(vec![]);
        }
        Ok(vec![connect_store::InboundCall {
            provider_id: "12345678-1234-1234-1234-123456789015".into(),
            assistant_id: b.assistant_id.clone(),
            phone_number_id: b.phone_number_id.clone(),
            caller: "+15555550101".into(),
            created_at: chrono::Utc::now().timestamp(),
            state: "in-progress".into(),
            control_url: None,
            ended_reason: None,
        }])
    }

    async fn start(&self, _: &VoiceStart) -> Result<RemoteCall, Failure> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.uncertain {
            Err(Failure::Uncertain)
        } else {
            Ok(self.remote("queued"))
        }
    }
    async fn status(&self, _: &str) -> Result<RemoteCall, Failure> {
        Ok(self.remote(if self.ended.load(Ordering::SeqCst) {
            "ended"
        } else {
            "in-progress"
        }))
    }
    async fn end(&self, _: &str, control: Option<&str>) -> Result<RemoteCall, Failure> {
        assert!(control.is_some_and(|url| url.ends_with("/control")));
        self.ended.store(true, Ordering::SeqCst);
        Ok(self.remote("ended"))
    }
}
impl Driver {
    fn remote(&self, state: &str) -> RemoteCall {
        RemoteCall {
            id: "12345678-1234-1234-1234-123456789015".into(),
            state: state.into(),
            control_url: Some(
                "https://fixture.vapi.ai/12345678-1234-1234-1234-123456789015/control".into(),
            ),
            ended_reason: None,
        }
    }
}
fn start_worker(server: &RunningServer, driver: Arc<Driver>, normalized: bool) -> Worker {
    if normalized {
        Worker::with_observed_driver(server.service.clone(), driver).unwrap()
    } else {
        Worker::with_driver(server.service.clone(), driver).unwrap()
    }
}
async fn run(uncertain: bool, incoming: bool, normalized: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&dir.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let mut app_config = ClientConfig::load(&dir.join("reviews.toml")).unwrap();
    app_config.endpoint = server.address;
    let bot = ClientConfig::load(&dir.join("agent.toml")).unwrap();
    let operator = ClientConfig::load(&dir.join("operator.toml")).unwrap();
    let authority = server.store.authenticate(&operator.credential).unwrap();
    let key = server
        .store
        .issue_as(
            &authority,
            &app_config.identity_id,
            Scope::communications(vec![bot.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    app_config.credential = key.token;
    app_config.credential_id = key.id;
    let mut app = ConnectClient::open(app_config.clone()).await.unwrap();
    let conv = app
        .create("create", vec![bot.identity_id.clone()])
        .await
        .unwrap();
    let route = VoiceRoute {
        id: "voice".into(),
        conversation_id: conv.id.clone(),
        identity_id: bot.identity_id,
        assistant_id: "12345678-1234-1234-1234-123456789012".into(),
        phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
        destinations: vec!["+15555550101".into()],
        enabled: true,
    };
    server
        .store
        .configure_voice_route(&authority.credential_id, &route)
        .unwrap();
    let driver = Arc::new(Driver {
        starts: AtomicUsize::new(0),
        ended: AtomicBool::new(false),
        uncertain,
        incoming,
        normalized,
    });
    if incoming {
        server
            .store
            .configure_voice_ingress(&authority.credential_id, &route.id, &route.phone_number_id)
            .unwrap();
        let worker = start_worker(&server, driver.clone(), normalized);
        let call = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = app.inbox(None, 100, 100).await.unwrap();
                if let Some(call) = page.events.iter().find_map(|e| e.call.clone()) {
                    break call;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(call.direction, "inbound");
        assert_eq!(
            app.call_status(&conv.id, &call.id)
                .await
                .unwrap()
                .provider_id,
            call.provider_id
        );
        worker.shutdown().await.unwrap();
        let restarted = start_worker(&server, driver.clone(), normalized);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let page = app.inbox(None, 100, 0).await.unwrap();
        assert_eq!(
            page.events
                .iter()
                .filter(|e| e.call.as_ref().is_some_and(|c| c.id == call.id))
                .count(),
            1
        );
        assert_eq!(driver.starts.load(Ordering::SeqCst), 0);
        app.call_end("end-inbound", &conv.id, &call.id)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while app.call_status(&conv.id, &call.id).await.unwrap().state != "ended" {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        restarted.shutdown().await.unwrap();
        app.close().await;
        server.shutdown().await.unwrap();
        return;
    }
    let worker = start_worker(&server, driver.clone(), normalized);
    let call = app
        .call_start("dial", &conv.id, &route.id, &route.destinations[0])
        .await
        .unwrap();
    let replay: Response = reqwest::Client::new()
        .post(format!(
            "http://{}/v1/operations",
            server.http_address.unwrap()
        ))
        .bearer_auth(&app_config.credential)
        .json(&Operation::CallStart {
            request_id: "dial".into(),
            conversation_id: conv.id.clone(),
            route_id: route.id.clone(),
            destination: route.destinations[0].clone(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay.result.unwrap(), Outcome::Call(call.clone()));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let c = app.call_status(&conv.id, &call.id).await.unwrap();
            if !["pending", "starting"].contains(&c.state.as_str()) {
                if uncertain {
                    assert_eq!(c.state, "uncertain");
                } else {
                    assert!(
                        ["queued", "in-progress"].contains(&c.state.as_str()),
                        "unexpected provider state: {}",
                        c.state
                    );
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
    if uncertain {
        worker.shutdown().await.unwrap();
        let restarted = start_worker(&server, driver.clone(), normalized);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
        restarted.shutdown().await.unwrap();
    } else {
        app.call_end("end", &conv.id, &call.id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if app.call_status(&conv.id, &call.id).await.unwrap().state == "ended" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let page = app.inbox(None, 100, 0).await.unwrap();
        assert!(page.events.iter().any(|e| e
            .call
            .as_ref()
            .is_some_and(|c| c.id == call.id && c.state == "ended")));
        let encoded = serde_json::to_string(&page.events).unwrap();
        assert!(!encoded.contains("control"));
        worker.shutdown().await.unwrap();
        if normalized {
            let c = rusqlite::Connection::open(dir.join("state.sqlite3")).unwrap();
            let json: String = c
                .query_row(
                    "SELECT projection_json FROM connector_voice_observations WHERE call_id=?1",
                    [&call.id],
                    |row| row.get(0),
                )
                .unwrap();
            let projection: connect_connector_api::lifecycle::Projection =
                serde_json::from_str(&json).unwrap();
            assert_eq!(
                projection.state,
                connect_connector_api::lifecycle::State::Voice(
                    connect_connector_api::lifecycle::VoiceState::Ended
                )
            );
            assert_eq!(
                projection.external_id.as_deref(),
                Some("12345678-1234-1234-1234-123456789015")
            );
        }
    }
    app.close().await;
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread")]
async fn quic_http_replay_starts_once_and_end_reaches_journal() {
    run(false, false, false).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn uncertain_provider_start_is_never_retried_after_worker_restart() {
    run(true, false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn inbound_discovery_reaches_native_sdk_and_restart_never_redials() {
    run(false, true, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn normalized_voice_worker_preserves_private_control_and_journals_end() {
    run(false, false, true).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn normalized_voice_uncertain_creation_still_never_redials() {
    run(true, false, true).await;
}

struct RestrictedDriver {
    inner: Driver,
    discover: bool,
    start: bool,
    discoveries: AtomicUsize,
}
#[async_trait]
impl VoiceConnector for RestrictedDriver {
    fn voice_manifest(&self) -> Option<connect_connector_api::manifest::Manifest> {
        use connect_connector_api::manifest::ConnectorOperation;
        let mut manifest = connect_connectors::provider_mapping::vapi_telephone_manifest();
        if !self.discover {
            manifest
                .operations
                .remove(&ConnectorOperation::VoiceDiscover);
        }
        if !self.start {
            manifest.operations.remove(&ConnectorOperation::VoiceStart);
        }
        Some(manifest)
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        self.inner.voice_configuration()
    }
    fn normalize(
        &self,
        id: &str,
        call: &RemoteCall,
    ) -> Result<connect_connector_api::lifecycle::Observation, Failure> {
        self.inner.normalize(id, call)
    }
    async fn inbound(
        &self,
        query: &VoiceQuery,
    ) -> Result<Vec<connect_store::InboundCall>, Failure> {
        assert!(
            self.discover,
            "voice discovery must never call an undeclared provider operation"
        );
        self.discoveries.fetch_add(1, Ordering::SeqCst);
        self.inner.inbound(query).await
    }
    async fn start(&self, request: &VoiceStart) -> Result<RemoteCall, Failure> {
        assert!(
            self.start,
            "voice start must never call an undeclared provider operation"
        );
        self.inner.start(request).await
    }
    async fn status(&self, id: &str) -> Result<RemoteCall, Failure> {
        self.inner.status(id).await
    }
    async fn end(&self, id: &str, control: Option<&str>) -> Result<RemoteCall, Failure> {
        self.inner.end(id, control).await
    }
}
async fn restricted_operations(discover: bool, start: bool) {
    use connect_connector_api::manifest::ConnectorOperation;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&dir.join("server.toml")).unwrap(),
        None,
        false,
    )
    .await
    .unwrap();
    let mut config = ClientConfig::load(&dir.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let bot = ClientConfig::load(&dir.join("agent.toml")).unwrap();
    let operator = ClientConfig::load(&dir.join("operator.toml")).unwrap();
    let authority = server.store.authenticate(&operator.credential).unwrap();
    let key = server
        .store
        .issue_as(
            &authority,
            &config.identity_id,
            Scope::communications(vec![bot.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    config.credential = key.token;
    config.credential_id = key.id;
    let mut app = ConnectClient::open(config).await.unwrap();
    let conversation = app
        .create("restricted-conversation", vec![bot.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let route = VoiceRoute {
        id: "restricted-route".into(),
        conversation_id: conversation.clone(),
        identity_id: bot.identity_id,
        assistant_id: "12345678-1234-1234-1234-123456789012".into(),
        phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
        destinations: vec!["+15555550101".into()],
        enabled: true,
    };
    server
        .store
        .configure_voice_route(&authority.credential_id, &route)
        .unwrap();
    // Both cases have a real configured ingress. Absence of VoiceDiscover must
    // prevent calling it; absence of VoiceStart must still allow inbound use.
    server
        .store
        .configure_voice_ingress(&authority.credential_id, &route.id, &route.phone_number_id)
        .unwrap();
    let driver = Arc::new(RestrictedDriver {
        inner: Driver {
            starts: AtomicUsize::new(0),
            ended: AtomicBool::new(false),
            uncertain: false,
            incoming: discover,
            normalized: true,
        },
        discover,
        start,
        discoveries: AtomicUsize::new(0),
    });
    let worker = Worker::with_observed_driver(server.service.clone(), driver.clone()).unwrap();
    let absent = if start {
        ConnectorOperation::VoiceDiscover
    } else {
        ConnectorOperation::VoiceStart
    };
    assert_eq!(
        server
            .store
            .require_connector_operation("vapi-telephone", absent)
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    let call = app
        .call_start(
            "restricted-dial",
            &conversation,
            &route.id,
            &route.destinations[0],
        )
        .await
        .unwrap();
    let connection = rusqlite::Connection::open(server.store.path()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = app.call_status(&conversation, &call.id).await.unwrap();
            if (start && current.provider_id.is_some()) || (!start && current.state == "failed") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    if start {
        assert_eq!(driver.discoveries.load(Ordering::SeqCst), 0);
        assert_eq!(driver.inner.starts.load(Ordering::SeqCst), 1);
        app.call_end("restricted-end", &conversation, &call.id)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while app
                .call_status(&conversation, &call.id)
                .await
                .unwrap()
                .state
                != "ended"
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    } else {
        let current = app.call_status(&conversation, &call.id).await.unwrap();
        assert_eq!(
            current.ended_reason.as_deref(),
            Some("unsupported-connector-operation")
        );
        assert!(current.provider_id.is_none());
        assert_eq!(
            connection
                .query_row::<u64, _, _>(
                    "SELECT COUNT(*) FROM media_start_admissions WHERE resource_id=?1",
                    [&call.id],
                    |row| row.get(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(driver.inner.starts.load(Ordering::SeqCst), 0);
        let json: String = connection
            .query_row(
                "SELECT projection_json FROM connector_voice_observations WHERE call_id=?1",
                [&call.id],
                |row| row.get(0),
            )
            .unwrap();
        let projection: connect_connector_api::lifecycle::Projection =
            serde_json::from_str(&json).unwrap();
        assert_eq!(
            projection.state,
            connect_connector_api::lifecycle::State::Voice(
                connect_connector_api::lifecycle::VoiceState::Failed
            )
        );
        let inbound = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = app.inbox(None, 100, 0).await.unwrap();
                if let Some(inbound) = page.events.iter().find_map(|event| {
                    event
                        .call
                        .as_ref()
                        .filter(|call| call.direction == "inbound")
                        .cloned()
                }) {
                    let control: Option<String> = connection
                        .query_row(
                            "SELECT control_url FROM voice_calls WHERE id=?1",
                            [&inbound.id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    if control.is_some() {
                        break inbound;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(driver.discoveries.load(Ordering::SeqCst) > 0);
        app.call_end("inbound-only-end", &conversation, &inbound.id)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while app
                .call_status(&conversation, &inbound.id)
                .await
                .unwrap()
                .state
                != "ended"
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    worker.shutdown().await.unwrap();
    let restarted = Worker::with_observed_driver(server.service.clone(), driver.clone()).unwrap();
    assert_eq!(
        driver.inner.starts.load(Ordering::SeqCst),
        usize::from(start)
    );
    restarted.shutdown().await.unwrap();
    app.close().await;
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn normalized_voice_without_discovery_does_not_call_inbound_but_retains_outbound_control() {
    restricted_operations(false, true).await;
}
#[tokio::test]
async fn normalized_voice_without_start_refuses_creation_but_retains_inbound_status_and_cleanup() {
    restricted_operations(true, false).await;
}
