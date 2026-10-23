//! Real HTTP/WS phase failures: inline admission and updates of an owned call.
use super::*;
use connect_server::assistant_vapi::*;
use futures_util::SinkExt;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    InlineRejected,
    InlineLost,
    InlineReused,
    UpdateTruncated,
    UpdateMissing,
    UpdateEnded,
    UpdateSocketLost,
    UpdateForeignAck,
    UpdateForeignEnd,
}
impl Mode {
    fn inline(self) -> bool {
        matches!(
            self,
            Self::InlineRejected | Self::InlineLost | Self::InlineReused
        )
    }
}
#[derive(Clone)]
struct PhaseProvider {
    base: String,
    reference: String,
    mode: Mode,
    requests: Arc<AtomicUsize>,
    exports: Arc<AtomicUsize>,
    sockets: Arc<AtomicUsize>,
    ends: Arc<AtomicUsize>,
}
fn history(message: &Value, update: bool) {
    assert_eq!(message["role"], "system");
    let raw = message["content"].as_str().unwrap();
    let view: connect_protocol::ContextView = serde_json::from_str(raw).unwrap();
    assert!(view
        .items
        .iter()
        .any(|i| i.body == "authorized provider history marker"));
    assert!(!raw.contains("assistant-context-pair-only marker"));
    assert!(!raw.contains("assistant-context-ai-only marker"));
    assert!(view.items.iter().all(|i| !i.source_id.is_empty()
        && !i.actor_id.is_empty()
        && i.sequence > 0
        && i.sequence <= view.watermark));
    if update {
        assert!(raw.contains("NATIVE-CONTEXT-PHASE-UPDATE"));
    }
}
async fn create_phase(
    State(s): State<PhaseProvider>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer phase-fixture-token"
    );
    assert_eq!(s.requests.fetch_add(1, Ordering::SeqCst), 0);
    assert_eq!(body["assistantOverrides"]["model"]["tools"], json!([]));
    assert_eq!(body["assistantOverrides"]["model"]["toolIds"], json!([]));
    let messages = body["assistantOverrides"]["model"]["messages"]
        .as_array()
        .unwrap();
    if s.mode.inline() {
        assert_eq!(messages.len(), 2);
        history(&messages[1], false);
        s.exports.fetch_add(1, Ordering::SeqCst);
    } else {
        assert_eq!(messages.len(), 1);
    }
    if s.mode == Mode::InlineRejected {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error":"designated initialization refusal"})),
        )
            .into_response();
    }
    if s.mode == Mode::InlineLost {
        return Json(json!({"id":s.reference,"transport":{}})).into_response();
    }
    Json(json!({"id":s.reference,"transport":{"websocketCallUrl":format!("{}/{}/transport",s.base.replace("http:","ws:"),s.reference)}})).into_response()
}
async fn upgrade_phase(
    State(s): State<PhaseProvider>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer phase-fixture-token"
    );
    s.sockets.fetch_add(1, Ordering::SeqCst);
    assert!(
        !s.mode.inline(),
        "failed/reused inline creation attached a socket"
    );
    upgrade.on_upgrade(move |ws| phase_socket(ws, s))
}
async fn phase_socket(mut socket: WebSocket, s: PhaseProvider) {
    while let Some(Ok(frame)) = socket.recv().await {
        match frame {
            Message::Text(text) => {
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
                let ordinal = s.exports.fetch_add(1, Ordering::SeqCst);
                assert!(ordinal < 2, "context was exported again on logical replay");
                history(&value["message"], ordinal == 1);
                assert_eq!(value["triggerResponseEnabled"], false);
                if ordinal == 0 {
                    socket.send(Message::Text(json!({"type":"conversation-update","messagesOpenAIFormatted":[value["message"].clone()]}).to_string().into())).await.unwrap();
                    continue;
                }
                match s.mode {
                    Mode::UpdateTruncated => {
                        let raw = value["message"]["content"].as_str().unwrap();
                        let partial = raw
                            .chars()
                            .take(raw.chars().count() / 2)
                            .collect::<String>();
                        socket.send(Message::Text(json!({"type":"conversation-update","messagesOpenAIFormatted":[{"role":"system","content":partial}]}).to_string().into())).await.unwrap();
                    }
                    Mode::UpdateMissing => {
                        socket
                            .send(Message::Text(
                                json!({"type":"status-update","status":"in-progress"})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    Mode::UpdateEnded => {
                        socket
                            .send(Message::Text(
                                json!({"type":"status-update","status":"ended"})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    Mode::UpdateForeignAck => {
                        socket.send(Message::Text(json!({"type":"conversation-update","callId":"another-call","messagesOpenAIFormatted":[value["message"].clone()]}).to_string().into())).await.unwrap();
                    }
                    Mode::UpdateForeignEnd => {
                        socket.send(Message::Text(json!({"type":"status-update","status":"ended","call":{"id":"another-call"}}).to_string().into())).await.unwrap();
                    }
                    Mode::UpdateSocketLost => {
                        let _ = socket.close().await;
                        return;
                    }
                    _ => panic!("inline failure unexpectedly exported over WebSocket"),
                }
            }
            Message::Close(frame) => {
                let _ = socket.send(Message::Close(frame)).await;
                return;
            }
            _ => {}
        }
    }
}
pub(super) async fn verify(f: &Fixture, key: &str, endpoint: &str, cid: &str) {
    for (index, mode) in [
        Mode::InlineRejected,
        Mode::InlineLost,
        Mode::InlineReused,
        Mode::UpdateTruncated,
        Mode::UpdateMissing,
        Mode::UpdateEnded,
        Mode::UpdateSocketLost,
        Mode::UpdateForeignAck,
        Mode::UpdateForeignEnd,
    ]
    .into_iter()
    .enumerate()
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let provider = PhaseProvider {
            base: base.clone(),
            reference: if mode == Mode::InlineReused {
                "public-context-call-0".into()
            } else {
                format!("phase-context-call-{index}")
            },
            mode,
            requests: Arc::new(AtomicUsize::new(0)),
            exports: Arc::new(AtomicUsize::new(0)),
            sockets: Arc::new(AtomicUsize::new(0)),
            ends: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/call", post(create_phase))
            .route(
                &format!("/{}/transport", provider.reference),
                get(upgrade_phase),
            )
            .with_state(provider.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let profile = AssistantVapiProfile {
            observations: None,
            api_base: format!("{base}/").parse().unwrap(),
            websocket_origin: format!("{}/", base.replace("http:", "ws:"))
                .parse()
                .unwrap(),
            loopback_fixture: true,
            model: "designated-phase-model".into(),
            application_prompt: "Use only the attributed authorized history.".into(),
            context_timing: if mode.inline() {
                AssistantProviderTiming::Initialization
            } else {
                AssistantProviderTiming::AfterCreation
            },
        };
        let secrets: ProviderSecrets = Arc::new(|reference| {
            assert_eq!(reference, "private_fixture_vapi_secret");
            connect_server::secrets::SecretValue::new("phase-fixture-token".into())
                .map_err(|_| connect_protocol::Error::forbidden())
        });
        let operation = format!("public-provider-phase-{index}");
        let mut job = AssistantVapiContextSession::start(
            f.server.service.clone(),
            operation.clone(),
            public_context::selection(f, key, endpoint).await,
            profile.clone(),
            secrets.clone(),
        )
        .await
        .unwrap();
        let parent = job.delivery_id().to_owned();
        let ready = tokio::time::timeout(Duration::from_secs(6), job.context_ready())
            .await
            .unwrap();
        if mode.inline() {
            assert!(ready.is_err());
        } else {
            ready.unwrap();
            let mut client =
                connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(f))
                    .await
                    .unwrap();
            client
                .send_message(
                    &format!("phase-source-{index}"),
                    cid,
                    &format!("phase-source-message-{index}"),
                    connect_client::messages::MessageDraft::text_all(format!(
                        "NATIVE-CONTEXT-PHASE-UPDATE-{index}"
                    )),
                )
                .await
                .unwrap();
            assert!(tokio::time::timeout(
                Duration::from_secs(7),
                job.update_context(&format!("phase-update-{index}"))
            )
            .await
            .unwrap()
            .is_err());
        }
        assert!(job.shutdown().await.is_err());
        let mut api =
            connect_client::application::ApplicationClient::open(canonical_http_config(f))
                .await
                .unwrap();
        let operation_read = connect_client::application::ApplicationOperation::try_from(
            connect_protocol::Operation::ContextDeliveries {
                conversation_id: cid.into(),
                before: None,
                limit: 100,
            },
        )
        .unwrap();
        let connect_protocol::Outcome::ContextDeliveries(page) =
            api.execute(operation_read).await.unwrap()
        else {
            panic!()
        };
        let original = page.deliveries.iter().find(|d| d.id == parent).unwrap();
        if mode.inline() {
            assert_eq!(
                original.state,
                if mode == Mode::InlineRejected {
                    "failed"
                } else {
                    "uncertain"
                }
            );
            assert_eq!(
                original.problem,
                Some(if mode == Mode::InlineRejected {
                    connect_protocol::ContextDeliveryProblem::Rejected
                } else {
                    connect_protocol::ContextDeliveryProblem::Unknown
                })
            );
            assert!(original.provider_ref.is_none());
            let lifecycle = original.lifecycle.as_ref().unwrap();
            assert_eq!(
                lifecycle.phase,
                connect_protocol::ContextDeliveryPhase::Initialization
            );
            assert!(
                lifecycle.creation_admitted
                    && lifecycle.export_admitted
                    && !lifecycle.creation_confirmed
                    && !lifecycle.delivery_confirmed
            );
            assert_eq!(provider.sockets.load(Ordering::SeqCst), 0);
            assert_eq!(provider.exports.load(Ordering::SeqCst), 1);
            assert_eq!(provider.ends.load(Ordering::SeqCst), 0);
        } else {
            assert_eq!(original.state, "delivered");
            assert!(original.problem.is_none());
            let updated = page
                .deliveries
                .iter()
                .find(|d| {
                    d.lifecycle
                        .as_ref()
                        .is_some_and(|l| l.parent_delivery_id.as_deref() == Some(parent.as_str()))
                })
                .unwrap();
            let lifecycle = updated.lifecycle.as_ref().unwrap();
            assert_eq!(
                lifecycle.phase,
                connect_protocol::ContextDeliveryPhase::Update
            );
            assert!(
                !lifecycle.creation_admitted
                    && lifecycle.creation_confirmed
                    && lifecycle.export_admitted
                    && !lifecycle.delivery_confirmed
            );
            assert_eq!(updated.destination, original.destination);
            assert_eq!(updated.provider_ref, original.provider_ref);
            assert_eq!(
                updated.state,
                if mode == Mode::UpdateTruncated {
                    "failed"
                } else {
                    "uncertain"
                }
            );
            assert_eq!(
                updated.problem,
                Some(match mode {
                    Mode::UpdateTruncated => connect_protocol::ContextDeliveryProblem::Truncated,
                    Mode::UpdateEnded => connect_protocol::ContextDeliveryProblem::Incomplete,
                    _ => connect_protocol::ContextDeliveryProblem::Unknown,
                })
            );
            assert_eq!(lifecycle.closed, mode != Mode::UpdateSocketLost);
            assert_eq!(provider.exports.load(Ordering::SeqCst), 2);
            assert_eq!(
                provider.ends.load(Ordering::SeqCst),
                usize::from(!matches!(mode, Mode::UpdateEnded | Mode::UpdateSocketLost))
            );
        }
        let mut replay = AssistantVapiContextSession::start(
            f.server.service.clone(),
            operation,
            public_context::selection(f, key, endpoint).await,
            profile,
            secrets,
        )
        .await
        .unwrap();
        assert_eq!(replay.delivery_id(), parent);
        assert!(
            tokio::time::timeout(Duration::from_secs(6), replay.context_ready())
                .await
                .unwrap()
                .is_err()
        );
        assert!(replay.shutdown().await.is_err());
        assert_eq!(provider.requests.load(Ordering::SeqCst), 1);
        assert!(public_context::selection(f, key, endpoint).await.current());
        println!("actual native context phase: {mode:?}; inline/child receipt and original native generation retained; logical replay sent no creation or context");
        server.abort();
        let _ = server.await;
    }
}
