//! Trusted fixture host. Exact provider receipts are controlled; native source,
//! current source custody, public HTTP observation reads and handoffs are real.
use super::*;
use connect_client::application::{ApplicationClient, ApplicationOperation};
use connect_protocol::{events::*, tasks::*, Outcome};
use connect_store::{AssistantProviderApplied, AssistantProviderContext, AssistantProviderCreated};
use std::sync::Arc;
pub(super) struct Host {
    pub work: ObservationWork,
    publisher: String,
    captures: Vec<(
        AssistantProviderContext,
        Arc<AssistantProviderApplied>,
        AssistantProviderCreated,
        connect_service::ProviderObservationSource,
    )>,
}
async fn task(api: &mut ApplicationClient, command: TaskCommand) -> TaskOutcome {
    let op = ApplicationOperation::try_from(connect_protocol::Operation::Task {
        request: Box::new(TaskRequest::new(command)),
    })
    .unwrap();
    let Outcome::Task(outcome) = api.execute(op).await.unwrap() else {
        panic!("task result")
    };
    *outcome
}
pub(super) async fn prepare(f: &Fixture, identity: &str, cid: &str, source: &str) -> Host {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend(
        [
            "task_create",
            "task_invoke",
            "task_get",
            "task_effect_get",
            "session.observation.publish",
        ]
        .map(str::to_owned),
    );
    let publisher = f
        .server
        .store
        .issue(identity, scope, chrono::Utc::now().timestamp() + 600)
        .unwrap();
    let mut cfg = canonical_http_config(f);
    cfg.credential = publisher.token;
    let mut api = ApplicationClient::open(cfg).await.unwrap();
    let alias = f.server.store.identity(identity).unwrap().alias;
    api.negotiate_tasks(&format!("connect:{alias}@local"), Some(cid))
        .await
        .unwrap();
    let recipients = f
        .server
        .store
        .membership_snapshot(&publisher.id, cid)
        .unwrap()
        .participants
        .into_iter()
        .filter(|p| p.active)
        .map(|p| p.identity_id)
        .collect();
    let TaskOutcome::Record(record) = task(
        &mut api,
        TaskCommand::Create {
            request_id: "worker-task".into(),
            definition: TaskDefinition {
                conversation_id: cid.into(),
                scope: TaskScope {
                    sms_enrollments: vec![],
                    operations: vec![TaskPermission::Send, TaskPermission::History],
                    recipients,
                },
                expires_at: chrono::Utc::now().timestamp() + 500,
                initiating_event_id: None,
            },
        },
    )
    .await
    else {
        panic!("task")
    };
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect),
        ..
    }) = task(
        &mut api,
        TaskCommand::Invoke {
            request_id: "worker-question".into(),
            task_id: record.id.clone(),
            step_id: "question".into(),
            operation: Box::new(connect_protocol::Operation::Send {
                request_id: "worker-question".into(),
                conversation_id: cid.into(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "Please consider the application's independently approved handoff decision",
                ),
            }),
            reply_to: None,
            wait_for_reply: true,
        },
    )
    .await
    else {
        panic!("pending question")
    };
    let binding = f
        .server
        .store
        .prepare_task_observation_binding(&publisher.id, &record.id, &effect)
        .unwrap();
    let work = binding.descriptor().clone();
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: "worker-provider-endpoint".into(),
                identity_id: identity.into(),
                device: DeviceSpec {
                    id: "worker-provider".into(),
                    kind: "service".into(),
                    platform: "controlled-fixture".into(),
                },
                transport: EndpointTransport::VapiVoice,
                address: "worker-provider-fixture".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("worker_fixture_secret".into()),
            },
        )
        .unwrap()
    else {
        panic!("endpoint")
    };
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "worker-verify-provider".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-fixture".into(),
            },
        )
        .unwrap();
    let generation = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT generation FROM connections WHERE id=?1",
            [source],
            |r| r.get(0),
        )
        .unwrap();
    let mut captures = vec![];
    for index in 0..2 {
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        let selected = loop {
            match f.server.service.try_assistant_context_read(
                &publisher.id,
                source,
                generation,
                &endpoint_id,
            ) {
                Ok(read) => break read,
                Err(e)
                    if matches!(
                        e.code.as_str(),
                        "authority-busy" | "not-ready" | "storage-unavailable"
                    ) && tokio::time::Instant::now() < until =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(e) => panic!("actual AI source: {}", e.code),
            }
        };
        let context = f
            .server
            .service
            .prepare_assistant_provider(&format!("worker-provider-{index}"), selected)
            .unwrap();
        let read = f
            .server
            .service
            .try_assistant_provider_read(&context)
            .unwrap();
        let (attempt, read) = f
            .server
            .service
            .begin_assistant_provider(context.clone(), read)
            .unwrap();
        drop(read);
        let created = attempt
            .created(format!("worker-provider-reference-{index}"))
            .unwrap();
        f.server
            .store
            .record_native_assistant_provider_created(&created)
            .unwrap();
        let read = f
            .server
            .service
            .try_assistant_provider_read(&context)
            .unwrap();
        let json = read.context_json().unwrap().to_owned();
        let view: connect_protocol::ContextView = serde_json::from_str(&json).unwrap();
        assert!(view
            .items
            .iter()
            .any(|item| item.kind == "message" && item.source_id == work.question_id));
        let (export, read) = f
            .server
            .service
            .begin_assistant_export(context.clone(), read)
            .unwrap();
        drop(read);
        let applied = Arc::new(export.acknowledged(&json).unwrap());
        f.server
            .store
            .record_native_assistant_delivered(&applied)
            .unwrap();
        let captured = f
            .server
            .service
            .bind_assistant_observation_source_with_work(
                &publisher.id,
                &context,
                applied.clone(),
                ObservationCapabilities {
                    transcript: true,
                    actions: vec!["handoff".into()],
                    ..Default::default()
                },
                Some(binding.clone()),
            )
            .unwrap();
        captures.push((context, applied, created, captured));
    }
    Host {
        work,
        publisher: publisher.id,
        captures,
    }
}
impl Host {
    pub async fn publish(&self, f: &Fixture) {
        for (index, (context, applied, _, source)) in self.captures.iter().enumerate() {
            let request = AgentObservationContent::Action {
                name: Some("handoff".into()),
                call_id: Some("same-tool-call".into()),
                phase: AgentActionPhase::Requested,
                arguments: Some(
                    json!({"task_id":"model-forged-task","effect_id":"model-forged-effect","operation_id":"model-forged-operation","target":"model-forged-target"}),
                ),
                result: None,
                completeness: AgentCompleteness::Final,
            };
            for (sequence, content) in [
                AgentObservationContent::Transcript {
                    text: Some("partial suggestion".into()),
                    speaker_role: Some(AgentSpeakerRole::User),
                    completeness: AgentCompleteness::Partial,
                },
                request.clone(),
                request,
                AgentObservationContent::Action {
                    name: Some("handoff".into()),
                    call_id: Some("same-tool-call".into()),
                    phase: AgentActionPhase::ResultObserved,
                    arguments: None,
                    result: Some(json!({"completed":true})),
                    completeness: AgentCompleteness::Final,
                },
            ]
            .into_iter()
            .enumerate()
            {
                let draft = ObservationDraft::Agent {
                    observation: AgentObservation {
                        local_sequence: sequence as u64 + 1,
                        capture_complete: false,
                        missing_before: None,
                        provider_session_id: Some(format!("worker-provider-reference-{index}")),
                        provider_event_id: Some("same-provider-event".into()),
                        captured_at: chrono::Utc::now().timestamp(),
                        content,
                    },
                };
                let until = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let result = f
                        .server
                        .service
                        .try_assistant_memory_read(context, applied)
                        .and_then(|read| {
                            f.server.service.publish_assistant_observation(
                                source,
                                &read,
                                &format!("worker-{index}-{sequence}"),
                                draft.clone(),
                            )
                        });
                    match result {
                        Ok(_) => break,
                        Err(e)
                            if matches!(
                                e.code.as_str(),
                                "authority-busy" | "storage-unavailable"
                            ) && tokio::time::Instant::now() < until =>
                        {
                            tokio::time::sleep(Duration::from_millis(20)).await
                        }
                        Err(e) => panic!("publish controlled provider data: {}", e.code),
                    }
                }
            }
        }
    }
    pub async fn page(&self, f: &Fixture, cid: &str) -> Vec<SessionObservation> {
        let identity = f
            .server
            .store
            .revalidate(&self.publisher)
            .unwrap()
            .identity_id;
        self.page_as(f, cid, &identity)
            .await
            .observations
            .into_iter()
            .filter(|v| v.work.as_ref() == Some(&self.work))
            .collect()
    }
    pub async fn page_as(&self, f: &Fixture, cid: &str, identity: &str) -> ObservationPage {
        let key = f
            .server
            .store
            .issue(
                &identity,
                Scope::collaboration(vec!["*".into()]),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut cfg = canonical_http_config(f);
        cfg.credential = key.token;
        let mut api = ApplicationClient::open(cfg).await.unwrap();
        let op = ApplicationOperation::try_from(connect_protocol::Operation::SessionObservations {
            conversation_id: cid.into(),
            session_id: "native-session".into(),
            query: ObservationQuery::default(),
        })
        .unwrap();
        let Outcome::SessionObservations(page) = api.execute(op).await.unwrap() else {
            panic!("observation page")
        };
        page
    }
    pub fn close(self, f: &Fixture) {
        for (_, _, created, source) in self.captures {
            source.close();
            let end = f.server.store.begin_native_assistant_end(&created).unwrap();
            f.server
                .store
                .record_native_assistant_closed(&end.closed())
                .unwrap();
        }
    }
}
