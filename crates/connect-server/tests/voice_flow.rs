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
}
#[async_trait]
impl VoiceConnector for Driver {
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
    async fn end(&self, _: &str, _: Option<&str>) -> Result<RemoteCall, Failure> {
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
async fn run(uncertain: bool, incoming: bool) {
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
    });
    if incoming {
        server
            .store
            .configure_voice_ingress(&authority.credential_id, &route.id, &route.phone_number_id)
            .unwrap();
        let worker = Worker::with_driver(server.service.clone(), driver.clone()).unwrap();
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
        let restarted = Worker::with_driver(server.service.clone(), driver.clone()).unwrap();
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
    let worker = Worker::with_driver(server.service.clone(), driver.clone()).unwrap();
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
                assert_eq!(c.state, if uncertain { "uncertain" } else { "queued" });
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
        let restarted = Worker::with_driver(server.service.clone(), driver.clone()).unwrap();
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
    }
    app.close().await;
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread")]
async fn quic_http_replay_starts_once_and_end_reaches_journal() {
    run(false, false).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn uncertain_provider_start_is_never_retried_after_worker_restart() {
    run(true, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn inbound_discovery_reaches_native_sdk_and_restart_never_redials() {
    run(false, true).await;
}
