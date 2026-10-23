use super::incoming_custody::{another_room, incoming_import};
use super::*;
use connect_protocol::incoming_contacts::*;
const RESOURCE: &str = "12345678-1234-1234-1234-123456789015";
fn setup() -> (Fixture, Arc<ProviderSessionVoiceWorker>, IncomingContact) {
    let f = Fixture::new();
    let w = f.worker();
    let a = incoming_policy(&f, &w, "first");
    let _b = another_room(&f, &w, "second");
    assert!(incoming_import(&f, &w, &a, RESOURCE, "in-progress")
        .calls
        .is_empty());
    let contact = f
        .store
        .incoming_contacts(&f.keys[0], None, 32)
        .unwrap()
        .contacts
        .remove(0);
    (f, w, contact)
}
fn selection(contact: &IncomingContact, index: usize, operation: &str) -> IncomingContactSelection {
    IncomingContactSelection {
        operation_id: operation.into(),
        contact_id: contact.id.clone(),
        candidate_id: contact.candidates[index].id.clone(),
        expected_revision: 0,
    }
}
fn ticket(
    f: &Fixture,
    w: &Arc<ProviderSessionVoiceWorker>,
    s: IncomingContactSelection,
) -> ProviderIncomingVoiceResolution {
    let ProviderIncomingVoiceResolutionPreparation::Query(q) = f
        .store
        .prepare_incoming_voice_resolution(w, &f.keys[0], s, &manifest())
        .unwrap()
    else {
        panic!("new selection must read its original resource")
    };
    q
}
fn capture(
    q: ProviderIncomingVoiceResolution,
    state: &str,
) -> ProviderIncomingVoiceResolutionCapture {
    let at = q.query().unwrap().since;
    q.capture(vec![incoming_receipt(at, RESOURCE, state)])
        .unwrap()
}
fn counts(f: &Fixture) -> (u64, u64, u64, u64, u64) {
    (
        f.count("sessions"),
        f.count("journal"),
        f.count("operations"),
        f.count("incoming_contact_audit"),
        f.count("provider_session_voice"),
    )
}
#[test]
fn explicit_selection_imports_one_original_observed_session_and_returns_owned_origin() {
    let (f, w, contact) = setup();
    let s = selection(&contact, 1, "select-original");
    let before = f.count("sessions");
    let q = ticket(&f, &w, s.clone());
    let query = q.query().unwrap();
    assert_eq!(query.scan_until - query.scanned_until, 999);
    assert_eq!(f.count("sessions"), before);
    let result = f
        .store
        .record_incoming_voice_resolution(&capture(q, "in-progress"))
        .unwrap();
    assert_eq!(result.contact.state, IncomingContactState::Resolved);
    let o = result.origin.unwrap();
    assert_eq!(o.conversation_id(), contact.candidates[1].conversation_id);
    assert_eq!(o.direction(), ProviderCallDirection::Inbound);
    assert_eq!(f.count("sessions"), before + 1);
    let decision = result.contact.decision.as_ref().unwrap();
    assert_eq!(decision.operation_id, s.operation_id);
    assert_eq!(decision.actor_identity_id, f._ids[0]);
    assert_eq!(decision.actor_credential_id, f.keys[0]);
    assert_eq!(decision.session_id.as_deref(), Some(o.session_id()));
    let dispatch = f.begin(&w, &o, ProviderSessionVoiceAction::Status);
    assert_eq!(dispatch.existing_resource().unwrap().0, RESOURCE);
    assert_eq!(
        f.store
            .begin_provider_session_voice(&w, &o, ProviderSessionVoiceAction::Start, &manifest())
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![o.session_id(), o.participant_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "invited");
    let before = counts(&f);
    let ProviderIncomingVoiceResolutionPreparation::Completed(replayed) = f
        .store
        .prepare_incoming_voice_resolution(&w, &f.keys[0], s, &manifest())
        .unwrap()
    else {
        panic!("replay must not require provider IO")
    };
    assert_eq!(replayed, result.contact);
    assert_eq!(counts(&f), before);
}
#[test]
fn fresh_read_requires_the_original_resource_and_immutable_binding() {
    let (f, w, contact) = setup();
    let before = counts(&f);
    let s = selection(&contact, 0, "proof-required");
    assert_eq!(
        ticket(&f, &w, s.clone()).capture(vec![]).unwrap_err().code,
        "provider-unavailable"
    );
    for field in ["caller", "created_at"] {
        let q = ticket(&f, &w, s.clone());
        let mut r = incoming_receipt(q.query().unwrap().since, RESOURCE, "in-progress");
        if field == "caller" {
            r.call.caller = "+15555550999".into();
        } else {
            r.call.created_at += 1;
        }
        assert_eq!(q.capture(vec![r]).unwrap_err().code, "conflict");
    }
    let q = ticket(&f, &w, s);
    let at = q.query().unwrap().since;
    assert_eq!(
        q.capture(vec![
            incoming_receipt(at, RESOURCE, "in-progress"),
            incoming_receipt(at, RESOURCE, "ended")
        ])
        .unwrap_err()
        .code,
        "provider-unavailable"
    );
    assert_eq!(counts(&f), before);
}
#[test]
fn current_authority_is_rechecked_after_provider_io_and_before_replay() {
    let (f, w, contact) = setup();
    let s = selection(&contact, 0, "current-authority");
    assert!(f
        .store
        .prepare_incoming_voice_resolution(&w, &f.keys[1], s.clone(), &manifest())
        .is_err());
    let mut forged = s.clone();
    forged.candidate_id = "not-an-original-candidate".into();
    assert!(f
        .store
        .prepare_incoming_voice_resolution(&w, &f.keys[0], forged, &manifest())
        .is_err());
    let captured = capture(ticket(&f, &w, s.clone()), "in-progress");
    let before = counts(&f);
    f.revoke(1);
    assert!(f.store.record_incoming_voice_resolution(&captured).is_err());
    assert_eq!(counts(&f), before);
    assert!(f
        .store
        .prepare_incoming_voice_resolution(&w, &f.keys[0], s, &manifest())
        .is_err());
}
#[test]
fn completed_replay_preserves_attribution_for_another_current_owner_credential() {
    let (f, w, contact) = setup();
    let s = selection(&contact, 0, "attributed-replay");
    let result = f
        .store
        .record_incoming_voice_resolution(&capture(ticket(&f, &w, s.clone()), "in-progress"))
        .unwrap();
    let key = f
        .store
        .issue(&f._ids[0], Scope::operator(), crate::now() + 600)
        .unwrap()
        .id;
    let ProviderIncomingVoiceResolutionPreparation::Completed(replayed) = f
        .store
        .prepare_incoming_voice_resolution(&w, &key, s.clone(), &manifest())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(replayed, result.contact);
    assert_eq!(replayed.decision.unwrap().actor_credential_id, f.keys[0]);
    f.revoke(1);
    assert!(f
        .store
        .prepare_incoming_voice_resolution(&w, &key, s, &manifest())
        .is_err());
}
#[test]
fn competing_decisions_commit_one_session_and_one_attributed_delivery() {
    let (f, w, contact) = setup();
    let captures = [
        capture(
            ticket(&f, &w, selection(&contact, 0, "choose-first")),
            "in-progress",
        ),
        capture(
            ticket(&f, &w, selection(&contact, 1, "choose-second")),
            "in-progress",
        ),
    ];
    let before = f.count("sessions");
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = captures
            .iter()
            .map(|capture| scope.spawn(|| f.store.record_incoming_voice_resolution(capture)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_ref().is_err_and(|e| e.code == "conflict"))
            .count(),
        1
    );
    assert_eq!(f.count("sessions"), before + 1);
    assert_eq!(f.count("incoming_contact_deliveries"), 1);
    assert_eq!(f.count("incoming_contact_audit"), 2);
    assert_eq!(f.count("provider_session_voice"), 1);
}
#[test]
fn delivery_storage_failure_rolls_back_session_and_decision_then_retries_once() {
    let (f, w, contact) = setup();
    let s = selection(&contact, 0, "rollback-selection");
    let captured = capture(ticket(&f, &w, s), "in-progress");
    let before = counts(&f);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_incoming_selection BEFORE INSERT ON incoming_contact_deliveries BEGIN SELECT RAISE(ABORT,'fixture rollback'); END;").unwrap();
    assert!(f.store.record_incoming_voice_resolution(&captured).is_err());
    assert_eq!(counts(&f), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_incoming_selection;")
        .unwrap();
    let result = f.store.record_incoming_voice_resolution(&captured).unwrap();
    assert!(result.origin.is_some());
    let before = counts(&f);
    let replay = f.store.record_incoming_voice_resolution(&captured).unwrap();
    assert!(replay.origin.is_none());
    assert_eq!(replay.contact, result.contact);
    assert_eq!(counts(&f), before);
}
#[test]
fn a_call_that_ended_while_held_records_terminal_absence_without_end_submission() {
    let (f, w, contact) = setup();
    let result = f
        .store
        .record_incoming_voice_resolution(&capture(
            ticket(&f, &w, selection(&contact, 0, "ended-selection")),
            "ended",
        ))
        .unwrap();
    let origin = result.origin.unwrap();
    let view = f
        .store
        .provider_session_voice_view(&f.keys[0], origin.id())
        .unwrap();
    assert_eq!(view.state, ProviderCallState::Ended);
    assert!(view.closed_at.is_some());
    assert!(!view.remote_identity_verified);
    assert!(!view.local_media);
    assert!(f
        .store
        .begin_provider_session_voice(&w, &origin, ProviderSessionVoiceAction::End, &manifest())
        .unwrap()
        .is_none());
}
#[test]
fn retirement_keeps_ticket_custody_until_drop_and_refuses_selection() {
    let (f, w, contact) = setup();
    let captured = capture(
        ticket(&f, &w, selection(&contact, 0, "retired-selection")),
        "in-progress",
    );
    let before = counts(&f);
    w.retire();
    drop(w);
    assert!(f.store.acquire_provider_session_voice_worker().is_err());
    assert!(f.store.record_incoming_voice_resolution(&captured).is_err());
    assert_eq!(counts(&f), before);
    drop(captured);
    assert!(f.store.acquire_provider_session_voice_worker().is_ok());
}

#[test]
fn readmitted_membership_cannot_inherit_a_held_original_candidate_after_io() {
    use connect_protocol::membership::{MembershipAction, MembershipRequest};
    let (f, w, contact) = setup();
    let s = selection(&contact, 0, "epoch-choice");
    let captured = capture(ticket(&f, &w, s.clone()), "in-progress");
    let candidate = &contact.candidates[0];
    for (operation_id, change) in [
        (
            "remove-held-candidate",
            MembershipAction::Remove {
                participant_id: candidate.participant_id.clone(),
                reason: Some("new epoch".into()),
            },
        ),
        (
            "readmit-held-candidate",
            MembershipAction::Add {
                identity_id: f._ids[1].clone(),
                kind: connect_protocol::provisioning::IdentityKind::Human,
                role: "participant".into(),
            },
        ),
    ] {
        let revision = f
            .store
            .membership_snapshot(&f.keys[0], &candidate.conversation_id)
            .unwrap()
            .revision;
        f.store
            .execute(
                &f.keys[0],
                &Operation::Membership {
                    request: MembershipRequest {
                        operation_id: operation_id.into(),
                        conversation_id: candidate.conversation_id.clone(),
                        expected_revision: revision,
                        change,
                    },
                },
            )
            .unwrap();
    }
    let before = counts(&f);
    assert!(f.store.record_incoming_voice_resolution(&captured).is_err());
    assert_eq!(counts(&f), before);
    assert!(f
        .store
        .prepare_incoming_voice_resolution(&w, &f.keys[0], s, &manifest())
        .is_err());
}
#[test]
fn fresh_resolution_uses_the_same_aggregate_receipt_budget_as_discovery() {
    let (f, w, contact) = setup();
    let q = ticket(&f, &w, selection(&contact, 0, "bounded-proof"));
    let at = q.query().unwrap().since;
    let before = counts(&f);
    let receipts = (0..256)
        .map(|_| {
            let mut r = incoming_receipt(at, RESOURCE, "in-progress");
            r.call.control_url = Some(format!("https://fixture.vapi.ai/{}", "x".repeat(4000)));
            r.call.ended_reason = Some("x".repeat(256));
            r
        })
        .collect();
    assert_eq!(q.capture(receipts).unwrap_err().code, "capacity-exceeded");
    assert_eq!(counts(&f), before);
}
