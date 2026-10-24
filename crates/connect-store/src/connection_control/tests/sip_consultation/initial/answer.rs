//! Captured-answer consent is separate from public presence and media readiness.
use super::*;

fn public_state(f: &Fixture) -> Vec<String> {
    let c = f.store.connection().unwrap();
    ["SELECT json_group_array(json_array(state,topology_revision)) FROM sessions",
     "SELECT json_group_array(json_array(session_id,participant_id,membership_epoch,state,role)) FROM session_participants",
     "SELECT json_group_array(json_array(session_id,participant_id,membership_epoch,capabilities_json)) FROM session_presence_capabilities"]
        .into_iter().map(|sql|c.query_row(sql,[],|r|r.get(0)).unwrap()).collect()
}

#[test]
fn private_answer_accepts_only_original_connection_without_public_presence_or_codec_changes() {
    let (f, parent, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let public = public_state(&f);
    let receipts = f.count("operation_control_receipts");
    let operations = f.count("operations");
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    for _ in 0..2 {
        let accepted = f
            .store
            .record_private_initial_telephone_answer(&capture)
            .unwrap();
        assert!(accepted.problem.is_none());
        let media = accepted.media.unwrap();
        assert_eq!(media.connection_id(), initial.connection_id());
        assert_eq!(media.codec(), "g.711-mu");
        assert!(
            f.store.try_initial_telephone_media_read(media).is_err(),
            "consent is not private media readiness"
        );
    }
    assert_eq!(public_state(&f), public);
    assert_eq!(f.count("operation_control_receipts"), receipts);
    assert_eq!(f.count("operations"), operations);
    assert_eq!(f.count("initial_telephone_private_acceptances"), 1);
    assert_eq!(f.count("connections"), 4);
    let c = f.store.connection().unwrap();
    let (accepted,kind,record,connected):(bool,String,String,bool)=c.query_row("SELECT accepted,acceptance_kind,acceptance_record_id,connected_at IS NOT NULL FROM initial_telephone_answers WHERE operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert!(accepted && !connected);
    assert_eq!(kind, "private-connection");
    assert_eq!(record, initial.record_id());
    let source:(String,String,String)=c.query_row("SELECT consultation_record_id,endpoint_id,codec FROM initial_telephone_private_acceptances WHERE operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        source,
        (
            parent.record_id().into(),
            initial.source().endpoint_id.clone(),
            "g.711-mu".into()
        )
    );
    let state: (String, Option<String>, Option<String>) = c
        .query_row(
            "SELECT state,observed_runtime_id,readiness_json FROM connections WHERE id=?1",
            [initial.connection_id()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, ("preparing".into(), None, None));
}

#[test]
fn private_answer_public_replay_never_returns_private_consent_as_public_media() {
    let (f, _, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMA").unwrap();
    assert!(f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_some());
    for _ in 0..2 {
        let result = f.store.record_initial_telephone_answer(&capture).unwrap();
        assert!(result.media.is_none());
        assert_eq!(result.problem.as_deref(), Some("private-context-required"));
    }
    assert!(f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_some());
}

#[test]
fn private_answer_withdrawal_preserves_actual_fact_without_acceptance_or_ready_publication() {
    for fault in 0..8 {
        let (f, parent, policy, _, initial) = prepared();
        let signal = signaling(&f, &initial);
        let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
        let public = public_state(&f);
        let (sql, key) = match fault {
            0 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                f.keys[0].as_str(),
            ),
            1 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                policy.endpoint_credential_id.as_str(),
            ),
            2 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                policy.route_credential_id.as_str(),
            ),
            3 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                policy.endpoint_credential_id.as_str(),
            ),
            4 => (
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                policy.endpoint_id.as_str(),
            ),
            5 => (
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                f.pids[2].as_str(),
            ),
            6 => (
                "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                parent.record_id(),
            ),
            _ => (
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                f.pids[1].as_str(),
            ),
        };
        f.store.connection().unwrap().execute(sql, [key]).unwrap();
        let result = f
            .store
            .record_private_initial_telephone_answer(&capture)
            .unwrap();
        assert!(
            result.media.is_none() && result.problem.is_some(),
            "fault {fault}"
        );
        assert_eq!(public_state(&f), public, "fault {fault}");
        assert_eq!(f.count("initial_telephone_answers"), 1);
        assert_eq!(f.count("initial_telephone_private_acceptances"), 0);
        let c = f.store.connection().unwrap();
        let fact:(bool,String,String)=c.query_row("SELECT a.accepted,a.codec,e.state FROM initial_telephone_answers a JOIN initial_telephone_operations n ON n.operation_record_id=a.operation_record_id JOIN operation_effects e ON e.id=n.effect_id WHERE a.operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            fact,
            (false, "g.711-mu".into(), "confirmed".into()),
            "fault {fault}"
        );
    }
}

#[test]
fn private_answer_accepted_replay_rechecks_current_authority_without_erasing_consent() {
    for fault in 0..5 {
        let (f, parent, policy, _, initial) = prepared();
        let signal = signaling(&f, &initial);
        let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
        assert!(f
            .store
            .record_private_initial_telephone_answer(&capture)
            .unwrap()
            .media
            .is_some());
        let (sql, key) = match fault {
            0 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                policy.endpoint_credential_id.as_str(),
            ),
            1 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                policy.route_credential_id.as_str(),
            ),
            2 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                f.keys[0].as_str(),
            ),
            3 => (
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                policy.endpoint_id.as_str(),
            ),
            _ => (
                "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                parent.record_id(),
            ),
        };
        f.store.connection().unwrap().execute(sql, [key]).unwrap();
        assert!(
            f.store
                .record_private_initial_telephone_answer(&capture)
                .is_err(),
            "fault {fault}"
        );
        assert_eq!(f.count("initial_telephone_private_acceptances"), 1);
        assert!(f
            .store
            .record_initial_telephone_answer(&capture)
            .unwrap()
            .media
            .is_none());
    }
}

#[test]
fn private_answer_conflicting_observation_and_public_fallback_cannot_rebind_consent() {
    let (f, _, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    assert!(signal.capture_answer("wrong-call", "PCMU").is_err());
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    assert!(f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_some());
    let changed = signal.capture_answer(signal.sip_call_id(), "PCMA").unwrap();
    assert_eq!(
        f.store
            .record_private_initial_telephone_answer(&changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.count("initial_telephone_private_acceptances"), 1);

    let (f, _, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    assert!(f
        .store
        .record_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_none());
    assert!(f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_none());
    assert_eq!(f.count("initial_telephone_private_acceptances"), 0);
}

#[test]
fn private_answer_sql_requires_exact_original_consent_and_keeps_it_immutable() {
    let (f, parent, policy, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    // Inspect the raw captured-but-unaccepted boundary before any consent.
    let c = f.store.connection().unwrap();
    let observation = json!({"kind":"answered","runtime_connection_id":signal.runtime_connection_id(),"call_id":signal.sip_call_id(),"codec":"g.711-mu"}).to_string();
    c.execute("INSERT INTO initial_telephone_answers(operation_record_id,observation_json,codec,observed_at) VALUES(?1,?2,'g.711-mu',?3)",params![initial.record_id(),observation,crate::now()]).unwrap();
    for kind in ["public-session", "private-connection"] {
        assert!(c.execute("UPDATE initial_telephone_answers SET accepted=1,acceptance_kind=?2,acceptance_record_id=operation_record_id WHERE operation_record_id=?1",params![initial.record_id(),kind]).is_err());
    }
    let audience = serde_json::to_string(&initial.source().audience).unwrap();
    let own = &initial.source().target;
    for fault in 0..5 {
        let endpoint = if fault == 0 {
            "wrong-endpoint"
        } else {
            initial.source().endpoint_id.as_str()
        };
        let root = if fault == 1 {
            initial.record_id()
        } else {
            parent.record_id()
        };
        let codec = if fault == 2 { "g.711-a" } else { "g.711-mu" };
        let private = if fault == 3 { "[]" } else { &audience };
        let epoch = own.membership_epoch + u64::from(fault == 4);
        assert!(c.execute("INSERT INTO initial_telephone_private_acceptances VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![initial.record_id(),root,policy.endpoint_credential_id,endpoint,initial.source().endpoint_revision,own.participant_id,epoch,codec,private,crate::now()]).is_err(),"fault {fault}");
    }
    drop(c);
    // A raw saved answer cannot be promoted by replay; consent is once-only.
    assert!(f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_none());
    let (f, _, _, _, initial) = prepared();
    let signal = signaling(&f, &initial);
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    f.store
        .record_private_initial_telephone_answer(&capture)
        .unwrap();
    let c = f.store.connection().unwrap();
    assert!(c
        .execute(
            "UPDATE initial_telephone_private_acceptances SET codec='g.711-a'",
            []
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM initial_telephone_private_acceptances", [])
        .is_err());
    assert!(c
        .execute(
            "UPDATE initial_telephone_answers SET acceptance_kind='public-session'",
            []
        )
        .is_err());
    assert!(c
        .execute(
            "UPDATE initial_telephone_answers SET acceptance_record_id=?1",
            [parent.record_id()]
        )
        .is_err());
}

#[test]
fn private_answer_backup_preserves_consent_detects_its_tampering_and_fences_old_runtime() {
    let (f, parent, policy, reservation, initial) = prepared();
    let signal = signaling(&f, &initial);
    let capture = signal.capture_answer(signal.sip_call_id(), "PCMU").unwrap();
    assert!(f
        .store
        .record_private_initial_telephone_answer(&capture)
        .unwrap()
        .media
        .is_some());
    let backup = f._dir.path().join("private-consent-backup.sqlite3");
    let expected = f.store.backup(&backup).unwrap();
    assert_eq!(expected.schema, crate::SCHEMA_VERSION);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), expected);
    let tampered = f._dir.path().join("private-consent-tampered.sqlite3");
    std::fs::copy(&backup, &tampered).unwrap();
    let c = rusqlite::Connection::open(&tampered).unwrap();
    c.execute_batch("DROP TRIGGER private_telephone_acceptance_immutable; UPDATE initial_telephone_private_acceptances SET accepted_at=accepted_at+1;").unwrap();
    drop(c);
    assert_ne!(
        Store::inspect_snapshot(&tampered).unwrap().digest,
        expected.digest,
        "snapshot digest must include the populated own-consent row"
    );
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_ne!(restored.runtime_epoch(), f.store.runtime_epoch());
    let c = restored.connection().unwrap();
    let row:(String,String,String,u64,String,String)=c.query_row("SELECT consultation_record_id,endpoint_credential_id,endpoint_id,membership_epoch,codec,audience_json FROM initial_telephone_private_acceptances WHERE operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        row,
        (
            parent.record_id().into(),
            policy.endpoint_credential_id,
            policy.endpoint_id,
            initial.source().target.membership_epoch,
            "g.711-mu".into(),
            serde_json::to_string(&initial.source().audience).unwrap()
        )
    );
    let accepted:(String,String,bool,bool)=c.query_row("SELECT acceptance_kind,acceptance_record_id,accepted,connected_at IS NOT NULL FROM initial_telephone_answers WHERE operation_record_id=?1",[initial.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        accepted,
        (
            "private-connection".into(),
            initial.record_id().into(),
            true,
            false
        )
    );
    drop(c);
    assert!(restored
        .record_private_initial_telephone_answer(&capture)
        .is_err());
    assert!(restored
        .validate_sip_consultation_reservation(&reservation)
        .is_err());
    assert_eq!(
        restored
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM initial_telephone_private_acceptances",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}
