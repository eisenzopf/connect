//! Controlled Store custody, not physical provider capture or media evidence.
use super::*;
use crate::{
    AssistantProviderApplied, AssistantProviderContext, AssistantProviderCreated,
    SessionObservationSource,
};
use connect_protocol::{events::*, membership::*, messages::*, Outcome};

fn provider(
    f: &Fixture,
    target: &crate::NativeMediaOrigin,
    endpoint: &str,
) -> (
    AssistantProviderContext,
    Arc<AssistantProviderApplied>,
    AssistantProviderCreated,
) {
    provider_state(f, target, endpoint, "observation-instance", true)
}
fn provider_state(
    f: &Fixture,
    target: &crate::NativeMediaOrigin,
    endpoint: &str,
    operation: &str,
    delivered: bool,
) -> (
    AssistantProviderContext,
    Arc<AssistantProviderApplied>,
    AssistantProviderCreated,
) {
    let context = reserve(f, target, endpoint, operation);
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_assistant_provider(context.clone(), read)
        .unwrap();
    drop(read);
    let created = attempt.created(format!("provider-{operation}")).unwrap();
    f.store
        .record_native_assistant_provider_created(&created)
        .unwrap();
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    let json = read.context_json().unwrap().to_owned();
    let (export, read) = f
        .store
        .begin_native_assistant_export(context.clone(), read)
        .unwrap();
    drop(read);
    let applied = Arc::new(export.acknowledged(&json).unwrap());
    if delivered {
        f.store.record_native_assistant_delivered(&applied).unwrap();
    }
    (context, applied, created)
}
fn publisher(f: &Fixture) -> String {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "session.observation.publish".into(),
        "session.observation.publish.any".into(),
    ]);
    f.store
        .issue(
            &f.store.revalidate(&f.keys[0]).unwrap().identity_id,
            scope,
            crate::now() + 600,
        )
        .unwrap()
        .id
}
fn source(
    f: &Fixture,
    publisher: &str,
    context: &AssistantProviderContext,
    applied: Arc<AssistantProviderApplied>,
) -> SessionObservationSource {
    f.store
        .bind_native_assistant_observation_source(
            publisher,
            context,
            applied,
            ObservationCapabilities {
                transcript: true,
                ..Default::default()
            },
        )
        .unwrap()
}
fn transcript() -> ObservationDraft {
    ObservationDraft::Transcript {
        text: "provider content derived from its memory".into(),
        completeness: TranscriptCompleteness::Interim,
        turn_id: None,
        fragment_id: None,
    }
}
fn page(f: &Fixture) -> ObservationPage {
    f.store
        .session_observations(&f.keys[0], &f.cid, "session", ObservationQuery::default())
        .unwrap()
}
#[test]
fn observation_source_is_captured_generation_audience_and_cannot_be_rebound_as_generic() {
    let (f, target, endpoint, _) = setup(true);
    let (context, applied, _) = provider(&f, &target, &endpoint);
    let publisher = publisher(&f);
    let source = source(&f, &publisher, &context, applied.clone());
    let read = f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap();
    f.store
        .publish_native_assistant_observation(&source, &read, "event-one", transcript())
        .unwrap();
    let audience = read.target().connection.clone();
    drop(read);
    let page = page(&f);
    assert_eq!(page.observations.len(), 1);
    let item = &page.observations[0];
    assert_eq!(item.connection_id, context.connection_id());
    assert_eq!(item.generation, context.generation());
    assert_eq!(item.participant_id, audience.participant_id);
    assert_eq!(item.membership_epoch, audience.membership_epoch);
    assert_eq!(item.source.correlation_id, context.delivery_id());
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
                ObservationCapabilities {
                    transcript: true,
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let unauthorized = f
        .store
        .bind_native_assistant_observation_source(
            &f.keys[0],
            &context,
            applied,
            ObservationCapabilities::default(),
        )
        .unwrap_err();
    assert_eq!(unauthorized.code, "forbidden");
}
#[test]
fn provider_end_stops_publishing_and_hides_buffer_while_native_connection_remains_current() {
    let (f, target, endpoint, _) = setup(true);
    let (context, applied, created) = provider(&f, &target, &endpoint);
    let source = source(&f, &publisher(&f), &context, applied);
    f.store
        .publish_session_observation(&source, "event-one", transcript())
        .unwrap();
    assert_eq!(page(&f).observations.len(), 1);
    let _end = f.store.begin_native_assistant_end(&created).unwrap();
    assert!(f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap()
        .current());
    assert!(f
        .store
        .publish_session_observation(&source, "event-two", transcript())
        .is_err());
    let current = page(&f);
    assert!(current.routes.is_empty() && current.observations.is_empty());
}
#[test]
fn revoked_history_hides_already_buffered_derived_content_without_retiring_native_source() {
    let (f, target, endpoint, _) = setup(true);
    let Outcome::Accepted { message_id, .. } = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "source".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text(
                    MessageAudience::Participants(vec![f.pids[0].clone()]),
                    "revocable model memory",
                ),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let MembershipOutcome::Sharing { sharing_id, .. } = f
        .store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "share-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[0], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::ShareHistory {
                    resources: vec![ContentRef {
                        kind: ContentKind::Message,
                        id: message_id,
                    }],
                    participants: f.pids.iter().skip(1).cloned().collect(),
                    provenance: "fixture source grant".into(),
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let (context, applied, _) = provider(&f, &target, &endpoint);
    let source = source(&f, &publisher(&f), &context, applied.clone());
    f.store
        .publish_session_observation(&source, "event-one", transcript())
        .unwrap();
    assert_eq!(page(&f).observations.len(), 1);
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "revoke-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[0], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::RevokeHistoryShare { sharing_id },
            },
        )
        .unwrap();
    assert!(f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap()
        .current());
    assert!(f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .is_err());
    assert!(f
        .store
        .publish_session_observation(&source, "event-two", transcript())
        .is_err());
    let current = page(&f);
    assert!(current.observations.is_empty() && current.routes.is_empty());
}
#[test]
fn uncertain_update_blocks_observation_reads_even_when_initial_receipt_stays_delivered() {
    let (f, target, endpoint, _) = setup(true);
    let (context, applied, _) = provider(&f, &target, &endpoint);
    let source = source(&f, &publisher(&f), &context, applied.clone());
    f.store
        .publish_session_observation(&source, "event-one", transcript())
        .unwrap();
    let read = f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap();
    let update = f
        .store
        .prepare_native_assistant_update("observation-update", &context, &applied, read)
        .unwrap();
    assert_eq!(
        page(&f).observations.len(),
        1,
        "unsubmitted updates must not taint memory"
    );
    let read = f
        .store
        .try_native_assistant_update_read(&update, &applied)
        .unwrap();
    let (export, read) = f
        .store
        .begin_native_assistant_update(&update, read)
        .unwrap();
    drop(read);
    f.store.native_assistant_update_uncertain(&export).unwrap();
    let current = page(&f);
    assert!(current.observations.is_empty() && current.routes.is_empty());
    assert!(f
        .store
        .publish_session_observation(&source, "late", transcript())
        .is_err());
}
#[test]
fn original_memory_interval_publishes_without_reentering_behind_a_waiting_writer() {
    let (f, target, endpoint, _) = setup(true);
    let (context, applied, _) = provider(&f, &target, &endpoint);
    let source = source(&f, &publisher(&f), &context, applied.clone());
    let read = f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap();
    let write = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| {
            f.store.acquire_authority(
                &[write.clone()],
                &crate::AuthorityWait::new(std::time::Duration::from_secs(5)),
            )
        });
        crate::authority_fences::test_wait_for_writer(&f.store, &write);
        assert_eq!(
            f.store
                .publish_session_observation(&source, "non-reused", transcript())
                .unwrap_err()
                .code,
            "authority-busy"
        );
        f.store
            .publish_native_assistant_observation(&source, &read, "reused", transcript())
            .unwrap();
        let current = f
            .store
            .session_observations_held(
                &f.keys[0],
                &f.cid,
                "session",
                ObservationQuery::default(),
                Some(read.observation_interval(&context).unwrap()),
            )
            .unwrap();
        assert_eq!(current.observations.len(), 1);
        drop(read);
        writer.join().unwrap().unwrap();
    });
    assert_eq!(page(&f).observations.len(), 1);
}

#[test]
fn source_requires_committed_exact_ack_and_cannot_borrow_another_instance_receipt() {
    let (f, target, endpoint, _) = setup(true);
    let (context, applied, _) = provider_state(&f, &target, &endpoint, "not-committed", false);
    let publisher = publisher(&f);
    assert!(f
        .store
        .bind_native_assistant_observation_source(
            &publisher,
            &context,
            applied.clone(),
            ObservationCapabilities::default(),
        )
        .is_err());
    f.store.record_native_assistant_delivered(&applied).unwrap();
    let _source = source(&f, &publisher, &context, applied);
    let (other, receipt, _) = provider_state(&f, &target, &endpoint, "other-instance", true);
    assert_ne!(other.delivery_id(), context.delivery_id());
    assert!(f
        .store
        .bind_native_assistant_observation_source(
            &publisher,
            &context,
            receipt,
            ObservationCapabilities::default(),
        )
        .is_err());
}

#[test]
fn activity_busy_preserves_cursor_and_synchronous_closure_hides_buffer_without_cleanup_sql() {
    use std::sync::atomic::{AtomicU8, Ordering};
    struct Activity(AtomicU8);
    impl crate::SessionObservationActivity for Activity {
        fn check(&self) -> connect_protocol::Result<()> {
            match self.0.load(Ordering::Acquire) {
                0 => Ok(()),
                1 => Err(connect_protocol::Error::new(
                    "authority-busy",
                    "fixture contention",
                )),
                _ => Err(connect_protocol::Error::forbidden()),
            }
        }
    }
    let (f, target, endpoint, _) = setup(true);
    let (context, applied, _) = provider(&f, &target, &endpoint);
    let publisher = publisher(&f);
    let activity = Arc::new(Activity(AtomicU8::new(0)));
    let caps = ObservationCapabilities {
        transcript: true,
        ..Default::default()
    };
    activity.0.store(1, Ordering::Release);
    assert_eq!(
        f.store
            .bind_native_assistant_observation_source_with_activity(
                &publisher,
                &context,
                applied.clone(),
                caps.clone(),
                activity.clone()
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
    activity.0.store(0, Ordering::Release);
    let source = f
        .store
        .bind_native_assistant_observation_source_with_activity(
            &publisher,
            &context,
            applied.clone(),
            caps.clone(),
            activity.clone(),
        )
        .unwrap();
    let before = page(&f);
    f.store
        .publish_session_observation(&source, "one", transcript())
        .unwrap();
    activity.0.store(1, Ordering::Release);
    assert_eq!(
        f.store
            .session_observations(
                &f.keys[0],
                &f.cid,
                "session",
                ObservationQuery {
                    after: Some(before.cursor.clone()),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(
        f.store
            .publish_session_observation(&source, "two", transcript())
            .unwrap_err()
            .code,
        "authority-busy"
    );
    activity.0.store(0, Ordering::Release);
    let recovered = f
        .store
        .session_observations(
            &f.keys[0],
            &f.cid,
            "session",
            ObservationQuery {
                after: Some(before.cursor),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(recovered.observations.len(), 1);
    assert_eq!(
        f.store
            .bind_native_assistant_observation_source(
                &publisher,
                &context,
                applied.clone(),
                caps.clone()
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .bind_native_assistant_observation_source_with_activity(
                &publisher,
                &context,
                applied.clone(),
                caps,
                Arc::new(Activity(AtomicU8::new(0)))
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    activity.0.store(2, Ordering::Release);
    assert!(page(&f).observations.is_empty());
    assert!(f
        .store
        .publish_session_observation(&source, "late", transcript())
        .is_err());
    assert!(
        f.store
            .try_native_assistant_memory_read(&context, &applied)
            .is_ok(),
        "host closure is independent of persisted provider/native metadata"
    );
}

mod work_sources;
