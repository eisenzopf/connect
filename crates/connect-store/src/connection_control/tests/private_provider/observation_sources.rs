//! Controlled private Store custody; no physical adapter/provider claim.
use super::*;
use connect_protocol::events::*;
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
fn caps() -> ObservationCapabilities {
    ObservationCapabilities {
        transcript: true,
        ..Default::default()
    }
}
fn transcript() -> ObservationDraft {
    ObservationDraft::Transcript {
        text: "derived private consultation memory".into(),
        completeness: TranscriptCompleteness::Interim,
        turn_id: None,
        fragment_id: None,
    }
}
fn page(f: &Fixture, n: usize) -> ObservationPage {
    f.store
        .session_observations(&f.keys[n], &f.cid, "session", ObservationQuery::default())
        .unwrap()
}

#[test]
fn captured_private_source_keeps_exact_pair_and_closes_without_retiring_native_metadata() {
    let (f, sources, candidate, origin, _, endpoint, native) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let applied = Arc::new(delivered(&f, &context));
    f.store.record_native_consultation_decoded(&native).unwrap();
    f.store.complete_native_consultation(&native).unwrap();
    let publisher = publisher(&f);
    let source = f
        .store
        .bind_private_provider_observation_source(&publisher, &context, applied.clone(), caps())
        .unwrap();
    let read = f
        .store
        .try_private_provider_media_read(&context, &applied)
        .unwrap();
    assert_eq!(read.connection_id(), candidate.connection_id());
    assert_eq!(read.generation(), candidate.generation());
    f.store
        .publish_private_provider_observation(&source, &read, "one", transcript())
        .unwrap();
    drop(read);
    for n in [0, 2] {
        let p = page(&f, n);
        assert_eq!(p.observations.len(), 1);
        assert_eq!(p.observations[0].participant_id, candidate.participant_id());
        assert_eq!(
            p.observations[0].source.correlation_id,
            context.delivery_id()
        );
    }
    for n in [1, 3] {
        let p = page(&f, n);
        assert!(p.observations.is_empty() && p.routes.is_empty());
    }
    assert!(f
        .store
        .bind_private_provider_observation_source(&f.keys[0], &context, applied.clone(), caps())
        .is_err());
    f.store.record_private_provider_closed(&applied).unwrap();
    assert!(page(&f, 0).observations.is_empty());
    assert!(f
        .store
        .publish_session_observation(&source, "late", transcript())
        .is_err());
    assert!(f.store.try_native_media_read(&sources[0], None).is_ok());
}
#[test]
fn private_source_requires_committed_exact_receipt_and_reuses_interval_before_waiting_writer() {
    let (f, sources, candidate, origin, _, endpoint, native) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    created(&f, &context, "captured-private");
    let applied = Arc::new(export_attempt(&f, &context).delivered());
    let publisher = publisher(&f);
    assert!(f
        .store
        .bind_private_provider_observation_source(&publisher, &context, applied.clone(), caps())
        .is_err());
    f.store.record_private_provider_delivered(&applied).unwrap();
    f.store.record_native_consultation_decoded(&native).unwrap();
    f.store.complete_native_consultation(&native).unwrap();
    let source = f
        .store
        .bind_private_provider_observation_source(&publisher, &context, applied.clone(), caps())
        .unwrap();
    let (other, others, other_candidate, other_origin, other_endpoint) = fresh_fixture();
    let other_context = prepared(
        &other,
        &others,
        &other_candidate,
        &other_origin,
        &other_endpoint,
    );
    let other_applied = Arc::new(delivered(&other, &other_context));
    assert!(f
        .store
        .bind_private_provider_observation_source(
            &publisher,
            &context,
            other_applied.clone(),
            caps()
        )
        .is_err());
    let foreign_read = other
        .store
        .try_private_provider_media_read(&other_context, &other_applied)
        .unwrap();
    assert!(f
        .store
        .publish_private_provider_observation(&source, &foreign_read, "foreign", transcript())
        .is_err());
    drop(foreign_read);
    let read = f
        .store
        .try_private_provider_media_read(&context, &applied)
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
                .publish_session_observation(&source, "no-reuse", transcript())
                .unwrap_err()
                .code,
            "authority-busy"
        );
        f.store
            .publish_private_provider_observation(&source, &read, "reused", transcript())
            .unwrap();
        let p = f
            .store
            .session_observations_held(
                &f.keys[0],
                &f.cid,
                "session",
                ObservationQuery::default(),
                Some(read.observation_interval(&context, &applied).unwrap()),
            )
            .unwrap();
        assert_eq!(p.observations.len(), 1);
        drop(read);
        writer.join().unwrap().unwrap();
    });
    assert_eq!(page(&f, 0).observations.len(), 1);
}
#[test]
fn private_source_rejects_uncertain_memory_and_stale_generation_before_buffer_replay() {
    for uncertainty in [true, false] {
        let (f, sources, candidate, origin, _, endpoint, _) = fixture();
        let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
        let applied = Arc::new(delivered(&f, &context));
        let source = f
            .store
            .bind_private_provider_observation_source(
                &publisher(&f),
                &context,
                applied.clone(),
                caps(),
            )
            .unwrap();
        f.store
            .publish_session_observation(&source, "one", transcript())
            .unwrap();
        assert_eq!(page(&f, 0).observations.len(), 1);
        {
            let c = f.store.connection().unwrap();
            if uncertainty {
                c.execute(
                    "UPDATE private_provider_contexts SET export_uncertain=1 WHERE delivery_id=?1",
                    [context.delivery_id()],
                )
                .unwrap();
            } else {
                c.execute(
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    [candidate.connection_id()],
                )
                .unwrap();
            }
        }
        assert!(page(&f, 0).observations.is_empty());
        assert!(f
            .store
            .publish_session_observation(&source, "late", transcript())
            .is_err());
        assert!(f
            .store
            .try_private_provider_media_read(&context, &applied)
            .is_err());
    }
}

#[test]
fn private_buffer_is_hidden_when_original_human_listener_loses_exported_history() {
    use connect_protocol::{membership::*, messages::*, Operation, Outcome};
    let (f, sources, candidate, origin, _, endpoint, _) = fixture();
    let Outcome::Accepted { message_id, .. } = f
        .store
        .execute(
            &f.keys[2],
            &Operation::Send {
                request_id: "private-memory-source".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text(
                    MessageAudience::Participants(vec![f.pids[2].clone()]),
                    "revocable memory for private listener",
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
            &f.keys[2],
            MembershipRequest {
                operation_id: "private-listener-grant".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[2], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::ShareHistory {
                    resources: vec![ContentRef {
                        kind: ContentKind::Message,
                        id: message_id,
                    }],
                    participants: vec![f.pids[0].clone()],
                    provenance: "controlled listener history grant".into(),
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let context = f
        .store
        .prepare_private_provider_context(
            read(&f, &sources, &candidate, &origin),
            &f.keys[2],
            &endpoint,
        )
        .unwrap();
    let applied = Arc::new(delivered(&f, &context));
    let source = f
        .store
        .bind_private_provider_observation_source(&publisher(&f), &context, applied.clone(), caps())
        .unwrap();
    f.store
        .publish_session_observation(&source, "one", transcript())
        .unwrap();
    assert_eq!(page(&f, 0).observations.len(), 1);
    f.store
        .execute_membership(
            &f.keys[2],
            MembershipRequest {
                operation_id: "revoke-private-listener-memory".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[2], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::RevokeHistoryShare { sharing_id },
            },
        )
        .unwrap();
    assert!(page(&f, 0).observations.is_empty());
    assert!(
        page(&f, 2).observations.is_empty(),
        "AI's own continued access cannot grant the original human listener access"
    );
    assert!(f
        .store
        .publish_session_observation(&source, "late", transcript())
        .is_err());
    assert!(
        f.store.try_native_media_read(&sources[0], None).is_ok(),
        "current primary/native metadata is not current provider memory authority"
    );
}

#[test]
fn final_callback_try_does_not_queue_writer_behind_current_media_interval() {
    let (f, sources, candidate, origin, _, endpoint, native) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let applied = delivered(&f, &context);
    f.store.record_native_consultation_decoded(&native).unwrap();
    f.store.complete_native_consultation(&native).unwrap();
    let read = f
        .store
        .try_private_provider_media_read(&context, &applied)
        .unwrap();
    let began = std::time::Instant::now();
    assert_eq!(
        f.store
            .try_record_private_provider_transcript(
                &context,
                &applied,
                "busy-callback",
                "assistant",
                "final",
                "actual final"
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(began.elapsed() < std::time::Duration::from_millis(200));
    assert!(read.current());
    drop(read);
    // No pending writer is retained, and retry preserves one actual callback.
    assert!(f
        .store
        .try_private_provider_media_read(&context, &applied)
        .unwrap()
        .current());
    let one = f
        .store
        .try_record_private_provider_transcript(
            &context,
            &applied,
            "busy-callback",
            "assistant",
            "final",
            "actual final",
        )
        .unwrap()
        .unwrap();
    let again = f
        .store
        .try_record_private_provider_transcript(
            &context,
            &applied,
            "busy-callback",
            "assistant",
            "final",
            "actual final",
        )
        .unwrap()
        .unwrap();
    assert_eq!(one.id, again.id);
}
#[test]
fn final_callback_try_refuses_foreign_sql_writer_without_holding_process_sql_mutex() {
    let (f, sources, candidate, origin, _, endpoint, native) = fixture();
    let context = prepared(&f, &sources, &candidate, &origin, &endpoint);
    let applied = delivered(&f, &context);
    f.store.record_native_consultation_decoded(&native).unwrap();
    f.store.complete_native_consultation(&native).unwrap();
    let c = rusqlite::Connection::open(f.store.path()).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let began = std::time::Instant::now();
    let error = f
        .store
        .try_record_private_provider_transcript(
            &context,
            &applied,
            "sql-callback",
            "assistant",
            "final",
            "actual final",
        )
        .unwrap_err();
    assert!(matches!(
        error.code.as_str(),
        "authority-busy" | "storage-unavailable"
    ));
    assert!(began.elapsed() < std::time::Duration::from_millis(200));
    assert!(f
        .store
        .try_private_provider_media_read(&context, &applied)
        .unwrap()
        .current());
    c.execute_batch("ROLLBACK").unwrap();
    let one = f
        .store
        .try_record_private_provider_transcript(
            &context,
            &applied,
            "sql-callback",
            "assistant",
            "final",
            "actual final",
        )
        .unwrap()
        .unwrap();
    let again = f
        .store
        .try_record_private_provider_transcript(
            &context,
            &applied,
            "sql-callback",
            "assistant",
            "final",
            "actual final",
        )
        .unwrap()
        .unwrap();
    assert_eq!(one.id, again.id);
}
