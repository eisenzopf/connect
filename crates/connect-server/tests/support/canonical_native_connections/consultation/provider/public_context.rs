//! Actual public context HTTP/WS delivery on the existing signed native source.
use super::*;
use connect_server::assistant_vapi::*;
use futures_util::SinkExt;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Ack,
    Truncated,
    Missing,
    Rejected,
    LostCreation,
    Revoked,
    SocketLost,
    EndedBeforeAck,
    ReusedReference,
}
#[derive(Clone)]
struct PublicProvider {
    base: String,
    reference: String,
    mode: Mode,
    requests: Arc<AtomicUsize>,
    exports: Arc<AtomicUsize>,
    ends: Arc<AtomicUsize>,
}
async fn create_public(
    State(s): State<PublicProvider>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer public-fixture-token"
    );
    s.requests.fetch_add(1, Ordering::SeqCst);
    assert_eq!(body["assistantId"], "private-fixture-assistant");
    if s.mode == Mode::Ack {
        observation_sources::verify_proposal_tools(&body);
    } else {
        assert_eq!(body["assistantOverrides"]["model"]["tools"], json!([]));
    }
    assert_eq!(body["assistantOverrides"]["model"]["toolIds"], json!([]));
    if s.mode == Mode::Rejected {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error":"designated rejection"})),
        )
            .into_response();
    }
    if s.mode == Mode::LostCreation {
        return Json(json!({"id":s.reference,"transport":{}})).into_response();
    }
    Json(json!({"id":s.reference,"transport":{"websocketCallUrl":format!("{}/{}/transport",s.base.replace("http:","ws:"),s.reference)}})).into_response()
}
async fn upgrade_public(
    State(s): State<PublicProvider>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer public-fixture-token"
    );
    upgrade.on_upgrade(move |socket| public_socket(socket, s))
}
async fn public_socket(mut socket: WebSocket, s: PublicProvider) {
    while let Some(Ok(frame)) = socket.recv().await {
        let Message::Text(text) = frame else {
            if matches!(frame, Message::Close(_)) {
                break;
            }
            continue;
        };
        let value: Value = serde_json::from_str(&text).unwrap();
        if value["type"] == "end-call" {
            s.ends.fetch_add(1, Ordering::SeqCst);
            let _ = socket
                .send(Message::Text(
                    json!({"type":"status-update","status":"ended"})
                        .to_string()
                        .into(),
                ))
                .await;
            continue;
        }
        if value["type"] != "add-message" {
            continue;
        }
        assert_eq!(
            s.exports.fetch_add(1, Ordering::SeqCst),
            0,
            "public context exported twice"
        );
        assert_eq!(value["message"]["role"], "system");
        assert_eq!(value["triggerResponseEnabled"], false);
        let body = value["message"]["content"].as_str().unwrap();
        let view: connect_protocol::ContextView = serde_json::from_str(body).unwrap();
        assert!(view
            .items
            .iter()
            .any(|item| item.body == "authorized provider history marker"));
        assert!(view.items.iter().any(|item| item.kind == "session_control"));
        assert!(view.items.iter().all(|item| !item.source_id.is_empty()
            && !item.actor_id.is_empty()
            && item.sequence > 0
            && item.sequence <= view.watermark));
        assert!(!body.contains("assistant-context-pair-only marker"));
        assert!(!body.contains("assistant-context-ai-only marker"));
        assert_eq!(view.omitted, 0);
        if s.mode == Mode::EndedBeforeAck {
            socket
                .send(Message::Text(
                    json!({"type":"status-update","status":"ended"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            continue;
        }
        if s.mode == Mode::SocketLost {
            let _ = socket.close().await;
            return;
        }
        if matches!(s.mode, Mode::Ack | Mode::Revoked | Mode::Truncated) {
            let observed = if s.mode == Mode::Truncated {
                body.chars()
                    .take(body.chars().count() / 2)
                    .collect::<String>()
            } else {
                body.to_owned()
            };
            socket.send(Message::Text(json!({"type":"conversation-update","messagesOpenAIFormatted":[{"role":"system","content":observed}]}).to_string().into())).await.unwrap();
            if s.mode == Mode::Ack {
                for frame in observation_sources::frames() {
                    socket
                        .send(Message::Text(frame.to_string().into()))
                        .await
                        .unwrap();
                }
            }
        } else {
            socket
                .send(Message::Text(
                    json!({"type":"status-update","status":"active"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
    }
}
pub(super) async fn selection(
    f: &Fixture,
    key: &str,
    endpoint: &str,
) -> connect_service::AssistantContextRead {
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match f
            .server
            .service
            .try_assistant_context_read(key, "public-consultant", 1, endpoint)
        {
            Ok(read) => return read,
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "not-ready" | "authority-busy" | "storage-unavailable"
                ) && tokio::time::Instant::now() < until =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("actual public provider source unavailable: {}", e.code),
        }
    }
}
pub(super) async fn verify(f: &Fixture, key: &str, endpoint: &str) {
    for (index, mode) in [
        Mode::Ack,
        Mode::Truncated,
        Mode::Missing,
        Mode::Rejected,
        Mode::LostCreation,
        Mode::Revoked,
        Mode::SocketLost,
        Mode::EndedBeforeAck,
        Mode::ReusedReference,
    ]
    .into_iter()
    .enumerate()
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let provider = PublicProvider {
            base: base.clone(),
            reference: if mode == Mode::ReusedReference {
                "public-context-call-0".into()
            } else {
                format!("public-context-call-{index}")
            },
            mode,
            requests: Arc::new(AtomicUsize::new(0)),
            exports: Arc::new(AtomicUsize::new(0)),
            ends: Arc::new(AtomicUsize::new(0)),
        };
        let route = format!("/{}/transport", provider.reference);
        let app = Router::new()
            .route("/call", post(create_public))
            .route(&route, get(upgrade_public))
            .with_state(provider.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let context_key = if mode == Mode::Revoked {
            let actor = f.server.store.revalidate(key).unwrap();
            f.server
                .store
                .issue(
                    &actor.identity_id,
                    actor.scope,
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap()
                .id
        } else {
            key.into()
        };
        let (observations, work) = if mode == Mode::Ack {
            let cid = selection(f, key, endpoint)
                .await
                .target()
                .connection
                .conversation_id
                .clone();
            let (config, work) = observation_sources::configuration_with_work(f, key, &cid).await;
            (Some(config), Some(work))
        } else {
            (None, None)
        };
        let profile=AssistantVapiProfile{observations,context_timing:AssistantProviderTiming::AfterCreation,api_base:format!("{base}/").parse().unwrap(),websocket_origin:format!("{}/",base.replace("http:","ws:")).parse().unwrap(),loopback_fixture:true,model:"designated-public-context-model".into(),application_prompt:"You are the configured public assistant. Use only the supplied current public audience history.".into()};
        let secrets: ProviderSecrets = Arc::new(|reference| {
            assert_eq!(reference, "private_fixture_vapi_secret");
            connect_server::secrets::SecretValue::new("public-fixture-token".into())
                .map_err(|_| connect_protocol::Error::forbidden())
        });
        let read = selection(f, &context_key, endpoint).await;
        let target = read.target().connection.clone();
        let operation = format!("public-context-lifecycle-{index}");
        let mut session = AssistantVapiContextSession::start(
            f.server.service.clone(),
            operation.clone(),
            read,
            profile.clone(),
            secrets.clone(),
        )
        .await
        .unwrap();
        let delivery = session.delivery_id().to_owned();
        let ready = tokio::time::timeout(Duration::from_secs(12), session.context_ready())
            .await
            .unwrap();
        if matches!(mode, Mode::Ack | Mode::Revoked) {
            ready.unwrap();
        } else {
            assert!(
                ready.is_err(),
                "non-exact or rejected context was reported delivered"
            );
        }
        if mode == Mode::Ack {
            observation_sources::verify_live(
                f,
                &target.conversation_id,
                &target.session_id,
                &delivery,
                &target.participant_id,
                || format!("{:?}", session.status()),
                work.as_ref(),
            )
            .await;
        }
        if mode == Mode::Revoked {
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            f.server.store.revoke(&admin, &context_key).unwrap();
            let until = tokio::time::Instant::now() + Duration::from_secs(5);
            while !matches!(session.status(), AssistantVapiStatus::Stopped { .. }) {
                assert!(
                    tokio::time::Instant::now() < until,
                    "revoked public context remained live"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        let result = session.shutdown().await;
        if mode == Mode::Ack {
            result.unwrap();
            observation_sources::verify_closed(
                f,
                &target.conversation_id,
                &target.session_id,
                &delivery,
            )
            .await;
        } else {
            assert!(result.is_err());
        }
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let (state,problem,reference,closed,end_unknown):(String,Option<String>,Option<String>,bool,bool)=c.query_row("SELECT d.state,p.problem,p.provider_ref,p.closed_at IS NOT NULL,p.end_uncertain FROM native_assistant_providers p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1",[&delivery],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        match mode {
            Mode::Ack | Mode::Revoked => {
                assert_eq!(state, "delivered");
                assert!(closed);
                assert_eq!(provider.ends.load(Ordering::SeqCst), 1);
            }
            Mode::Truncated => {
                assert_eq!(state, "failed");
                assert_eq!(problem.as_deref(), Some("truncated"));
                assert!(closed);
            }
            Mode::Missing => {
                assert_eq!(state, "uncertain");
                assert_eq!(problem.as_deref(), Some("unknown"));
                assert!(closed);
            }
            Mode::Rejected => {
                assert_eq!(state, "failed");
                assert_eq!(problem.as_deref(), Some("rejected"));
                assert!(reference.is_none());
                assert!(!closed);
            }
            Mode::LostCreation => {
                assert_eq!(state, "uncertain");
                assert!(reference.is_none());
                assert!(!closed);
            }
            Mode::ReusedReference => {
                assert_eq!(state, "uncertain");
                assert!(reference.is_none());
                assert!(!closed);
                assert_eq!(provider.exports.load(Ordering::SeqCst), 0);
                assert_eq!(provider.ends.load(Ordering::SeqCst), 0);
            }
            Mode::EndedBeforeAck => {
                assert_eq!(state, "uncertain");
                assert_eq!(problem.as_deref(), Some("incomplete"));
                assert!(closed);
                assert_eq!(provider.ends.load(Ordering::SeqCst), 0);
            }
            Mode::SocketLost => {
                assert_eq!(state, "uncertain");
                assert!(!closed);
                assert!(end_unknown);
            }
        }
        assert_eq!(provider.requests.load(Ordering::SeqCst), 1);
        assert!(provider.ends.load(Ordering::SeqCst) <= 1);
        drop(c);
        // Read the same observation through the public named HTTP API using a
        // current original owner credential, including after child revocation.
        let mut api =
            connect_client::application::ApplicationClient::open(canonical_http_config(f))
                .await
                .unwrap();
        let receipt_operation = connect_client::application::ApplicationOperation::try_from(
            connect_protocol::Operation::ContextDeliveries {
                conversation_id: selection(f, key, endpoint)
                    .await
                    .target()
                    .connection
                    .conversation_id
                    .clone(),
                before: None,
                limit: 100,
            },
        )
        .unwrap();
        let connect_protocol::Outcome::ContextDeliveries(page) =
            api.execute(receipt_operation).await.unwrap()
        else {
            panic!("HTTP context receipts")
        };
        let receipt = page.deliveries.iter().find(|d| d.id == delivery).unwrap();
        assert_eq!(receipt.state, state);
        assert_eq!(receipt.provider_ref, reference);
        let destination = receipt.destination.as_ref().unwrap();
        assert_eq!(destination.connection_id, "public-consultant");
        assert_eq!(destination.generation, 1);
        assert_eq!(
            destination.purpose,
            connect_protocol::ContextDeliveryPurpose::PublicSession
        );
        let expected = problem.as_deref().map(|p| match p {
            "rejected" => connect_protocol::ContextDeliveryProblem::Rejected,
            "truncated" => connect_protocol::ContextDeliveryProblem::Truncated,
            "incomplete" => connect_protocol::ContextDeliveryProblem::Incomplete,
            "unknown" => connect_protocol::ContextDeliveryProblem::Unknown,
            _ => panic!("unexpected problem"),
        });
        assert_eq!(receipt.problem, expected);
        let lifecycle = receipt.lifecycle.as_ref().unwrap();
        assert_eq!(lifecycle.delivery_confirmed, state == "delivered");
        assert_eq!(lifecycle.closed, closed);
        assert_eq!(lifecycle.cleanup_uncertain, Some(end_unknown));
        let json = serde_json::to_string(receipt).unwrap();
        for hidden in [
            "secret_ref",
            "context_credential_id",
            "selection_json",
            "public-fixture-token",
            "assistant-context-pair-only marker",
        ] {
            assert!(!json.contains(hidden), "private field in HTTP receipt");
        }

        let replay = AssistantVapiContextSession::start(
            f.server.service.clone(),
            operation,
            selection(f, key, endpoint).await,
            profile,
            secrets,
        )
        .await;
        if mode == Mode::Revoked {
            assert!(replay.is_err());
        } else {
            let mut replay = replay.unwrap();
            assert_eq!(replay.delivery_id(), delivery);
            assert!(
                tokio::time::timeout(Duration::from_secs(6), replay.context_ready())
                    .await
                    .unwrap()
                    .is_err()
            );
            assert!(replay.shutdown().await.is_err());
        }
        assert_eq!(
            provider.requests.load(Ordering::SeqCst),
            1,
            "replay created another public provider call"
        );
        assert!(provider.exports.load(Ordering::SeqCst) <= 1);
        assert!(
            selection(f, key, endpoint).await.current(),
            "public native source was retired by provider cleanup"
        );
        println!("actual public provider context: {mode:?}; durable={state}; exact native incarnation retained; replay made no second creation/export");
        server.abort();
        let _ = server.await;
    }
}
