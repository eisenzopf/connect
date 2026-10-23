//! Controlled origins: phase custody, complete cumulative memory and recovery.
use super::*;
use connect_protocol::{messages::*, ContextDeliveryPhase};
fn send(f: &Fixture, id: &str, body: &str) {
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: id.into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text_all(body),
            },
        )
        .unwrap();
}
fn root(
    f: &Fixture,
    target: &crate::NativeMediaOrigin,
    endpoint: &str,
) -> (
    crate::AssistantProviderContext,
    crate::AssistantProviderApplied,
) {
    let context = reserve(f, target, endpoint, "phase-root");
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_assistant_provider(context.clone(), read)
        .unwrap();
    drop(read);
    let created = attempt.created("phase-root-call".into()).unwrap();
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
    let applied = export.acknowledged(&json).unwrap();
    drop(read);
    f.store.record_native_assistant_delivered(&applied).unwrap();
    (context, applied)
}
fn prepare(
    f: &Fixture,
    context: &crate::AssistantProviderContext,
    applied: &crate::AssistantProviderApplied,
    id: &str,
) -> crate::AssistantProviderUpdate {
    let read = f
        .store
        .try_native_assistant_memory_read(context, applied)
        .unwrap();
    f.store
        .prepare_native_assistant_update(id, context, applied, read)
        .unwrap()
}
fn apply(
    f: &Fixture,
    update: &crate::AssistantProviderUpdate,
    applied: &crate::AssistantProviderApplied,
) -> String {
    let read = f
        .store
        .try_native_assistant_update_read(update, applied)
        .unwrap();
    let json = read.context_json().unwrap().to_owned();
    let (export, read) = f.store.begin_native_assistant_update(update, read).unwrap();
    let ack = export.acknowledged(&json).unwrap();
    drop(read);
    f.store
        .record_native_assistant_update_delivered(&ack)
        .unwrap();
    json
}
fn receipt(f: &Fixture, id: &str) -> connect_protocol::ContextDelivery {
    let Outcome::ContextDeliveries(page) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ContextDeliveries {
                conversation_id: f.cid.clone(),
                before: None,
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    page.deliveries.into_iter().find(|d| d.id == id).unwrap()
}
fn reserve_error(
    f: &Fixture,
    target: &crate::NativeMediaOrigin,
    endpoint: &str,
    id: &str,
) -> String {
    let read = f
        .store
        .try_native_assistant_context_read(target, &f.keys[0], endpoint)
        .unwrap();
    f.store
        .prepare_native_assistant_provider(id, read)
        .unwrap_err()
        .code
}
#[test]
fn initialization_reserves_inline_export_before_reference_and_never_repeats_or_downgrades() {
    let (f, target, endpoint, _) = setup(true);
    send(&f, "init-history", "INITIALIZATION-HISTORY");
    let selection = f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap();
    let context = f
        .store
        .prepare_native_assistant_provider_timed(
            "initialization",
            selection,
            crate::AssistantProviderTiming::Initialization,
        )
        .unwrap();
    assert_eq!(
        context.timing(),
        crate::AssistantProviderTiming::Initialization
    );
    assert_eq!(
        reserve_error(&f, &target, &endpoint, "initialization"),
        "conflict"
    );
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    assert_eq!(read.context_json().unwrap_err().code, "not-ready");
    let (init, read) = f
        .store
        .begin_native_assistant_initialization(context.clone(), read)
        .unwrap();
    let json = read.context_json().unwrap().to_owned();
    assert!(json.contains("INITIALIZATION-HISTORY"));
    drop(read);
    assert!(f
        .store
        .try_native_assistant_provider_read(&context)
        .is_err());
    let pending = receipt(&f, context.delivery_id());
    assert!(pending.provider_ref.is_none());
    let lifecycle = pending.lifecycle.unwrap();
    assert_eq!(lifecycle.phase, ContextDeliveryPhase::Initialization);
    assert!(
        lifecycle.creation_admitted
            && lifecycle.export_admitted
            && !lifecycle.creation_confirmed
            && !lifecycle.delivery_confirmed
    );
    let created = init.attempt().created("inline-actual-call".into()).unwrap();
    f.store
        .record_native_assistant_provider_created(&created)
        .unwrap();
    let read = f
        .store
        .try_native_assistant_initialization_read(&context)
        .unwrap();
    assert_eq!(read.context_json().unwrap(), json);
    let export = init.export_for(&created).unwrap();
    assert!(export.acknowledged("truncated").is_err());
    assert!(f
        .store
        .begin_native_assistant_export(context.clone(), read)
        .is_err());
    let applied = export.acknowledged(&json).unwrap();
    f.store.record_native_assistant_delivered(&applied).unwrap();
    assert!(f
        .store
        .try_native_assistant_initialization_read(&context)
        .is_err());
    assert!(f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap()
        .current());
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    f.store.record_native_assistant_delivered(&applied).unwrap();
    assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
}
#[test]
fn unknown_inline_creation_keeps_possible_export_visible_without_an_adoptable_instance() {
    let (f, target, endpoint, _) = setup(true);
    let read = f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap();
    let context = f
        .store
        .prepare_native_assistant_provider_timed(
            "lost-inline-create",
            read,
            crate::AssistantProviderTiming::Initialization,
        )
        .unwrap();
    let read = f
        .store
        .try_native_assistant_provider_read(&context)
        .unwrap();
    let (init, read) = f
        .store
        .begin_native_assistant_initialization(context.clone(), read)
        .unwrap();
    drop(read);
    f.store
        .native_assistant_creation_uncertain(init.attempt())
        .unwrap();
    let result = receipt(&f, context.delivery_id());
    assert_eq!(result.state, "uncertain");
    assert_eq!(
        result.problem,
        Some(connect_protocol::ContextDeliveryProblem::Unknown)
    );
    let lifecycle = result.lifecycle.unwrap();
    assert!(
        lifecycle.export_admitted && !lifecycle.creation_confirmed && !lifecycle.delivery_confirmed
    );
    assert!(result.provider_ref.is_none());
    assert!(f
        .store
        .try_native_assistant_provider_read(&context)
        .is_err());
    assert!(f
        .store
        .try_native_assistant_initialization_read(&context)
        .is_err());
}
#[test]
fn updates_freeze_independent_receipts_and_replay_without_overwriting_initial_selection() {
    let (f, target, endpoint, _) = setup(true);
    send(&f, "original", "ORIGINAL-ROOT");
    let (context, applied) = root(&f, &target, &endpoint);
    send(&f, "first-update", "UPDATED-HISTORY");
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let update = prepare(&f, &context, &applied, "update-one");
    assert_ne!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let replay = prepare(&f, &context, &applied, "update-one");
    assert_eq!(replay.delivery_id(), update.delivery_id());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(
        reserve_error(&f, &target, &endpoint, "update-one"),
        "conflict"
    );
    send(&f, "after-update-selection", "NOT-IN-FROZEN-UPDATE");
    let json = apply(&f, &update, &applied);
    assert!(json.contains("UPDATED-HISTORY"));
    assert!(!json.contains("NOT-IN-FROZEN-UPDATE"));
    let original = f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap();
    assert!(!original.context_json().unwrap().contains("UPDATED-HISTORY"));
    drop(original);
    let result = receipt(&f, update.delivery_id());
    assert_eq!(result.state, "delivered");
    let lifecycle = result.lifecycle.unwrap();
    assert_eq!(lifecycle.phase, ContextDeliveryPhase::Update);
    assert_eq!(
        lifecycle.parent_delivery_id.as_deref(),
        Some(context.delivery_id())
    );
    assert!(
        !lifecycle.creation_admitted
            && lifecycle.creation_confirmed
            && lifecycle.delivery_confirmed
    );
    assert!(f
        .store
        .try_native_assistant_update_read(&update, &applied)
        .is_err());
    assert_eq!(
        prepare(&f, &context, &applied, "update-one").delivery_id(),
        update.delivery_id()
    );
}
#[test]
fn every_earlier_update_remains_authorized_when_its_source_falls_out_of_latest_context() {
    use connect_protocol::membership::*;
    let (f, target, endpoint, _) = setup(true);
    send(&f, "root-source", "ROOT-ONLY-HISTORY");
    let (context, applied) = root(&f, &target, &endpoint);
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "unseen-update".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text(
                    MessageAudience::Participants(vec![f.pids[0].clone()]),
                    "PREVIOUSLY-HIDDEN-SOURCE",
                ),
            },
        )
        .unwrap();
    let replacement: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM messages WHERE body='PREVIOUSLY-HIDDEN-SOURCE'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "shared-update".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text(
                    MessageAudience::Participants(vec![f.pids[0].clone()]),
                    "REVOCABLE-UPDATE-MEMORY",
                ),
            },
        )
        .unwrap();
    let message: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM messages WHERE body='REVOCABLE-UPDATE-MEMORY'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let MembershipOutcome::Sharing { sharing_id, .. } = f
        .store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "share-update-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[0], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::ShareHistory {
                    resources: vec![ContentRef {
                        kind: ContentKind::Message,
                        id: message,
                    }],
                    participants: f.pids.iter().skip(1).cloned().collect(),
                    provenance: "explicit shared public update".into(),
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let first = prepare(&f, &context, &applied, "shared-first");
    assert!(apply(&f, &first, &applied).contains("REVOCABLE-UPDATE-MEMORY"));
    for i in 0..3 {
        send(
            &f,
            &format!("filler-{i}"),
            &format!("RECENT-{i}:{}", "x".repeat(12_000)),
        );
    }
    let second = prepare(&f, &context, &applied, "recent-second");
    let latest = apply(&f, &second, &applied);
    assert!(!latest.contains("REVOCABLE-UPDATE-MEMORY"));
    assert!(f
        .store
        .try_native_assistant_memory_read(&context, &applied)
        .unwrap()
        .current());
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "revoke-update-source".into(),
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
    // Exchange visibility of two older omitted sources. The latest page's
    // items, watermark and omission count stay identical, so checking only the
    // original and latest context would miss the revoked intermediate memory.
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "share-replacement-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.keys[0], &f.cid)
                    .unwrap()
                    .revision,
                change: MembershipAction::ShareHistory {
                    resources: vec![ContentRef {
                        kind: ContentKind::Message,
                        id: replacement,
                    }],
                    participants: f.pids.iter().skip(1).cloned().collect(),
                    provenance: "balanced current visibility regression".into(),
                },
            },
        )
        .unwrap();
    let original_json: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT selection_json FROM context_deliveries WHERE id=?1",
            [context.delivery_id()],
            |r| r.get(0),
        )
        .unwrap();
    let preview = f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    {
        let c = f.store.connection().unwrap();
        let delegate = f.store.native_media_route_actor(&c, &target).unwrap();
        for saved in [&original_json, &latest] {
            let saved: connect_protocol::ContextView = serde_json::from_str(saved).unwrap();
            let current = crate::context::view_for_audience_scope_at(
                &f.store,
                &c,
                &actor,
                &f.cid,
                1000,
                Some(&delegate.identity_id),
                Some(&delegate),
                Some(saved.watermark),
                None,
                Some(preview.audience()),
            )
            .unwrap();
            assert_eq!(
                current, saved,
                "original/latest projections must stay identical in this regression"
            );
        }
    }
    drop(preview);
    assert!(f
        .store
        .try_native_assistant_context_read(&target, &f.keys[0], &endpoint)
        .unwrap()
        .current());
    assert_eq!(
        f.store
            .try_native_assistant_memory_read(&context, &applied)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn failed_or_uncertain_update_blocks_memory_and_exact_late_observations_never_create_another_attempt(
) {
    for truncated in [false, true] {
        let (f, target, endpoint, _) = setup(true);
        let (context, applied) = root(&f, &target, &endpoint);
        send(&f, "pending-source", "PENDING-UPDATE");
        let update = prepare(&f, &context, &applied, "update-observation");
        let read = f
            .store
            .try_native_assistant_update_read(&update, &applied)
            .unwrap();
        let json = read.context_json().unwrap().to_owned();
        let (export, read) = f
            .store
            .begin_native_assistant_update(&update, read)
            .unwrap();
        drop(read);
        assert!(f
            .store
            .finish_context_delivery(update.delivery_id(), "delivered", Some("phase-root-call"))
            .is_err());
        assert!(f
            .store
            .try_native_assistant_memory_read(&context, &applied)
            .is_err());
        assert!(f
            .store
            .try_native_assistant_update_read(&update, &applied)
            .is_err());
        if truncated {
            f.store
                .record_native_assistant_update_rejected(
                    &export.rejected(crate::AssistantContextFailure::Truncated),
                )
                .unwrap();
            assert!(f
                .store
                .record_native_assistant_update_delivered(&export.acknowledged(&json).unwrap())
                .is_err());
            assert_eq!(
                receipt(&f, update.delivery_id()).problem,
                Some(connect_protocol::ContextDeliveryProblem::Truncated)
            );
        } else {
            f.store.native_assistant_update_uncertain(&export).unwrap();
            let before = Store::inspect_snapshot(f.store.path()).unwrap();
            f.store.native_assistant_update_uncertain(&export).unwrap();
            assert_eq!(before, Store::inspect_snapshot(f.store.path()).unwrap());
            f.store
                .record_native_assistant_update_delivered(&export.acknowledged(&json).unwrap())
                .unwrap();
            assert_eq!(receipt(&f, update.delivery_id()).state, "delivered");
        }
        assert!(f
            .store
            .try_native_assistant_memory_read(&context, &applied)
            .is_err());
    }
}
#[test]
fn update_recovery_is_metadata_only_and_distinguishes_zero_io_from_possible_export() {
    for started in [false, true] {
        let (f, target, endpoint, _) = setup(true);
        let (context, applied) = root(&f, &target, &endpoint);
        let update = prepare(&f, &context, &applied, "update-recovery");
        if started {
            let read = f
                .store
                .try_native_assistant_update_read(&update, &applied)
                .unwrap();
            let (_, read) = f
                .store
                .begin_native_assistant_update(&update, read)
                .unwrap();
            drop(read);
            f.store.connection().unwrap().execute("UPDATE native_assistant_context_updates SET export_expires_at=1 WHERE delivery_id=?1",[update.delivery_id()]).unwrap();
        } else {
            let expires:i64=f.store.connection().unwrap().query_row("SELECT prepared_expires_at FROM native_assistant_context_updates WHERE delivery_id=?1",[update.delivery_id()],|r|r.get(0)).unwrap();
            while crate::now() < expires {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        assert_eq!(f.store.recover_native_assistant_providers(1).unwrap(), 1);
        assert_eq!(f.store.recover_native_assistant_providers(1).unwrap(), 0);
        assert_eq!(
            receipt(&f, update.delivery_id()).state,
            if started { "uncertain" } else { "failed" }
        );
        assert_eq!(
            f.store
                .try_native_assistant_memory_read(&context, &applied)
                .is_ok(),
            !started
        );
    }
}
