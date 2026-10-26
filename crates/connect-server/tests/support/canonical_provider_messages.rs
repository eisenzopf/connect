use super::*;
use connect_connector_api::Connector;
use connect_server::canonical_messages::{
    CanonicalMessagesConfig, MessageAccount, TelnyxCallbacks,
};
use connect_store::{ProviderMessageEndpoint, Route};
use ed25519_dalek::{Signer, SigningKey};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};
const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";
struct Prepared {
    _temp: tempfile::TempDir,
    config: CanonicalServerConfig,
    client: CanonicalClientConfig,
    root: String,
    administrator: String,
    other_cid: Option<String>,
    cid: String,
    ai_pid: String,
    observer: String,
    endpoints: Vec<ProviderMessageEndpoint>,
}
fn endpoint(
    f: &Fixture,
    root: &str,
    identity: &str,
    name: &str,
    transport: EndpointTransport,
    address: &str,
    secret: &str,
) -> String {
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            root,
            ProvisioningCommand::Endpoint {
                operation_id: format!("enroll-{name}"),
                identity_id: identity.into(),
                device: DeviceSpec {
                    id: format!("{name}-device"),
                    kind: "service".into(),
                    platform: "provider-message-fixture".into(),
                },
                transport,
                address: address.into(),
                capabilities: vec!["messaging".into()],
                secret_ref: Some(secret.into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            root,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: format!("verify-{name}"),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "isolated-fixture".into(),
            },
        )
        .unwrap();
    endpoint_id
}
async fn prepare(sms: bool) -> Prepared {
    prepare_inner(sms, false).await
}
async fn prepare_inner(sms: bool, overlap: bool) -> Prepared {
    let f = Fixture::with_membership_scope("EdDSA", 32, true).await;
    let store = &f.server.store;
    let root = store.authenticate(&f.admin).unwrap();
    let ProvisioningOutcome::Identity {
        identity_id: ai, ..
    } = store
        .execute_provisioning(
            &root.credential_id,
            ProvisioningCommand::Identity {
                operation_id: "message-ai".into(),
                alias: "message-ai".into(),
                kind: IdentityKind::Ai,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let key = store
        .issue(&ai, Scope::operator(), chrono::Utc::now().timestamp() + 600)
        .unwrap();
    let alice = store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let observer = store
        .issue(
            &f.peer.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut create = connect_client::conversations::CreateRequest::persistent(vec![
        ai.clone(),
        f.peer.identity_id.clone(),
    ]);
    create.tenant_id = Some("tnt_default".into());
    let room = native
        .create_conversation("provider-room", create)
        .await
        .unwrap();
    let primer = store.membership_snapshot(&alice.id, &room.id).unwrap();
    let primer_pid = primer
        .participants
        .iter()
        .find(|m| m.identity_id == ai)
        .unwrap()
        .participant_id
        .clone();
    native
        .send_message(
            "primer",
            &room.id,
            "primer-msg",
            connect_client::MessageDraft::text(
                connect_protocol::messages::MessageAudience::Participants(vec![primer_pid]),
                "allowed-context-primer",
            ),
        )
        .await
        .unwrap();
    let other_room = if overlap {
        let mut create = connect_client::conversations::CreateRequest::persistent(vec![
            ai.clone(),
            f.peer.identity_id.clone(),
        ]);
        create.tenant_id = Some("tnt_default".into());
        Some(
            native
                .create_conversation("provider-overlap-room", create)
                .await
                .unwrap(),
        )
    } else {
        None
    };
    native.close();
    let members = store.membership_snapshot(&alice.id, &room.id).unwrap();
    let ai_pid = members
        .participants
        .iter()
        .find(|m| m.identity_id == ai)
        .unwrap()
        .participant_id
        .clone();
    store
        .configure_route(
            &root.credential_id,
            &Route {
                id: "canonical-vapi".into(),
                identity_id: ai.clone(),
                credential_id: key.id,
                conversation_id: room.id.clone(),
                connector: "vapi".into(),
                target: ASSISTANT.into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let target = endpoint(
        &f,
        &root.credential_id,
        &ai,
        "chat",
        EndpointTransport::VapiChat,
        ASSISTANT,
        "MESSAGE_VAPI",
    );
    let mut endpoints = vec![ProviderMessageEndpoint {
        route_id: "canonical-vapi".into(),
        connector: "vapi".into(),
        transport: connect_protocol::provisioning::EndpointTransport::VapiChat,
        endpoint_id: target,
        source_endpoint_id: None,
        account_credential_id: root.credential_id.clone(),
        secret_reference: "MESSAGE_VAPI".into(),
    }];
    if sms {
        store
            .configure_route(
                &root.credential_id,
                &Route {
                    id: "canonical-sms".into(),
                    identity_id: f.client.identity_id.clone(),
                    credential_id: alice.id.clone(),
                    conversation_id: room.id.clone(),
                    connector: "telnyx".into(),
                    target: "+15555550101".into(),
                    source: Some("+15555550102".into()),
                    enabled: true,
                    incoming_to: connect_protocol::messages::MessageAudience::Participants(vec![
                        ai_pid.clone(),
                    ]),
                },
            )
            .unwrap();
        let target = endpoint(
            &f,
            &root.credential_id,
            &f.client.identity_id,
            "sms-contact",
            EndpointTransport::Sms,
            "+15555550101",
            "MESSAGE_SMS",
        );
        let source = endpoint(
            &f,
            &root.credential_id,
            &root.identity_id,
            "sms-source",
            EndpointTransport::Sms,
            "+15555550102",
            "MESSAGE_SMS",
        );
        endpoints.push(ProviderMessageEndpoint {
            route_id: "canonical-sms".into(),
            connector: "telnyx".into(),
            transport: connect_protocol::provisioning::EndpointTransport::Sms,
            endpoint_id: target.clone(),
            source_endpoint_id: Some(source.clone()),
            account_credential_id: root.credential_id.clone(),
            secret_reference: "MESSAGE_SMS".into(),
        });
        if let Some(other) = &other_room {
            let members = store.membership_snapshot(&alice.id, &other.id).unwrap();
            let pid = members
                .participants
                .iter()
                .find(|p| p.identity_id == ai)
                .unwrap()
                .participant_id
                .clone();
            store
                .configure_route(
                    &root.credential_id,
                    &Route {
                        id: "overlapping-sms".into(),
                        identity_id: f.client.identity_id.clone(),
                        credential_id: alice.id.clone(),
                        conversation_id: other.id.clone(),
                        connector: "telnyx".into(),
                        target: "+15555550101".into(),
                        source: Some("+15555550102".into()),
                        enabled: true,
                        incoming_to: connect_protocol::messages::MessageAudience::Participants(
                            vec![pid],
                        ),
                    },
                )
                .unwrap();
            endpoints.push(ProviderMessageEndpoint {
                route_id: "overlapping-sms".into(),
                connector: "telnyx".into(),
                transport: connect_protocol::provisioning::EndpointTransport::Sms,
                endpoint_id: target,
                source_endpoint_id: Some(source),
                account_credential_id: root.credential_id.clone(),
                secret_reference: "MESSAGE_SMS".into(),
            });
        }
        store
            .refresh_sms_grants(
                "fixture",
                &connect_store::SmsGrantSnapshot {
                    revision: 1,
                    grants: std::iter::once(&room)
                        .chain(other_room.iter())
                        .enumerate()
                        .map(|(index, room)| connect_store::SmsGrant {
                            id: format!("reviewed-{index}"),
                            phone: "+15555550101".into(),
                            sender: "+15555550102".into(),
                            conversation_id: room.id.clone(),
                            purpose: "Requested local test".into(),
                            consent_id: "reviewed-form".into(),
                            disclosure_version: "v1".into(),
                            disclosure_text: "Explicit fixture consent".into(),
                            submitted_at: chrono::Utc::now().timestamp() - 400,
                            approved_at: chrono::Utc::now().timestamp() - 300,
                            expires_at: chrono::Utc::now().timestamp() + 3600,
                        })
                        .collect(),
                },
            )
            .unwrap();
    }
    let administrator = f.admin.clone();
    let Fixture {
        temp,
        config,
        server,
        client,
        ..
    } = f;
    server.shutdown().await.unwrap();
    Prepared {
        _temp: temp,
        config,
        client,
        root: root.credential_id,
        administrator,
        other_cid: other_room.map(|room| room.id),
        cid: room.id,
        ai_pid,
        observer: observer.token,
        endpoints,
    }
}
impl Prepared {
    async fn start(
        &mut self,
        drivers: Vec<Arc<dyn Connector>>,
        callbacks: Option<TelnyxCallbacks>,
    ) -> RunningCanonicalServer {
        self.config.provider_messages = Some(CanonicalMessagesConfig {
            operator_credential_id: self.root.clone(),
            accounts: if self
                .endpoints
                .iter()
                .any(|endpoint| endpoint.connector == "telnyx")
            {
                vec![
                    MessageAccount {
                        connector: "vapi".into(),
                        secret_reference: "MESSAGE_VAPI".into(),
                    },
                    MessageAccount {
                        connector: "telnyx".into(),
                        secret_reference: "MESSAGE_SMS".into(),
                    },
                ]
            } else {
                vec![MessageAccount {
                    connector: "vapi".into(),
                    secret_reference: "MESSAGE_VAPI".into(),
                }]
            },
            endpoints: self.endpoints.clone(),
            telnyx_callbacks: callbacks,
        });
        let server = RunningCanonicalServer::start_with_message_drivers(
            self.config.clone(),
            Some(drivers),
            None,
        )
        .await
        .unwrap();
        self.client.endpoint = server.address;
        server
    }
    fn http(
        &self,
        server: &RunningCanonicalServer,
        token: &str,
    ) -> connect_client::application::ApplicationClientConfig {
        connect_client::application::ApplicationClientConfig {
            endpoint: server.http_address.unwrap(),
            credential: token.into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            timeout_ms: 10000,
        }
    }
}
async fn assistant(mock: &MockServer) {
    Mock::given(method("GET")).and(header("authorization","Bearer fake-vapi")).and(path(format!("/assistant/{ASSISTANT}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ASSISTANT,"model":{"messages":[{"role":"system","content":"Context: {{connect_context}}"}]}}))).mount(mock).await;
}
async fn wait_messages(native: &mut CanonicalClient, cid: &str, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let page = native
            .history(cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        if page.messages.len() >= n {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing provider messages"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn callback(
    url: &str,
    key: &SigningKey,
    value: &Value,
    stamp: i64,
    tampered: bool,
) -> reqwest::Response {
    let raw = serde_json::to_vec(value).unwrap();
    let ts = stamp.to_string();
    let sig = key.sign(&[ts.as_bytes(), b"|", &raw].concat());
    reqwest::Client::new()
        .post(url)
        .header("telnyx-signature-ed25519", STANDARD.encode(sig.to_bytes()))
        .header("telnyx-timestamp", ts)
        .body(if tampered { b"{}".to_vec() } else { raw })
        .send()
        .await
        .unwrap()
}
fn incoming(id: &str, text: &str) -> Value {
    json!({"data":{"id":id,"event_type":"message.received","payload":{"id":format!("resource-{id}"),"from":{"phone_number":"+15555550101"},"to":[{"phone_number":"+15555550102"}],"text":text}}})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_sms_overlap_is_private_until_public_admin_selection_and_survives_restart() {
    use connect_client::application::ApplicationClient;
    use connect_protocol::incoming_contacts::*;
    let mut p = prepare_inner(true, true).await;
    let other = p.other_cid.clone().unwrap();
    let mock = MockServer::start().await;
    assistant(&mock).await;
    let chats = Arc::new(AtomicUsize::new(0));
    let sent = Arc::new(AtomicUsize::new(0));
    let calls = chats.clone();
    Mock::given(method("POST")).and(path("/chat")).respond_with(move|_:&wiremock::Request|{calls.fetch_add(1,Ordering::SeqCst);ResponseTemplate::new(200).set_body_json(json!({"id":"unexpected-chat","output":[{"role":"assistant","content":"unexpected"}]}))}).mount(&mock).await;
    let calls = sent.clone();
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(move |_: &wiremock::Request| {
            calls.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"unexpected-message"}}))
        })
        .mount(&mock)
        .await;
    let drivers: Vec<Arc<dyn Connector>> = vec![
        Arc::new(
            connect_connectors::Vapi::with_reference("fake-vapi", &mock.uri(), "MESSAGE_VAPI")
                .unwrap(),
        ),
        Arc::new(
            connect_connectors::Telnyx::with_reference("fake-sms", &mock.uri(), "MESSAGE_SMS")
                .unwrap(),
        ),
    ];
    let key = SigningKey::from_bytes(&[17; 32]);
    let callbacks = TelnyxCallbacks {
        bind: "127.0.0.1:0".parse().unwrap(),
        verification_key: STANDARD.encode(key.verifying_key().to_bytes()),
    };
    let mut server = p.start(drivers.clone(), Some(callbacks.clone())).await;
    let mut webhook = format!(
        "http://{}/webhooks/telnyx",
        server.telnyx_callback_address.unwrap()
    );
    let callback_admin = format!(
        "http://{}/v2/incoming-contacts",
        server.telnyx_callback_address.unwrap()
    );
    assert_eq!(
        reqwest::Client::new()
            .post(&callback_admin)
            .bearer_auth(&p.administrator)
            .json(&json!({"command":"list","limit":32}))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let stamp = chrono::Utc::now().timestamp();
    let event = incoming("held-public-sms", "private held SMS");
    assert_eq!(
        callback(&webhook, &key, &event, stamp, false)
            .await
            .status(),
        204
    );
    let mut admin = ApplicationClient::open(p.http(&server, &p.administrator))
        .await
        .unwrap();
    let IncomingContactOutcome::Page { page } = admin
        .incoming_contact_command(IncomingContactCommand::List {
            after: None,
            limit: 32,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(page.contacts.len(), 1);
    let held = page.contacts[0].clone();
    let raw_http = reqwest::Client::new();
    let raw_url = format!(
        "http://{}/v2/incoming-contacts",
        server.http_address.unwrap()
    );
    let request = json!({"command":"list","after":null,"limit":32});
    let (status, _) = super::incoming_contact_http::record(
        "sms-held-list",
        request.clone(),
        raw_http
            .post(&raw_url)
            .bearer_auth(&p.administrator)
            .json(&request)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(held.reason, IncomingContactReason::AmbiguousRoute);
    assert_eq!(held.candidates.len(), 2);
    let raw = serde_json::to_string(&held).unwrap();
    assert!(!raw.contains("private held SMS"));
    assert!(!raw.contains("resource-held-public-sms"));
    let mut native = CanonicalClient::open(p.client.clone()).await.unwrap();
    for cid in [&p.cid, &other] {
        let history = native
            .history(cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        assert!(history
            .messages
            .iter()
            .all(|message| message.body != "private held SMS"));
    }
    assert_eq!(chats.load(Ordering::SeqCst), 0);
    assert_eq!(sent.load(Ordering::SeqCst), 0);
    let chosen = held
        .candidates
        .iter()
        .find(|candidate| candidate.conversation_id == other)
        .unwrap();
    let selection = IncomingContactCommand::Resolve(IncomingContactSelection {
        operation_id: "select-public-sms".into(),
        contact_id: held.id.clone(),
        candidate_id: chosen.id.clone(),
        expected_revision: 0,
    });
    let IncomingContactOutcome::Contact { contact } = admin
        .incoming_contact_command(selection.clone())
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(contact.state, IncomingContactState::Resolved);
    assert_eq!(
        contact.decision.as_ref().unwrap().operation_id,
        "select-public-sms"
    );
    let history = native
        .history(
            &other,
            serde_json::from_value(json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].body, "private held SMS");
    assert_eq!(
        Some(history.messages[0].msg_id.as_str()),
        contact.decision.as_ref().unwrap().message_id.as_deref()
    );
    let mut observer = CanonicalHttpClient::open(p.http(&server, &p.observer))
        .await
        .unwrap();
    assert!(observer
        .history(
            &other,
            serde_json::from_value(json!({"limit":100})).unwrap()
        )
        .await
        .unwrap()
        .messages
        .is_empty());
    let mut account = CanonicalHttpClient::open(p.http(&server, &p.administrator))
        .await
        .unwrap();
    assert!(account
        .history(
            &other,
            serde_json::from_value(json!({"limit":100})).unwrap()
        )
        .await
        .is_err());
    assert_eq!(
        admin
            .incoming_contact_command(selection.clone())
            .await
            .unwrap(),
        IncomingContactOutcome::Contact {
            contact: contact.clone()
        }
    );
    let request = serde_json::to_value(&selection).unwrap();
    let (status, _) = super::incoming_contact_http::record(
        "sms-resolve-replay",
        request.clone(),
        raw_http
            .post(&raw_url)
            .bearer_auth(&p.administrator)
            .json(&request)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let mut retry = event.clone();
    retry["data"]["id"] = json!("held-public-sms-retry");
    assert_eq!(
        callback(&webhook, &key, &retry, stamp, false)
            .await
            .status(),
        204
    );
    assert_eq!(
        native
            .history(
                &other,
                serde_json::from_value(json!({"limit":100})).unwrap()
            )
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    let other_candidate = held
        .candidates
        .iter()
        .find(|candidate| candidate.conversation_id == p.cid)
        .unwrap();
    let conflicting = IncomingContactCommand::Resolve(IncomingContactSelection {
        operation_id: "competing-public-sms".into(),
        contact_id: held.id.clone(),
        candidate_id: other_candidate.id.clone(),
        expected_revision: 0,
    });
    assert!(
        matches!(admin.incoming_contact_command(conflicting).await,Err(connect_client::ClientError::Application(error)) if error.code=="conflict")
    );
    native.close();
    server.shutdown().await.unwrap();
    server = p.start(drivers, Some(callbacks)).await;
    webhook = format!(
        "http://{}/webhooks/telnyx",
        server.telnyx_callback_address.unwrap()
    );
    admin = ApplicationClient::open(p.http(&server, &p.administrator))
        .await
        .unwrap();
    assert_eq!(
        admin
            .incoming_contact_command(selection.clone())
            .await
            .unwrap(),
        IncomingContactOutcome::Contact {
            contact: contact.clone()
        }
    );
    let rejected_event = incoming("public-reject-sms", "never delivered");
    assert_eq!(
        callback(&webhook, &key, &rejected_event, stamp, false)
            .await
            .status(),
        204
    );
    let IncomingContactOutcome::Page { page } = admin
        .incoming_contact_command(IncomingContactCommand::List {
            after: None,
            limit: 32,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let undecided = page
        .contacts
        .iter()
        .find(|c| c.state == IncomingContactState::Held)
        .unwrap();
    let reject = IncomingContactCommand::Reject {
        operation_id: "public-reject".into(),
        contact_id: undecided.id.clone(),
        expected_revision: 0,
        reason: "not for these rooms".into(),
    };
    let rejected = admin
        .incoming_contact_command(reject.clone())
        .await
        .unwrap();
    assert_eq!(
        admin.incoming_contact_command(reject).await.unwrap(),
        rejected
    );
    let request = json!({"command":"get","contact_id":undecided.id});
    let url = format!(
        "http://{}/v2/incoming-contacts",
        server.http_address.unwrap()
    );
    let (status, _) = super::incoming_contact_http::record(
        "sms-rejected-get",
        request.clone(),
        raw_http
            .post(&url)
            .bearer_auth(&p.administrator)
            .json(&request)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        callback(&webhook, &key, &rejected_event, stamp, false)
            .await
            .status(),
        204
    );
    let mut stop = incoming("public-stop", "STOP");
    stop["data"]["occurred_at"] = json!(chrono::DateTime::from_timestamp(stamp, 0)
        .unwrap()
        .to_rfc3339());
    assert_eq!(
        callback(&webhook, &key, &stop, stamp, false).await.status(),
        204
    );
    let request = serde_json::to_value(&selection).unwrap();
    let (status, _) = super::incoming_contact_http::record(
        "sms-withdrawn-replay",
        request.clone(),
        raw_http
            .post(&url)
            .bearer_auth(&p.administrator)
            .json(&request)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 403);
    assert!(
        matches!(admin.incoming_contact_command(selection).await,Err(connect_client::ClientError::Application(error)) if error.code=="forbidden")
    );
    native = CanonicalClient::open(p.client.clone()).await.unwrap();
    for cid in [&p.cid, &other] {
        let history = native
            .history(cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        assert!(history.messages.iter().all(|m| m.body != "never delivered"));
        if *cid == p.cid {
            assert!(history
                .messages
                .iter()
                .all(|m| m.body != "private held SMS"));
        }
    }
    assert_eq!(chats.load(Ordering::SeqCst), 0);
    assert_eq!(sent.load(Ordering::SeqCst), 0);
    native.close();
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_sms_vapi_reply_delivery_and_http_quic_replay_share_one_journal() {
    let mut p = prepare(true).await;
    let mock = MockServer::start().await;
    assistant(&mock).await;
    let chats = Arc::new(AtomicUsize::new(0));
    let sms = Arc::new(AtomicUsize::new(0));
    let c = chats.clone();
    Mock::given(method("POST")).and(header("authorization","Bearer fake-vapi")).and(path("/chat")).respond_with(move|r:&wiremock::Request| {
  let body=String::from_utf8(r.body.clone()).unwrap();assert!(!body.contains("owner-only-to-observer"));assert!(body.contains("allowed-context-primer"),"authorized context was not initialized");let n=c.fetch_add(1,Ordering::SeqCst)+1;ResponseTemplate::new(200).set_body_json(json!({"id":format!("chat-{n}"),"output":[{"role":"assistant","content":"SMS answer"}]}))
 }).mount(&mock).await;
    let sent = sms.clone();
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer fake-sms"))
        .and(path("/messages"))
        .respond_with(move |_: &wiremock::Request| {
            let n = sent.fetch_add(1, Ordering::SeqCst) + 1;
            ResponseTemplate::new(200).set_body_json(json!({"data":{"id":format!("sms-{n}")}}))
        })
        .mount(&mock)
        .await;
    let key = SigningKey::from_bytes(&[7; 32]);
    let server = p
        .start(
            vec![
                Arc::new(
                    connect_connectors::Vapi::with_reference(
                        "fake-vapi",
                        &mock.uri(),
                        "MESSAGE_VAPI",
                    )
                    .unwrap(),
                ),
                Arc::new(
                    connect_connectors::Telnyx::with_reference(
                        "fake-sms",
                        &mock.uri(),
                        "MESSAGE_SMS",
                    )
                    .unwrap(),
                ),
            ],
            Some(TelnyxCallbacks {
                bind: "127.0.0.1:0".parse().unwrap(),
                verification_key: STANDARD.encode(key.verifying_key().to_bytes()),
            }),
        )
        .await;
    let base = format!("http://{}", server.telnyx_callback_address.unwrap());
    let webhook = format!("{base}/webhooks/telnyx");
    assert_eq!(
        reqwest::Client::new()
            .post(format!("{base}/v1/operations"))
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let mut native = CanonicalClient::open(p.client.clone()).await.unwrap();
    let snap = server
        .store
        .membership_snapshot(
            &server
                .store
                .authenticate(&p.client.credential)
                .unwrap()
                .credential_id,
            &p.cid,
        )
        .unwrap();
    let observer = server.store.authenticate(&p.observer).unwrap();
    let observer_pid = snap
        .participants
        .iter()
        .find(|m| m.identity_id == observer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    native
        .send_message(
            "private-note",
            &p.cid,
            "private-note-id",
            connect_client::MessageDraft::text(
                connect_protocol::messages::MessageAudience::Participants(vec![observer_pid]),
                "owner-only-to-observer",
            ),
        )
        .await
        .unwrap();
    let stamp = chrono::Utc::now().timestamp();
    for (i, text) in ["STOP", "stop all", "HELP", "START", "custom help"]
        .iter()
        .enumerate()
    {
        let mut event = incoming(&format!("control-{i}"), text);
        event["data"]["occurred_at"] =
            json!(chrono::DateTime::from_timestamp(stamp - 10 + i as i64, 0)
                .unwrap()
                .to_rfc3339());
        if i == 4 {
            event["data"]["payload"]["autoresponse_type"] = json!("HELP");
        }
        if i == 0 {
            let held = server
                .store
                .read_authority(
                    &server
                        .store
                        .authenticate(&p.client.credential)
                        .unwrap()
                        .credential_id,
                    Some(&p.cid),
                    &connect_store::AuthorityWait::new(Duration::from_secs(1)),
                )
                .unwrap();
            let before = connect_store::Store::inspect_snapshot(server.store.path()).unwrap();
            assert_eq!(
                callback(&webhook, &key, &event, stamp, false)
                    .await
                    .status(),
                503
            );
            assert_eq!(
                connect_store::Store::inspect_snapshot(server.store.path()).unwrap(),
                before
            );
            drop(held);
        }
        assert_eq!(
            callback(&webhook, &key, &event, stamp, false)
                .await
                .status(),
            204
        );
    }
    // Scoped callbacks reject another source, but signed STOP from an unknown
    // contact to the mounted source still records opt-out without enrollment.
    let mut foreign = incoming("foreign-source", "STOP");
    foreign["data"]["occurred_at"] = json!(chrono::Utc::now().to_rfc3339());
    foreign["data"]["payload"]["to"][0]["phone_number"] = json!("+15555550999");
    assert_eq!(
        callback(&webhook, &key, &foreign, stamp, false)
            .await
            .status(),
        403
    );
    let mut unknown = incoming("unknown-stop", "STOP");
    unknown["data"]["occurred_at"] = json!(chrono::Utc::now().to_rfc3339());
    unknown["data"]["payload"]["from"]["phone_number"] = json!("+15555550998");
    assert_eq!(
        callback(&webhook, &key, &unknown, stamp, false)
            .await
            .status(),
        204
    );
    unknown["data"]["id"] = json!("unknown-message");
    unknown["data"]["payload"]["text"] = json!("not enrolled");
    assert_eq!(
        callback(&webhook, &key, &unknown, stamp, false)
            .await
            .status(),
        403
    );
    let foreign_final = json!({"data":{"id":"foreign-final","event_type":"message.finalized","payload":{"id":"unattributed-message","to":[{"status":"delivered"}]}}});
    assert_eq!(
        callback(&webhook, &key, &foreign_final, stamp, false)
            .await
            .status(),
        403
    );
    let event = incoming("incoming-1", "help with SMS question");
    assert_eq!(
        callback(&webhook, &key, &event, stamp, true).await.status(),
        401
    );
    assert_eq!(
        callback(&webhook, &key, &event, stamp - 600, false)
            .await
            .status(),
        401
    );
    assert_eq!(
        callback(&webhook, &key, &event, stamp, false)
            .await
            .status(),
        204
    );
    assert_eq!(
        callback(&webhook, &key, &event, stamp, false)
            .await
            .status(),
        204
    );
    let mut replay = event.clone();
    replay["meta"] = json!({"attempt":2});
    assert_eq!(
        callback(&webhook, &key, &replay, stamp, false)
            .await
            .status(),
        204
    );
    replay["data"]["payload"]["text"] = json!("changed");
    assert_eq!(
        callback(&webhook, &key, &replay, stamp, false)
            .await
            .status(),
        400
    );
    wait_messages(&mut native, &p.cid, 4).await;
    let draft = connect_client::MessageDraft::text(
        connect_protocol::messages::MessageAudience::Participants(vec![p.ai_pid.clone()]),
        "quic-repeat-question",
    );
    let original = native
        .send_message("quic-send", &p.cid, "quic-msg", draft.clone())
        .await
        .unwrap();
    let mut http = CanonicalHttpClient::open(p.http(&server, &p.client.credential))
        .await
        .unwrap();
    assert_eq!(
        http.send_message("quic-send", &p.cid, "quic-msg", draft)
            .await
            .unwrap(),
        original
    );
    wait_messages(&mut native, &p.cid, 6).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while sms.load(Ordering::SeqCst) < 2 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let finalized = json!({"data":{"id":"delivery","event_type":"message.finalized","payload":{"id":"sms-1","to":[{"status":"delivered"}]}}});
    assert_eq!(
        callback(&webhook, &key, &finalized, stamp, false)
            .await
            .status(),
        204
    );
    assert_eq!(
        callback(&webhook, &key, &finalized, stamp, false)
            .await
            .status(),
        204
    );
    let mut conflicting_final = finalized.clone();
    conflicting_final["data"]["id"] = json!("conflicting-delivery");
    conflicting_final["data"]["payload"]["to"][0]["status"] = json!("delivery_failed");
    assert_eq!(
        callback(&webhook, &key, &conflicting_final, stamp, false)
            .await
            .status(),
        400
    );
    let mut observer = CanonicalHttpClient::open(p.http(&server, &p.observer))
        .await
        .unwrap();
    let history = observer
        .history(
            &p.cid,
            serde_json::from_value(json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].body, "owner-only-to-observer");
    assert_eq!(chats.load(Ordering::SeqCst), 2);
    assert_eq!(sms.load(Ordering::SeqCst), 2);
    native.close();
    server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owned_worker_shutdown_records_uncertainty_and_restart_does_not_resubmit() {
    let mut p = prepare(false).await;
    let mock = MockServer::start().await;
    assistant(&mock).await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer fake-vapi"))
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(60))
                .set_body_json(json!({"id":"late","output":[{"content":"late"}]})),
        )
        .mount(&mock)
        .await;
    let driver = Arc::new(
        connect_connectors::Vapi::with_reference("fake-vapi", &mock.uri(), "MESSAGE_VAPI").unwrap(),
    );
    let server = p.start(vec![driver.clone()], None).await;
    let mut native = CanonicalClient::open(p.client.clone()).await.unwrap();
    let draft = connect_client::MessageDraft::text(
        connect_protocol::messages::MessageAudience::Participants(vec![p.ai_pid.clone()]),
        "uncertain-question",
    );
    let receipt = native
        .send_message("uncertain-send", &p.cid, "uncertain-msg", draft.clone())
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|r| r.method.as_str() == "POST")
    {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    native.close();
    server.shutdown().await.unwrap();
    let restarted = p.start(vec![driver], None).await;
    let mut native = CanonicalClient::open(p.client.clone()).await.unwrap();
    assert_eq!(
        native
            .send_message("uncertain-send", &p.cid, "uncertain-msg", draft)
            .await
            .unwrap(),
        receipt
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        mock.received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .count(),
        1
    );
    let Outcome::ProviderStatus { deliveries } = restarted
        .store
        .execute(
            &restarted
                .store
                .authenticate(&p.client.credential)
                .unwrap()
                .credential_id,
            &connect_protocol::Operation::ProviderStatus {
                conversation_id: p.cid.clone(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(deliveries[0].state, "uncertain");
    native.close();
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_early_sms_final_is_scoped_and_settles_typed_delivery_after_ack() {
    let mut p = prepare(true).await;
    let mock = MockServer::start().await;
    assistant(&mock).await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":"early-chat","output":[{"role":"assistant","content":"early answer"}]}),
        ))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(5))
                .set_body_json(json!({"data":{"id":"early-sms"}})),
        )
        .mount(&mock)
        .await;
    let key = SigningKey::from_bytes(&[9; 32]);
    let server = p
        .start(
            vec![
                Arc::new(
                    connect_connectors::Vapi::with_reference(
                        "fake-vapi",
                        &mock.uri(),
                        "MESSAGE_VAPI",
                    )
                    .unwrap(),
                ),
                Arc::new(
                    connect_connectors::Telnyx::with_reference(
                        "fake-sms",
                        &mock.uri(),
                        "MESSAGE_SMS",
                    )
                    .unwrap(),
                ),
            ],
            Some(TelnyxCallbacks {
                bind: "127.0.0.1:0".parse().unwrap(),
                verification_key: STANDARD.encode(key.verifying_key().to_bytes()),
            }),
        )
        .await;
    let webhook = format!(
        "http://{}/webhooks/telnyx",
        server.telnyx_callback_address.unwrap()
    );
    let stamp = chrono::Utc::now().timestamp();
    assert_eq!(
        callback(
            &webhook,
            &key,
            &incoming("early-question", "early question"),
            stamp,
            false
        )
        .await
        .status(),
        204
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|r| r.url.path() == "/messages")
    {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let final_event = json!({"data":{"id":"f".repeat(249),"event_type":"message.finalized","payload":{"id":"early-sms","from":{"phone_number":"+15555550102"},"to":[{"phone_number":"+15555550101","status":"delivered"}]}}});
    assert_eq!(
        callback(&webhook, &key, &final_event, stamp, false)
            .await
            .status(),
        204
    );
    let before_ack = server
        .store
        .execute(
            &server
                .store
                .authenticate(&p.client.credential)
                .unwrap()
                .credential_id,
            &connect_protocol::Operation::ProviderStatus {
                conversation_id: p.cid.clone(),
            },
        )
        .unwrap();
    let Outcome::ProviderStatus { deliveries } = before_ack else {
        panic!()
    };
    assert!(
        deliveries
            .iter()
            .any(|d| d.connector == "telnyx" && d.state == "sending"),
        "early callback waited for the API response"
    );
    let actor = server.store.authenticate(&p.client.credential).unwrap();
    loop {
        let Outcome::ProviderStatus { deliveries } = server
            .store
            .execute(
                &actor.credential_id,
                &connect_protocol::Operation::ProviderStatus {
                    conversation_id: p.cid.clone(),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        if deliveries
            .iter()
            .any(|d| d.connector == "telnyx" && d.state == "delivered")
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "early final fact was not reconciled"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        callback(&webhook, &key, &final_event, stamp, false)
            .await
            .status(),
        204
    );
    assert_eq!(
        mock.received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/messages")
            .count(),
        1
    );
    server.shutdown().await.unwrap();
}
