//! Actual registered native Service binding and public HTTP projection. Provider
//! receipts here are controlled Store receipts, not provider transport evidence.
use super::*;
use connect_protocol::events::*;
async fn page(f: &Fixture, cid: &str, sid: &str) -> ObservationPage {
    let mut api = connect_client::application::ApplicationClient::open(canonical_http_config(f))
        .await
        .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let command = connect_client::application::ApplicationOperation::try_from(
            connect_protocol::Operation::SessionObservations {
                conversation_id: cid.into(),
                session_id: sid.into(),
                query: ObservationQuery::default(),
            },
        )
        .unwrap();
        match api.execute(command).await {
            Ok(connect_protocol::Outcome::SessionObservations(page)) => return page,
            Err(connect_client::ClientError::Application(e))
                if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                    && tokio::time::Instant::now() < until =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("public observation read: {e:?}"),
            _ => panic!("unexpected observation outcome"),
        }
    }
}
pub(super) async fn verify(f: &Fixture, key: &str, endpoint: &str, cid: &str) {
    let mut scope = connect_store::Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "session.observation.publish".into(),
        "session.observation.publish.any".into(),
    ]);
    let actor = f.server.store.revalidate(key).unwrap();
    let publisher = f
        .server
        .store
        .issue(
            &actor.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .id;
    for (index, explicit) in [true, false].into_iter().enumerate() {
        let selected = public_context::selection(f, key, endpoint).await;
        let sid = selected.target().connection.session_id.clone();
        let context = f
            .server
            .service
            .prepare_assistant_provider(&format!("controlled-observation-source-{index}"), selected)
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
            .created(format!("controlled-observation-no-provider-io-{index}"))
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
        let body = read.context_json().unwrap().to_owned();
        let (export, read) = f
            .server
            .service
            .begin_assistant_export(context.clone(), read)
            .unwrap();
        drop(read);
        let applied = Arc::new(export.acknowledged(&body).unwrap());
        f.server
            .store
            .record_native_assistant_delivered(&applied)
            .unwrap();
        let source = f
            .server
            .service
            .bind_assistant_observation_source(
                &publisher,
                &context,
                applied.clone(),
                ObservationCapabilities {
                    transcript: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let read = f
            .server
            .service
            .try_assistant_memory_read(&context, &applied)
            .unwrap();
        let observation = f
            .server
            .service
            .publish_assistant_observation(
                &source,
                &read,
                "one",
                ObservationDraft::Transcript {
                    text: "controlled observation on actual registered native source".into(),
                    completeness: TranscriptCompleteness::Interim,
                    turn_id: None,
                    fragment_id: None,
                },
            )
            .unwrap();
        drop(read);
        let p = page(f, cid, &sid).await;
        let item = p
            .observations
            .iter()
            .find(|o| o.observation_id == observation)
            .unwrap();
        assert_eq!(item.connection_id, context.connection_id());
        assert_eq!(item.generation, context.generation());
        assert_eq!(item.source.correlation_id, context.delivery_id());
        if explicit {
            source.close();
        }
        drop(source);
        let p = page(f, cid, &sid).await;
        assert!(!p
            .observations
            .iter()
            .any(|o| o.observation_id == observation));
        assert!(!p
            .routes
            .iter()
            .any(|r| r.correlation_id == context.delivery_id()));
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        let current = loop {
            match f
                .server
                .service
                .try_assistant_memory_read(&context, &applied)
            {
                Ok(read) => break read,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable"
                    ) && tokio::time::Instant::now() < until =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(error) => panic!(
                    "independently current native memory read refused: {}",
                    error.code
                ),
            }
        };
        assert!(current.current(),"closure/drop of host owner precedes any Store cleanup and leaves actual native source current");
        drop(current);
        let end = f.server.store.begin_native_assistant_end(&created).unwrap();
        f.server
            .store
            .record_native_assistant_closed(&end.closed())
            .unwrap();
    }
    println!("actual Service observation ownership: captured registry/native source; controlled provider receipts; public HTTP attribution; explicit close and owner drop hide buffer before metadata cleanup; no provider transport/publication claim");
}

pub(super) fn configuration(
    f: &Fixture,
    key: &str,
) -> connect_server::agent_observations::AgentObservationConfig {
    let mut scope = connect_store::Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "session.observation.publish".into(),
        "session.observation.publish.any".into(),
    ]);
    let actor = f.server.store.revalidate(key).unwrap();
    let credential = f
        .server
        .store
        .issue(
            &actor.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    connect_server::agent_observations::AgentObservationConfig::new(
        credential.id,
        ObservationCapabilities {
            transcript: true,
            speech: true,
            actions: vec!["handoff".into()],
        },
    )
    .unwrap()
}
// The test host selects a committed task question before selecting model memory.
// None of the provider's claimed IDs participate in this composition.
pub(super) async fn configuration_with_work(
    f: &Fixture,
    key: &str,
    cid: &str,
) -> (
    connect_server::agent_observations::AgentObservationConfig,
    ObservationWork,
) {
    use connect_client::application::{ApplicationClient, ApplicationOperation};
    use connect_protocol::tasks::*;
    let actor = f.server.store.revalidate(key).unwrap();
    let credential = f
        .server
        .store
        .issue(
            &actor.identity_id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(f);
    cfg.credential = credential.token;
    let mut api = ApplicationClient::open(cfg).await.unwrap();
    let alias = f.server.store.identity(&actor.identity_id).unwrap().alias;
    api.negotiate_tasks(&format!("connect:{alias}@local"), Some(cid))
        .await
        .unwrap();
    let participants = f
        .server
        .store
        .membership_snapshot(key, cid)
        .unwrap()
        .participants
        .into_iter()
        .filter(|p| p.active)
        .map(|p| p.identity_id)
        .collect();
    let create = TaskCommand::Create {
        request_id: "live-observation-work-task".into(),
        definition: TaskDefinition {
            conversation_id: cid.into(),
            scope: TaskScope {
                sms_enrollments: vec![],
                operations: vec![TaskPermission::Send, TaskPermission::History],
                recipients: participants,
            },
            expires_at: chrono::Utc::now().timestamp() + 500,
            initiating_event_id: None,
        },
    };
    async fn run(api: &mut ApplicationClient, command: TaskCommand) -> TaskOutcome {
        let op = ApplicationOperation::try_from(connect_protocol::Operation::Task {
            request: Box::new(TaskRequest::new(command)),
        })
        .unwrap();
        let connect_protocol::Outcome::Task(result) = api.execute(op).await.unwrap() else {
            panic!("task API")
        };
        *result
    }
    let TaskOutcome::Record(task) = run(&mut api, create).await else {
        panic!("task")
    };
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect),
        ..
    }) = run(
        &mut api,
        TaskCommand::Invoke {
            request_id: "live-observation-work-question".into(),
            task_id: task.id.clone(),
            step_id: "question".into(),
            operation: Box::new(connect_protocol::Operation::Send {
                request_id: "live-observation-work-question".into(),
                conversation_id: cid.into(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "Explicit task question supplied to this provider",
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
    let mut scope = connect_store::Scope::collaboration(vec!["*".into()]);
    scope.operations.extend(
        [
            "session.observation.publish",
            "session.observation.publish.any",
            "task_get",
            "task_effect_get",
        ]
        .map(str::to_owned),
    );
    let publisher = f
        .server
        .store
        .issue(
            &actor.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let binding = f
        .server
        .store
        .prepare_task_observation_binding(&publisher.id, &task.id, &effect)
        .unwrap();
    let work = binding.descriptor().clone();
    let config = connect_server::agent_observations::AgentObservationConfig::new(
        publisher.id,
        ObservationCapabilities {
            transcript: true,
            speech: true,
            actions: vec!["handoff".into()],
        },
    )
    .unwrap()
    .with_work(binding);
    (config, work)
}
pub(super) fn frames() -> Vec<Value> {
    let request = json!({"type":"tool-calls","eventId":"repeated-provider-event","toolCallList":[{"id":"repeated-tool","function":{"name":"handoff","arguments":{"task_id":"model-forged-task","effect_id":"model-forged-effect","operation_id":"model-forged-operation"}}}]});
    vec![
        json!({"type":"transcript","role":"user","transcriptType":"partial","transcript":"partial mixed-audio words"}),
        json!({"type":"transcript","role":"assistant","transcriptType":"final","transcript":"one final provider fragment"}),
        json!({"type":"transcript"}),
        json!({"type":"transcript","transcriptType":"final","transcript":"final without role"}),
        json!({"type":"transcript","role":"assistant","transcriptType":"final"}),
        json!({"type":"transcript","role":"optional-model-role","transcriptType":"final","transcript":"final other role"}),
        json!({"type":"speech-update","role":"user","status":"started","turn":4}),
        request.clone(),
        request,
        json!({"type":"tool-calls-result","results":[{"toolCallId":"repeated-tool","result":{"ok":true}}]}),
        json!({"type":"optional-unsupported-model-event"}),
        json!({"type":"transcript","role":"assistant","transcriptType":"partial","transcript":"\"".repeat(12_000)}),
    ]
}
// Capture is deliberately best effort. Drive exhaustive fixture samples one at
// a time, observing each through the public API before supplying its successor.
// Never resend a missing sample or treat a reported gap as successful capture.
pub(super) async fn release_live(
    f: &Fixture,
    cid: &str,
    sid: &str,
    correlation: &str,
    release: &tokio::sync::Notify,
) {
    for index in 0..frames().len() {
        let sequence = index as u64 + 2; // The initial assistant marker is first.
        release.notify_one();
        // The initial marker can still have its own five-second publication
        // attempt in flight. Allow that attempt and this sample's attempt.
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let p = page(f, cid, sid).await;
            if p.observations.iter().any(|v| {
                v.source.correlation_id == correlation
                    && matches!(&v.data, ObservationDraft::Agent { observation }
                        if observation.local_sequence == sequence)
            }) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "private fixture sample sequence {sequence} was not publicly projected"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
pub(super) async fn verify_live(
    f: &Fixture,
    cid: &str,
    sid: &str,
    correlation: &str,
    participant: &str,
    status: impl Fn() -> String,
    expected_work: Option<&ObservationWork>,
) {
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let (values, actual_page) = loop {
        let p = page(f, cid, sid).await;
        let values = p
            .observations
            .iter()
            .filter(|o| o.source.correlation_id == correlation)
            .cloned()
            .collect::<Vec<_>>();
        if values.len()>=9 && values.iter().any(|v|matches!(&v.data,ObservationDraft::Agent{observation} if matches!(&observation.content,AgentObservationContent::Transcript{text:Some(text),..} if text.len()==12_000))) {
            break (values,p);
        }
        if tokio::time::Instant::now() >= until {
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            let metadata=c.query_row("SELECT p.closed_at IS NOT NULL,p.uncertain,p.export_uncertain,o.state,v.state,p.delivered_at IS NOT NULL FROM private_provider_contexts p JOIN operations o ON o.record_id=p.operation_record_id JOIN native_consultation_legs l ON l.operation_record_id=o.record_id JOIN connections v ON v.id=l.connection_id WHERE p.delivery_id=?1",[correlation],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,bool>(1)?,r.get::<_,bool>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?)));
            panic!("live provider observations not projected: {}; status {}; private metadata {metadata:?}; visible routes {}",values.len(),status(),p.routes.len());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let observed_at = chrono::Utc::now().timestamp();
    let diagnostic = values
        .iter()
        .map(|v| match &v.data {
            ObservationDraft::Agent { observation } => {
                let content = match &observation.content {
                    AgentObservationContent::Transcript {
                        text,
                        speaker_role,
                        completeness,
                    } => format!(
                        "transcript role={speaker_role:?} completeness={completeness:?} bytes={:?}",
                        text.as_ref().map(String::len)
                    ),
                    AgentObservationContent::Action { phase, .. } => format!("action {phase:?}"),
                    AgentObservationContent::Speech { .. } => "speech".into(),
                    AgentObservationContent::Unavailable { issue } => {
                        format!("unavailable {issue:?}")
                    }
                    AgentObservationContent::ProviderStatus { .. } => "provider-status".into(),
                    AgentObservationContent::Gap { .. } => "gap".into(),
                };
                format!(
                    "sequence={} age={}s gap={:?} {content}",
                    observation.local_sequence,
                    observed_at - v.observed_at,
                    observation.missing_before
                )
            }
            _ => "unexpected observation kind".into(),
        })
        .collect::<Vec<_>>();
    let maximum_observation_age = values
        .iter()
        .map(|v| observed_at - v.observed_at)
        .max()
        .unwrap_or(0);
    let mut sequences = std::collections::BTreeSet::new();
    let mut requested = 0;
    let mut results = 0;
    let mut saw_user = false;
    let mut saw_large = false;
    let mut saw_missing = false;
    let mut saw_final_unknown_speaker = false;
    let mut saw_other_role = false;
    for value in values {
        assert_eq!(
            value.work.as_ref(),
            expected_work,
            "host question provenance must be independent of provider arguments"
        );
        assert_eq!(
            value.participant_id, participant,
            "provider user role cannot supply a human Participant"
        );
        let ObservationDraft::Agent { observation } = value.data else {
            panic!("provider data lost its typed provenance")
        };
        assert!(sequences.insert(observation.local_sequence));
        assert!(!observation.capture_complete);
        observation.validate().unwrap();
        match observation.content {
            AgentObservationContent::Transcript {
                text: Some(text),
                speaker_role: Some(AgentSpeakerRole::User),
                completeness: AgentCompleteness::Partial,
            } => {
                assert_eq!(text, "partial mixed-audio words");
                saw_user = true
            }
            AgentObservationContent::Transcript {
                text: Some(text), ..
            } if text.len() > 16 * 1024 / 2 => {
                assert_eq!(text, "\"".repeat(12_000));
                saw_large = true
            }
            AgentObservationContent::Transcript {
                text: Some(text),
                speaker_role: None,
                completeness: AgentCompleteness::Final,
            } => {
                assert_eq!(text, "final without role");
                saw_final_unknown_speaker = true;
            }
            AgentObservationContent::Transcript {
                text: Some(text),
                speaker_role: Some(AgentSpeakerRole::Other),
                completeness: AgentCompleteness::Final,
            } => {
                assert_eq!(text, "final other role");
                saw_other_role = true;
            }
            AgentObservationContent::Transcript {
                text: None,
                completeness: AgentCompleteness::Unavailable,
                ..
            } => saw_missing = true,
            AgentObservationContent::Action {
                phase: AgentActionPhase::Requested,
                call_id: Some(call),
                arguments: Some(args),
                ..
            } => {
                assert_eq!(call, "repeated-tool");
                assert_eq!(
                    observation.provider_event_id.as_deref(),
                    Some("repeated-provider-event")
                );
                assert_eq!(args["task_id"], "model-forged-task");
                requested += 1
            }
            AgentObservationContent::Action {
                phase: AgentActionPhase::ResultObserved,
                result: Some(_),
                ..
            } => results += 1,
            _ => {}
        }
    }
    assert!(saw_user && saw_large && saw_missing && saw_final_unknown_speaker && saw_other_role,
        "fixture observation classes missing: user={saw_user}, large={saw_large}, missing={saw_missing}, final_unknown={saw_final_unknown_speaker}, other={saw_other_role}; {diagnostic:?}");
    assert_eq!(requested, 2);
    assert_eq!(results, 1);
    if let Ok(root) = std::env::var("CONNECT_G6_AGENT_SAMPLE_ROOT") {
        let private=actual_page.observations.iter().any(|v|matches!(&v.data,ObservationDraft::Agent{observation} if observation.provider_session_id.as_deref()==Some("private-fixture-call")));
        let outcome = connect_protocol::Outcome::SessionObservations(actual_page);
        let response = json!({"profile":"connect-control/1","result":{"Ok":outcome}});
        std::fs::write(
            std::path::Path::new(&root).join(if private {
                "private.json"
            } else {
                "public.json"
            }),
            serde_json::to_vec_pretty(&response).unwrap(),
        )
        .unwrap();
    }
    println!("actual live provider observations: captured source Participant; partial/final/missing roles, >16KiB escaped metadata, repeated provider/tool IDs as distinct local sequences, requested vs reported result; no action dispatched; captured_sequences={} maximum_observation_age_seconds={maximum_observation_age}",sequences.len());
}
pub(super) async fn verify_closed(f: &Fixture, cid: &str, sid: &str, correlation: &str) {
    let p = page(f, cid, sid).await;
    assert!(!p.routes.iter().any(|r| r.correlation_id == correlation));
    assert!(!p
        .observations
        .iter()
        .any(|o| o.source.correlation_id == correlation));
}

pub(super) fn verify_proposal_tools(body: &Value) {
    let tools = body["assistantOverrides"]["model"]["tools"]
        .as_array()
        .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["async"], true);
    assert_eq!(tools[0]["function"]["name"], "handoff");
    assert!(tools[0].get("server").is_none());
    assert_eq!(body["assistantOverrides"]["serverMessages"], json!([]));
    assert!(body["assistantOverrides"]["clientMessages"]
        .as_array()
        .unwrap()
        .contains(&json!("tool-calls")));
}
