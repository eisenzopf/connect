//! Current native SDK communication composes with explicit application Task APIs.
//! Provider requests use only the bounded local fake endpoint, never an account.
use super::*;
use connect_client::application::{ApplicationClient, ApplicationClientConfig};
use connect_client::tasks::*;
use connect_connectors::Vapi;
use connect_server::providers::Worker;
use connect_store::{ProviderMessageEndpoint, Route};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};
const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";
const FACT: &str = "The current SDK context marker is cedar.";
struct Setup {
    f: Fixture,
    client: Client,
    room: sdk::Conversation,
    application: ApplicationClientConfig,
    root: String,
    ai: String,
    routes: Vec<ProviderMessageEndpoint>,
}
async fn application(config: &ApplicationClientConfig) -> ApplicationClient {
    ApplicationClient::open(config.clone()).await.unwrap()
}
async fn task(client: &mut ApplicationClient, command: TaskCommand) -> TaskOutcome {
    match client
        .execute(
            Operation::Task {
                request: Box::new(TaskRequest::new(command.clone())),
            }
            .try_into()
            .unwrap(),
        )
        .await
        .unwrap_or_else(|error| panic!("Task command {command:?} rejected: {error:?}"))
    {
        Outcome::Task(value) => *value,
        _ => panic!("typed Task result"),
    }
}
async fn access(config: &ApplicationClientConfig, command: TaskAccessCommand) -> TaskAccessOutcome {
    let config = config.clone();
    let http = tokio::task::spawn_blocking(|| {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let response: TaskAccessResponse = http
        .post(format!("http://{}/v1/task-access", config.endpoint))
        .bearer_auth(config.credential)
        .json(&TaskAccessRequest::new(command))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    response.result.unwrap()
}
async fn create_task(
    setup: &Setup,
    id: &str,
    permissions: Vec<TaskPermission>,
) -> (TaskRecord, ProtectedTaskGrant) {
    let mut app = application(&setup.application).await;
    app.negotiate_tasks("connect:sdk-task-ai@local", Some(setup.room.id().as_str()))
        .await
        .unwrap();
    let TaskOutcome::Record(record) = task(
        &mut app,
        TaskCommand::Create {
            request_id: id.into(),
            definition: TaskDefinition {
                conversation_id: setup.room.id().as_str().into(),
                scope: TaskScope {
                    operations: vec![
                        TaskPermission::Send,
                        TaskPermission::Inbox,
                        TaskPermission::History,
                    ],
                    recipients: vec![setup.ai.clone()],
                    sms_enrollments: vec![],
                },
                expires_at: chrono::Utc::now().timestamp() + 600,
                initiating_event_id: None,
            },
        },
    )
    .await
    else {
        panic!("Task record")
    };
    let TaskAccessOutcome::Issued(grant) = access(
        &setup.application,
        TaskAccessCommand::Issue {
            request_id: format!("{id}-grant"),
            definition: GrantDefinition {
                task_id: record.id.clone(),
                binding: InvocationBinding {
                    kind: InvocationKind::Native,
                    adapter: "current-sdk-task-fixture".into(),
                    external_id: id.into(),
                },
                operations: permissions,
                expires_at: chrono::Utc::now().timestamp() + 300,
            },
        },
    )
    .await
    else {
        panic!("issued grant")
    };
    (*record, grant)
}
async fn invocation(
    setup: &Setup,
    record: &TaskRecord,
    grant: &ProtectedTaskGrant,
) -> ApplicationClient {
    let mut app = application(&setup.application.clone().with_task_grant(grant).unwrap()).await;
    assert_eq!(app.negotiate_task(&record.id).await.unwrap().id, record.id);
    app
}

#[tokio::test]
async fn canonical_application_stdio_task_grant_keeps_exact_bootstrap_permissions_and_current_revocation(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let setup = setup(algorithm).await;
        let (record, grant) = create_task(&setup, "stdio-task", vec![TaskPermission::Inbox]).await;
        let (other, _) = create_task(&setup, "stdio-other-task", vec![TaskPermission::Inbox]).await;
        let config = setup.f.temp.path().join("stdio-application.toml");
        let installed = setup.application.clone().with_task_grant(&grant).unwrap();
        init::private_write(&config, toml::to_string(&installed).unwrap().as_bytes()).unwrap();
        let mut helper = crate::canonical_stdio::Process::start(&config, true);
        let get = |id: &str, binding: &str| json!({"id":"task-get","operation":{"op":"task","request":{"task_profile":"connect-task/1","command":{"action":"get","task_id":id}}},"task_binding":binding});
        assert_eq!(
            helper.exchange(get(&record.id, &other.id)).await["Err"]["code"],
            "invalid-request"
        );
        assert!(helper
            .exchange(get(&other.id, &other.id))
            .await
            .get("Err")
            .is_some());
        let read = helper.exchange(get(&record.id, &record.id)).await;
        assert_eq!(read["Ok"]["kind"], "task");
        assert_eq!(read["Ok"]["data"]["data"]["id"], record.id);
        let recall = json!({"id":"task-recall","operation":{"op":"task","request":{"task_profile":"connect-task/1","command":{"action":"recall","task_id":record.id,"after":null,"limit":100}}}});
        assert_eq!(helper.exchange(recall).await["Err"]["code"], "forbidden");
        assert_eq!(helper.exchange(json!({"id":"old-send","operation":Operation::Send {
            request_id:"no-fallback".into(), conversation_id:setup.room.id().as_str().into(),
            message:connect_client::MessageDraft::text_all("no authority fallback")
        }})).await["Err"]["code"],"canonical-command-required");
        assert!(helper
            .exchange(get(&record.id, &record.id))
            .await
            .get("Ok")
            .is_some());
        // A later owner configuration cannot replace the installed grant.
        std::fs::write(&config, toml::to_string(&setup.application).unwrap()).unwrap();
        access(
            &setup.application,
            TaskAccessCommand::Revoke {
                grant_id: grant.grant.id.clone(),
            },
        )
        .await;
        assert!(helper
            .exchange(get(&record.id, &record.id))
            .await
            .get("Err")
            .is_some());
        helper
            .finish(
                true,
                &[grant.expose_secret(), &setup.application.credential],
            )
            .await;
        let mut owner = application(&setup.application).await;
        assert_eq!(
            owner.negotiate_task(&record.id).await.unwrap().id,
            record.id
        );
        setup.client.close(options()).await.unwrap();
        setup.f.server.shutdown().await.unwrap();
    }
}
fn question(record: &TaskRecord, id: &str, body: &str) -> TaskCommand {
    TaskCommand::Invoke {
        request_id: id.into(),
        task_id: record.id.clone(),
        step_id: id.into(),
        operation: Box::new(Operation::Send {
            request_id: id.into(),
            conversation_id: record.conversation_id.clone(),
            message: connect_client::MessageDraft::text_all(body),
        }),
        reply_to: None,
        wait_for_reply: true,
    }
}
async fn jobs(app: &mut ApplicationClient, cid: &str) -> Vec<connect_protocol::ProviderDelivery> {
    let Outcome::ProviderStatus { deliveries } = app
        .execute(
            Operation::ProviderStatus {
                conversation_id: cid.into(),
            }
            .try_into()
            .unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("provider status")
    };
    deliveries
}
async fn history(room: &sdk::Conversation, client: &Client) -> Vec<Value> {
    let raw = client.raw();
    let frames = raw.request(raw.validate(json!({"v":2,"id":"caller-attempt","ts":chrono::Utc::now(),"type":"message.history","cid":room.id(),"payload":{"limit":100}})).unwrap(), options()).await.unwrap();
    frames
        .into_iter()
        .filter(|frame| frame.kind() == "message.send")
        .map(|frame| frame.to_json()["payload"].clone())
        .collect()
}

async fn setup(algorithm: &str) -> Setup {
    let f = Fixture::with_native_end_scope(algorithm, 16).await;
    let root = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Identity {
        identity_id: ai, ..
    } = f
        .server
        .store
        .execute_provisioning(
            &root,
            ProvisioningCommand::Identity {
                operation_id: "sdk-task-ai".into(),
                alias: "sdk-task-ai".into(),
                kind: IdentityKind::Ai,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!("provisioned AI")
    };

    let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
        .await
        .unwrap();
    let room = client
        .create_conversation(
            vec![ai.parse().unwrap()],
            ConversationPolicy::Persistent,
            mutation("sdk-task-room"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let target = room
        .refresh(options())
        .await
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id.as_str() == ai)
        .unwrap()
        .id;
    room.send(
        "sdk-task-context-message".parse().unwrap(),
        NonEmpty {
            first: target,
            rest: vec![],
        },
        FACT.into(),
        mutation("sdk-task-context-send"),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap();

    let owner = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let delegate = f
        .server
        .store
        .issue(
            &ai,
            Scope::messaging(vec![f.client.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.server
        .store
        .configure_route(
            &root,
            &Route {
                id: "sdk-task-provider".into(),
                identity_id: ai.clone(),
                credential_id: delegate.id,
                conversation_id: room.id().as_str().into(),
                connector: "vapi".into(),
                target: ASSISTANT.into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &root,
            ProvisioningCommand::Endpoint {
                operation_id: "sdk-task-endpoint".into(),
                identity_id: ai.clone(),
                device: DeviceSpec {
                    id: "sdk-task-provider-device".into(),
                    kind: "service".into(),
                    platform: "isolated-task-fixture".into(),
                },
                transport: EndpointTransport::VapiChat,
                address: ASSISTANT.into(),
                capabilities: vec!["messaging".into()],
                secret_ref: Some("SDK_TASK_VAPI".into()),
            },
        )
        .unwrap()
    else {
        panic!("provider endpoint")
    };
    f.server
        .store
        .execute_provisioning(
            &root,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "sdk-task-verify".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "bounded-local-task-fixture".into(),
            },
        )
        .unwrap();
    let routes = vec![ProviderMessageEndpoint {
        route_id: "sdk-task-provider".into(),
        connector: "vapi".into(),
        endpoint_id,
        source_endpoint_id: None,
        account_credential_id: root.clone(),
        secret_reference: "SDK_TASK_VAPI".into(),
    }];
    let application = ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: owner.token,
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 5000,
    };
    Setup {
        f,
        client,
        room,
        application,
        root,
        ai,
        routes,
    }
}
async fn start_worker(setup: &Setup, driver: Arc<Vapi>) -> Worker {
    Worker::with_endpoint_routes(
        setup.f.server.service.clone(),
        vec![driver],
        setup.root.clone(),
        setup.routes.clone(),
        None,
        false,
    )
    .await
    .unwrap()
}
async fn reply(app: &mut ApplicationClient, record: &TaskRecord) -> TaskReply {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut after = None;
        loop {
            let TaskOutcome::Inbox { events, next } = task(
                app,
                TaskCommand::Wait {
                    task_id: record.id.clone(),
                    after: after.clone(),
                    limit: 100,
                    wait_ms: 300,
                },
            )
            .await
            else {
                panic!("inbox")
            };
            for event in events {
                if let TaskEventData::Reply(reply) = event.data {
                    return reply;
                }
            }
            after = next;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn public_sdk_task_provider_context_grants_and_replies_use_current_communication_without_legacy_dispatch(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let mock = MockServer::start().await;
        Mock::given(method("GET")).and(path(format!("/assistant/{ASSISTANT}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ASSISTANT,"model":{"messages":[{"role":"system","content":"History: {{connect_context}}"}]}}))).mount(&mock).await;
        for (input, id, body) in [
            (
                "Restricted task question",
                "restricted",
                "Restricted answer",
            ),
            ("Recall task question", "recall", "Recall answer"),
        ] {
            Mock::given(method("POST"))
                .and(path("/chat"))
                .and(body_partial_json(json!({"input":input})))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(
                        json!({"id":id,"output":[{"role":"assistant","content":body}]}),
                    ),
                )
                .expect(1)
                .mount(&mock)
                .await;
        }
        let driver = Arc::new(
            Vapi::with_reference("isolated-fixture", &mock.uri(), "SDK_TASK_VAPI").unwrap(),
        );
        let setup = setup(algorithm).await;
        let (restricted, narrow) = create_task(
            &setup,
            "sdk-restricted",
            vec![TaskPermission::Send, TaskPermission::Inbox],
        )
        .await;
        let (recall, full) = create_task(
            &setup,
            "sdk-recall",
            vec![
                TaskPermission::Send,
                TaskPermission::Inbox,
                TaskPermission::History,
            ],
        )
        .await;
        let mut a = invocation(&setup, &restricted, &narrow).await;
        let mut b = invocation(&setup, &recall, &full).await;
        let worker = start_worker(&setup, driver).await;
        let qa = question(
            &restricted,
            "sdk-ask-restricted",
            "Restricted task question",
        );
        let qb = question(&recall, "sdk-ask-recall", "Recall task question");
        for (app, request) in [(&mut a, &qa), (&mut b, &qb)] {
            assert!(matches!(
                task(app, request.clone()).await,
                TaskOutcome::Tool(
                    TaskToolResult::Pending { .. } | TaskToolResult::Completed { .. }
                )
            ));
        }
        let ra = reply(&mut a, &restricted).await;
        let rb = reply(&mut b, &recall).await;
        assert_eq!(ra.task_id.as_deref(), Some(restricted.id.as_str()));
        assert_eq!(rb.task_id.as_deref(), Some(recall.id.as_str()));
        assert_ne!(ra.event_id, rb.event_id);
        for (app, request, observed) in [(&mut a, &qa, &ra), (&mut b, &qb, &rb)] {
            let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) =
                task(app, request.clone()).await
            else {
                panic!("recorded completion")
            };
            let Outcome::Accepted { message_id, .. } = *outcome else {
                panic!("send acceptance")
            };
            assert_eq!(observed.reply_to.as_deref(), Some(message_id.as_str()));
        }
        let requests = mock.received_requests().await.unwrap();
        let bodies: Vec<Value> = requests
            .iter()
            .filter(|r| r.method == "POST")
            .map(|r| r.body_json().unwrap())
            .collect();
        assert_eq!(bodies.len(), 2);
        let narrow_body = bodies
            .iter()
            .find(|v| v["input"] == "Restricted task question")
            .unwrap();
        assert!(
            narrow_body.get("assistantOverrides").is_none()
                && narrow_body.get("previousChatId").is_none()
        );
        let recall_body = bodies
            .iter()
            .find(|v| v["input"] == "Recall task question")
            .unwrap();
        assert!(recall_body.get("previousChatId").is_none());
        let context = recall_body["assistantOverrides"]["variableValues"]["connect_context"]
            .as_str()
            .unwrap();
        assert!(context.contains(FACT) && context.contains("conversation-journal:"));
        let denied = a
            .execute(
                Operation::Task {
                    request: Box::new(TaskRequest::new(TaskCommand::Recall {
                        task_id: restricted.id.clone(),
                        after: None,
                        limit: 100,
                    })),
                }
                .try_into()
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert!(matches!(denied,ClientError::Application(error) if error.code=="forbidden"));
        let TaskOutcome::Recall { sources, .. } = task(
            &mut b,
            TaskCommand::Recall {
                task_id: recall.id.clone(),
                after: None,
                limit: 100,
            },
        )
        .await
        else {
            panic!("recall")
        };
        assert!(sources.iter().any(|s| s.body.as_deref() == Some(FACT)));
        let mut owner = application(&setup.application).await;
        let delivered = jobs(&mut owner, setup.room.id().as_str()).await;
        assert_eq!(delivered.len(), 2);
        assert!(delivered.iter().all(|j| j.state == "completed"));
        assert_eq!(history(&setup.room, &setup.client).await.len(), 5);
        worker.shutdown().await.unwrap();
        drop((a, b, owner));
        setup.client.close(options()).await.unwrap();
        setup.f.server.shutdown().await.unwrap();
    }
}
struct AcceptWithoutReceipt(Arc<AtomicUsize>);
impl Respond for AcceptWithoutReceipt {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        assert_eq!(
            request.body_json::<Value>().unwrap()["input"],
            "Accept once without a receipt"
        );
        self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200)
            .set_body_json(json!({"id":"accepted-once","output":[{"content":"Accepted"}]}))
            .set_delay(Duration::from_secs(60))
    }
}
async fn uncertain(app: &mut ApplicationClient, request: &TaskCommand) -> String {
    let TaskOutcome::Tool(TaskToolResult::Uncertain { effect_id, .. }) =
        task(app, request.clone()).await
    else {
        panic!("uncertain original effect")
    };
    effect_id
}
#[tokio::test]
async fn public_sdk_task_provider_uncertainty_survives_restart_without_redispatch_and_rechecks_revocation(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let mock = MockServer::start().await;
        let accepted = Arc::new(AtomicUsize::new(0));
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(AcceptWithoutReceipt(accepted.clone()))
            .expect(1)
            .mount(&mock)
            .await;
        let driver = Arc::new(
            Vapi::with_reference("isolated-fixture", &mock.uri(), "SDK_TASK_VAPI").unwrap(),
        );
        let mut setup = setup(algorithm).await;
        let (record, grant) =
            create_task(&setup, "sdk-uncertain", vec![TaskPermission::Send]).await;
        let mut app = invocation(&setup, &record, &grant).await;
        let worker = start_worker(&setup, driver.clone()).await;
        let request = question(
            &record,
            "sdk-uncertain-send",
            "Accept once without a receipt",
        );
        assert!(matches!(
            task(&mut app, request.clone()).await,
            TaskOutcome::Tool(TaskToolResult::Pending { .. } | TaskToolResult::Uncertain { .. })
        ));
        tokio::time::timeout(Duration::from_secs(10), async {
            while accepted.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        worker.shutdown().await.unwrap();
        let original = uncertain(&mut app, &request).await;
        drop(app);
        setup.client.close(options()).await.unwrap();
        setup.f.server.shutdown().await.unwrap();
        setup.f.server = RunningCanonicalServer::start(setup.f.config.clone())
            .await
            .unwrap();
        setup.f.client.endpoint = setup.f.server.address;
        setup.application.endpoint = setup.f.server.http_address.unwrap();
        setup.client = Client::connect(setup.f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        setup.room = setup
            .client
            .conversation_by_id(record.conversation_id.parse().unwrap());
        let mut app = invocation(&setup, &record, &grant).await;
        let worker = start_worker(&setup, driver).await;
        assert_eq!(uncertain(&mut app, &request).await, original);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(uncertain(&mut app, &request).await, original);
        let mut owner = application(&setup.application).await;
        owner.negotiate_task(&record.id).await.unwrap();
        let TaskOutcome::Effect(effect) = task(
            &mut owner,
            TaskCommand::EffectGet {
                task_id: record.id.clone(),
                effect_id: original.clone(),
            },
        )
        .await
        else {
            panic!("retained effect")
        };
        assert_eq!(effect.state, EffectState::Uncertain);
        let denied = app
            .execute(
                Operation::Task {
                    request: Box::new(TaskRequest::new(TaskCommand::EffectGet {
                        task_id: record.id.clone(),
                        effect_id: original,
                    })),
                }
                .try_into()
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert!(matches!(denied,ClientError::Application(error) if error.code=="forbidden"));
        assert_eq!(history(&setup.room, &setup.client).await.len(), 2);
        let deliveries = jobs(&mut owner, setup.room.id().as_str()).await;
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].state, "uncertain");
        assert!(matches!(
            access(
                &setup.application,
                TaskAccessCommand::Revoke {
                    grant_id: grant.grant.id.clone()
                }
            )
            .await,
            TaskAccessOutcome::Revoked { .. }
        ));
        let denied = app
            .execute(
                Operation::Task {
                    request: Box::new(TaskRequest::new(request)),
                }
                .try_into()
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert!(matches!(denied,ClientError::Application(error) if error.code=="unauthorized"));
        worker.shutdown().await.unwrap();
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
        drop((app, owner));
        setup.client.close(options()).await.unwrap();
        setup.f.server.shutdown().await.unwrap();
    }
}
