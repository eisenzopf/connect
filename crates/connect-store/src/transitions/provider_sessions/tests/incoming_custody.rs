use super::*;
use connect_protocol::incoming_contacts::{IncomingContactReason, IncomingContactState};

pub(super) fn another_room(
    f: &Fixture,
    w: &Arc<ProviderSessionVoiceWorker>,
    name: &str,
) -> ProviderVoiceIngressPolicy {
    let Outcome::Conversation(room) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Create {
                request_id: format!("create-{name}"),
                request: connect_protocol::conversations::CreateRequest::persistent(
                    f._ids[1..].to_vec(),
                ),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let p = ProviderVoiceIngressPolicy {
        id: name.into(),
        conversation_id: room.id,
        policy: f.policy.clone(),
    };
    f.store
        .configure_provider_voice_ingress(w, p.clone())
        .unwrap();
    p
}
// The fixture provider reports one original creation time after all configured
// policies exist, independent of each discovery query's cursor/since fields.
pub(super) fn incoming_import(
    f: &Fixture,
    w: &Arc<ProviderSessionVoiceWorker>,
    p: &ProviderVoiceIngressPolicy,
    id: &str,
    state: &str,
) -> ProviderVoiceImport {
    let at: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT max(since) FROM provider_voice_ingress", [], |r| {
            r.get(0)
        })
        .unwrap();
    let scan = f
        .store
        .begin_provider_voice_scan(w, p, &manifest())
        .unwrap();
    f.store
        .record_provider_voice_scan(&scan.capture(vec![incoming_receipt(at, id, state)]).unwrap())
        .unwrap()
}
fn page(f: &Fixture) -> connect_protocol::incoming_contacts::IncomingContactPage {
    f.store.incoming_contacts(&f.keys[0], None, 32).unwrap()
}
fn public_counts(f: &Fixture) -> (u64, u64, u64, u64, u64) {
    (
        f.count("sessions"),
        f.count("operations"),
        f.count("journal"),
        f.count("provider_session_voice"),
        f.count("operation_effects"),
    )
}

#[test]
fn overlap_is_held_before_any_session_from_either_scanner() {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let b = another_room(&f, &w, "second");
    let before = public_counts(&f);
    for p in [&a, &b, &a] {
        let result = incoming_import(&f, &w, p, "overlapping-resource", "in-progress");
        assert_eq!(result.imported, 0);
        assert!(result.calls.is_empty());
        assert_eq!(public_counts(&f), before);
    }
    let contacts = page(&f);
    assert_eq!(contacts.contacts.len(), 1);
    let contact = &contacts.contacts[0];
    assert_eq!(contact.reason, IncomingContactReason::AmbiguousRoute);
    assert_eq!(contact.state, IncomingContactState::Held);
    assert_eq!(contact.candidates.len(), 2);
    assert!(contact
        .candidates
        .iter()
        .any(|c| c.conversation_id == a.conversation_id));
    assert!(contact
        .candidates
        .iter()
        .any(|c| c.conversation_id == b.conversation_id));
    let raw = serde_json::to_string(&contacts).unwrap();
    assert!(!raw.contains("overlapping-resource"));
    assert!(!raw.contains("/control"));
    assert_eq!(f.count("incoming_contact_resources"), 1);
    assert_eq!(f.count("incoming_contact_audit"), 1);
}

#[test]
fn held_call_cannot_narrow_into_remaining_room_after_closure() {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let b = another_room(&f, &w, "second");
    incoming_import(&f, &w, &a, "held-resource", "in-progress");
    let original = page(&f);
    f.store
        .execute(
            &f.keys[0],
            &Operation::Close {
                request_id: "close-second".into(),
                conversation_id: b.conversation_id,
                options: Default::default(),
            },
        )
        .unwrap();
    let before = public_counts(&f);
    let retry = incoming_import(&f, &w, &a, "held-resource", "ended");
    assert_eq!(retry.imported, 0);
    assert!(retry.calls.is_empty());
    assert_eq!(page(&f), original);
    assert_eq!(public_counts(&f), before);
    assert_eq!(f.count("incoming_contact_holds"), 1);
}

#[test]
fn preexisting_withdrawn_route_still_prevents_new_call_from_choosing_a_room() {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let b = another_room(&f, &w, "second");
    f.store
        .execute(
            &f.keys[0],
            &Operation::Close {
                request_id: "close-second".into(),
                conversation_id: b.conversation_id,
                options: Default::default(),
            },
        )
        .unwrap();
    let before = public_counts(&f);
    let batch = incoming_import(&f, &w, &a, "late-resource", "in-progress");
    assert!(batch.calls.is_empty());
    assert_eq!(public_counts(&f), before);
    let contact = page(&f).contacts.remove(0);
    assert_eq!(contact.reason, IncomingContactReason::HistoricalOverlap);
    assert_eq!(contact.candidates.len(), 1);
}

#[test]
fn unique_call_keeps_one_attributed_delivery_while_lifecycle_observations_change() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "unique");
    let first = incoming_import(&f, &w, &p, "unique-resource", "in-progress");
    assert_eq!(first.imported, 1);
    let before = public_counts(&f);
    let original = page(&f);
    let contact = &original.contacts[0];
    assert_eq!(contact.state, IncomingContactState::Resolved);
    let decision = contact.decision.as_ref().unwrap();
    assert_eq!(decision.actor_identity_id, f._ids[0]);
    assert_eq!(decision.actor_credential_id, f.keys[0]);
    assert_eq!(
        decision.session_id.as_deref(),
        Some(first.calls[0].session_id())
    );
    assert!(decision.message_id.is_none());
    let retry = incoming_import(&f, &w, &p, "unique-resource", "ended");
    assert_eq!(retry.imported, 0);
    assert_eq!(retry.calls[0].id(), first.calls[0].id());
    assert_eq!(public_counts(&f), before);
    assert_eq!(page(&f), original);
    assert_eq!(f.count("incoming_contact_holds"), 1);
    assert_eq!(f.count("incoming_contact_deliveries"), 1);
    assert_eq!(f.count("incoming_contact_audit"), 2);
}

#[test]
fn a_reused_provider_id_with_different_caller_is_held_without_rebinding() {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let first = incoming_import(&f, &w, &a, "reused-resource", "in-progress");
    let c = f.store.connection().unwrap();
    c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) SELECT 'other-phone',identity_id,tenant_id,device_id,transport,'+15555550112',capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id FROM endpoints WHERE id='initial-phone'",[]).unwrap();
    drop(c);
    let mut b = ProviderVoiceIngressPolicy {
        id: "other-caller".into(),
        conversation_id: a.conversation_id.clone(),
        policy: a.policy.clone(),
    };
    b.policy.endpoint_id = "other-phone".into();
    f.store
        .configure_provider_voice_ingress(&w, b.clone())
        .unwrap();
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &b, &manifest())
        .unwrap();
    let since = scan.query().unwrap().since;
    let mut receipt = incoming_receipt(since, "reused-resource", "in-progress");
    receipt.call.caller = "+15555550112".into();
    receipt.event_id = "other-caller-observation".into();
    let before = public_counts(&f);
    let batch = f
        .store
        .record_provider_voice_scan(&scan.capture(vec![receipt]).unwrap())
        .unwrap();
    assert!(batch.calls.is_empty());
    assert_eq!(public_counts(&f), before);
    let contacts = page(&f);
    assert_eq!(contacts.contacts.len(), 2);
    assert!(contacts
        .contacts
        .iter()
        .any(|c| c.reason == IncomingContactReason::ResourceConflict
            && c.state == IncomingContactState::Held));
    assert_eq!(f.count("incoming_contact_deliveries"), 1);
    assert_eq!(
        f.store
            .provider_session_voice_view(&f.keys[0], first.calls[0].id())
            .unwrap()
            .state,
        ProviderCallState::Active
    );
}

#[test]
fn contradictory_same_event_rolls_back_custody_and_cursor() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "unique");
    incoming_import(&f, &w, &p, "unique-resource", "in-progress");
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let query = scan.query().unwrap();
    let mut receipt = incoming_receipt(query.since, "unique-resource", "in-progress");
    receipt.call.control_url = Some("https://fixture.vapi.ai/contradiction/control".into());
    let before = (public_counts(&f), f.count("incoming_contact_receipts"));
    assert_eq!(
        f.store
            .record_provider_voice_scan(&scan.capture(vec![receipt]).unwrap())
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        (public_counts(&f), f.count("incoming_contact_receipts")),
        before
    );
    assert_eq!(
        f.store
            .begin_provider_voice_scan(&w, &p, &manifest())
            .unwrap()
            .query()
            .unwrap()
            .scanned_until,
        query.scanned_until
    );
}

#[test]
fn rejecting_held_voice_cannot_create_session_on_a_later_scan() {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let _b = another_room(&f, &w, "second");
    incoming_import(&f, &w, &a, "held-resource", "in-progress");
    let id = page(&f).contacts[0].id.clone();
    let rejected = f
        .store
        .reject_incoming_contact(
            &f.keys[0],
            "reject-held-voice",
            &id,
            0,
            "not for these rooms",
        )
        .unwrap();
    assert_eq!(rejected.state, IncomingContactState::Rejected);
    let before = public_counts(&f);
    assert!(incoming_import(&f, &w, &a, "held-resource", "ended")
        .calls
        .is_empty());
    assert_eq!(public_counts(&f), before);
    assert_eq!(page(&f).contacts[0], rejected);
}

#[test]
fn delivery_failure_rolls_back_invitation_journal_receipts_and_cursor() {
    let f = Fixture::new();
    let w = f.worker();
    let p = incoming_policy(&f, &w, "unique");
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER deny_voice_delivery BEFORE INSERT ON incoming_contact_deliveries BEGIN SELECT RAISE(ABORT,'fixture rollback'); END;").unwrap();
    let before = public_counts(&f);
    let scan = f
        .store
        .begin_provider_voice_scan(&w, &p, &manifest())
        .unwrap();
    let query = scan.query().unwrap();
    let captured = scan
        .capture(vec![incoming_receipt(
            query.since,
            "unique-resource",
            "in-progress",
        )])
        .unwrap();
    assert!(f.store.record_provider_voice_scan(&captured).is_err());
    assert_eq!(public_counts(&f), before);
    assert!(page(&f).contacts.is_empty());
    assert_eq!(f.count("incoming_contact_receipts"), 0);
    assert_eq!(
        f.store
            .begin_provider_voice_scan(&w, &p, &manifest())
            .unwrap()
            .query()
            .unwrap()
            .scanned_until,
        query.scanned_until
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER deny_voice_delivery;")
        .unwrap();
    assert_eq!(
        f.store
            .record_provider_voice_scan(&captured)
            .unwrap()
            .imported,
        1
    );
    assert_eq!(f.count("incoming_contact_deliveries"), 1);
}

#[test]
fn configured_account_cannot_acquire_another_owner_even_after_original_policy_withdrawal() {
    let f = Fixture::new();
    let w = f.worker();
    let original = incoming_policy(&f, &w, "first");
    // A separate valid Endpoint pair has another owner and different caller,
    // but cannot claim the retained original configured provider account.
    let c = f.store.connection().unwrap();
    for (old, new, address) in [
        ("initial-phone", "new-phone", "+15555550112"),
        (
            "provider-assistant",
            "new-assistant",
            "12345678-1234-1234-1234-123456789014",
        ),
    ] {
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) SELECT ?1,identity_id,tenant_id,device_id,transport,?2,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,?3 FROM endpoints WHERE id=?4",params![new,address,f._ids[1],old]).unwrap();
    }
    c.execute("UPDATE endpoints SET verification_state='revoked',revision=revision+1 WHERE id IN ('initial-phone','provider-assistant')",[]).unwrap();
    drop(c);
    let mut replacement = original.clone();
    replacement.id = "new-owner".into();
    replacement.policy.route_credential_id = f.keys[1].clone();
    replacement.policy.endpoint_id = "new-phone".into();
    replacement.policy.assistant_endpoint_id = "new-assistant".into();
    replacement.policy.assistant_id = "12345678-1234-1234-1234-123456789014".into();
    assert_eq!(
        f.store
            .configure_provider_voice_ingress(&w, replacement)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.count("provider_voice_ingress"), 1);
    assert_eq!(f.count("incoming_contact_resources"), 0);
}

#[test]
fn a_held_original_candidate_survives_worker_restart_without_rebasing() {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let _b = another_room(&f, &w, "second");
    incoming_import(&f, &w, &a, "held-resource", "in-progress");
    let original = page(&f);
    let before = public_counts(&f);
    w.retire();
    drop(w);
    let successor = f.worker();
    f.store.recover_provider_session_voice(&successor).unwrap();
    // Only one policy is remounted. The old hold must remain the same two-way
    // administrative choice, rather than generating an invitation now.
    f.store
        .configure_provider_voice_ingress(&successor, a.clone())
        .unwrap();
    let batch = incoming_import(&f, &successor, &a, "held-resource", "ended");
    assert!(batch.calls.is_empty());
    assert_eq!(page(&f), original);
    assert_eq!(public_counts(&f), before);
}

#[test]
fn independent_scanners_compete_without_choosing_or_duplicating_a_room() {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let b = another_room(&f, &w, "second");
    let at: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT max(since) FROM provider_voice_ingress", [], |r| {
            r.get(0)
        })
        .unwrap();
    let captures = [&a, &b].map(|p| {
        let scan = f
            .store
            .begin_provider_voice_scan(&w, p, &manifest())
            .unwrap();
        scan.capture(vec![incoming_receipt(at, "same-resource", "in-progress")])
            .unwrap()
    });
    let before = public_counts(&f);
    std::thread::scope(|scope| {
        let jobs: Vec<_> = captures
            .iter()
            .map(|captured| scope.spawn(|| f.store.record_provider_voice_scan(captured)))
            .collect();
        for job in jobs {
            let result = job.join().unwrap().unwrap();
            assert!(result.calls.is_empty());
            assert_eq!(result.imported, 0);
        }
    });
    assert_eq!(public_counts(&f), before);
    assert_eq!(page(&f).contacts.len(), 1);
    assert_eq!(f.count("incoming_contact_audit"), 1);
}
