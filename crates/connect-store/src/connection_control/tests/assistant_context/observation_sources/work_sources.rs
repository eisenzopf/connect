use super::*;
use connect_protocol::tasks::*;
use std::sync::atomic::{AtomicBool, Ordering};
struct Activity(AtomicBool);
impl crate::SessionObservationActivity for Activity {
    fn check(&self) -> connect_protocol::Result<()> {
        if self.0.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(connect_protocol::Error::forbidden())
        }
    }
}
fn question(f: &Fixture, wait: bool) -> (String, String, String) {
    let owner = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let key = f
        .store
        .issue(&owner, Scope::operator(), crate::now() + 1200)
        .unwrap();
    let recipients = f
        .keys
        .iter()
        .map(|k| f.store.revalidate(k).unwrap().identity_id)
        .collect();
    let TaskOutcome::Record(task) = f
        .store
        .execute_task(
            &key.id,
            &TaskRequest::new(TaskCommand::Create {
                request_id: "observation-task".into(),
                definition: TaskDefinition {
                    conversation_id: f.cid.clone(),
                    scope: TaskScope {
                        sms_enrollments: vec![],
                        operations: vec![TaskPermission::Send, TaskPermission::History],
                        recipients,
                    },
                    expires_at: crate::now() + 600,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap()
    else {
        panic!("task")
    };
    let result = f
        .store
        .execute_task(
            &key.id,
            &TaskRequest::new(TaskCommand::Invoke {
                request_id: "observation-question".into(),
                task_id: task.id.clone(),
                step_id: "ask".into(),
                operation: Box::new(Operation::Send {
                    request_id: "observation-question".into(),
                    conversation_id: f.cid.clone(),
                    message: MessageDraft::text_all("Explicit task question in provider memory"),
                }),
                reply_to: None,
                wait_for_reply: wait,
            }),
        )
        .unwrap();
    assert!(matches!(result, TaskOutcome::Tool(_)));
    let effect: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM task_effects WHERE task_id=?1",
            [&task.id],
            |r| r.get(0),
        )
        .unwrap();
    let TaskOutcome::Effect(saved) = f
        .store
        .execute_task(
            &key.id,
            &TaskRequest::new(TaskCommand::EffectGet {
                task_id: task.id.clone(),
                effect_id: effect.clone(),
            }),
        )
        .unwrap()
    else {
        panic!("effect")
    };
    let Outcome::Accepted { message_id, .. } = *saved.outcome.unwrap() else {
        panic!("message")
    };
    (task.id, effect, message_id)
}
fn work_publisher(f: &Fixture) -> String {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend(
        [
            "session.observation.publish",
            "session.observation.publish.any",
            "task_get",
            "task_effect_get",
        ]
        .map(str::to_owned),
    );
    f.store
        .issue(
            &f.store.revalidate(&f.keys[0]).unwrap().identity_id,
            scope,
            crate::now() + 1200,
        )
        .unwrap()
        .id
}
fn bind(
    f: &Fixture,
    p: &str,
    c: &AssistantProviderContext,
    a: Arc<AssistantProviderApplied>,
    activity: Arc<Activity>,
    work: Option<crate::TaskObservationBinding>,
) -> connect_protocol::Result<SessionObservationSource> {
    f.store.bind_native_assistant_observation_source_with_work(
        p,
        c,
        a,
        ObservationCapabilities {
            transcript: true,
            ..Default::default()
        },
        activity,
        work,
    )
}
#[test]
fn committed_question_work_labels_are_exact_and_require_actual_export() {
    let (f, target, endpoint, _) = setup(true);
    let (before, before_applied, _) =
        provider_state(&f, &target, &endpoint, "before-question", true);
    let (task, effect, question) = question(&f, true);
    let publisher = work_publisher(&f);
    let binding = f
        .store
        .prepare_task_observation_binding(&publisher, &task, &effect)
        .unwrap();
    assert_eq!(
        binding.descriptor(),
        &ObservationWork {
            task_id: task.clone(),
            effect_id: effect.clone(),
            question_id: question
        }
    );
    let live = Arc::new(Activity(AtomicBool::new(true)));
    assert_eq!(
        bind(
            &f,
            &publisher,
            &before,
            before_applied,
            live.clone(),
            Some(binding.clone())
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    assert!(f
        .store
        .prepare_task_observation_binding(&publisher, "unrelated-task", &effect)
        .is_err());
    let other = work_publisher(&f);
    let (context, applied, _) = provider_state(&f, &target, &endpoint, "after-question", true);
    assert_eq!(
        bind(
            &f,
            &other,
            &context,
            applied.clone(),
            live.clone(),
            Some(binding.clone())
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let source = bind(
        &f,
        &publisher,
        &context,
        applied,
        live.clone(),
        Some(binding.clone()),
    )
    .unwrap();
    f.store
        .publish_session_observation(&source, "event-one", transcript())
        .unwrap();
    let first = page(&f);
    assert_eq!(
        first.observations[0].work.as_ref(),
        Some(binding.descriptor())
    );
    assert_eq!(first.routes[0].work.as_ref(), Some(binding.descriptor()));
    let (second, receipt, _) = provider_state(&f, &target, &endpoint, "another-provider", true);
    let source = bind(
        &f,
        &publisher,
        &second,
        receipt,
        live,
        Some(binding.clone()),
    )
    .unwrap();
    f.store
        .publish_session_observation(&source, "event-two", transcript())
        .unwrap();
    let second = page(&f);
    assert_eq!(second.observations.len(), 2);
    assert_ne!(
        second.observations[0].source.correlation_id,
        second.observations[1].source.correlation_id
    );
    assert!(second
        .observations
        .iter()
        .all(|o| o.work.as_ref() == Some(binding.descriptor())));
}
#[test]
fn ordinary_non_question_send_cannot_create_work_binding() {
    let (f, _, _, _) = setup(true);
    let (task, effect, _) = question(&f, false);
    assert_eq!(
        f.store
            .prepare_task_observation_binding(&work_publisher(&f), &task, &effect)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn pruned_closed_route_cannot_drop_work_or_activity_or_become_generic() {
    let (f, target, endpoint, _) = setup(true);
    let (task, effect, _) = question(&f, true);
    let publisher = work_publisher(&f);
    let binding = f
        .store
        .prepare_task_observation_binding(&publisher, &task, &effect)
        .unwrap();
    let (context, applied, _) = provider(&f, &target, &endpoint);
    let activity = Arc::new(Activity(AtomicBool::new(true)));
    let captured = bind(
        &f,
        &publisher,
        &context,
        applied.clone(),
        activity.clone(),
        Some(binding.clone()),
    )
    .unwrap();
    f.store
        .publish_session_observation(&captured, "event", transcript())
        .unwrap();
    activity.0.store(false, Ordering::Release);
    assert!(page(&f).observations.is_empty());
    // Registering an unrelated source prunes the closed ephemeral route.
    let (other, receipt, _) = provider_state(&f, &target, &endpoint, "prune-trigger", true);
    let _other = source(&f, &publisher, &other, receipt);
    let live = Arc::new(Activity(AtomicBool::new(true)));
    assert_eq!(
        bind(
            &f,
            &publisher,
            &context,
            applied.clone(),
            live.clone(),
            None
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .bind_native_assistant_observation_source(
                &publisher,
                &context,
                applied.clone(),
                ObservationCapabilities::default()
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let members = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .participants
        .into_iter()
        .filter(|p| p.active)
        .map(|p| p.participant_id)
        .collect::<Vec<_>>();
    assert_eq!(
        f.store
            .bind_session_observation_source(
                &publisher,
                context.connection_id(),
                "vapi",
                context.delivery_id(),
                &members,
                ObservationCapabilities::default()
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    // A replacement with the same exact proof retains all restrictions.
    bind(&f, &publisher, &context, applied, live, Some(binding)).unwrap();
}

#[test]
fn current_task_inspection_revocation_hides_labels_and_prevents_rebinding_after_prune() {
    let (f, target, endpoint, _) = setup(true);
    let (task, effect, _) = question(&f, true);
    let publisher = work_publisher(&f);
    let binding = f
        .store
        .prepare_task_observation_binding(&publisher, &task, &effect)
        .unwrap();
    let (context, applied, _) = provider(&f, &target, &endpoint);
    let live = Arc::new(Activity(AtomicBool::new(true)));
    let captured = bind(
        &f,
        &publisher,
        &context,
        applied.clone(),
        live.clone(),
        Some(binding.clone()),
    )
    .unwrap();
    f.store
        .publish_session_observation(&captured, "before-revocation", transcript())
        .unwrap();
    assert_eq!(page(&f).observations.len(), 1);
    let mut scope = f.store.revalidate(&publisher).unwrap().scope;
    scope.operations.retain(|op| op != "task_effect_get");
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            rusqlite::params![publisher, serde_json::to_string(&scope.operations).unwrap()],
        )
        .unwrap();
    assert!(page(&f).observations.is_empty());
    assert_eq!(
        f.store
            .publish_session_observation(&captured, "after-revocation", transcript())
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap()
        .current());
    let (other, receipt, _) = provider_state(&f, &target, &endpoint, "prune-revoked-work", true);
    let _trigger = source(&f, &publisher, &other, receipt);
    assert_eq!(
        bind(
            &f,
            &publisher,
            &context,
            applied.clone(),
            live.clone(),
            None
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        bind(&f, &publisher, &context, applied, live, Some(binding))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
