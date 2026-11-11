//! Exact current source births; no Core/remote-media qualification is inferred.
use super::*;

fn endpoint(f: &Fixture, id: &str) -> TransitionCommand {
    {
        let writer = fixture_operation_writer(f);
        let c = f.store.connection_with_authority(&writer).unwrap();
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('intent-device',?1,'tnt_default','fixture','fixture')", [&f.ids[0]]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES('intent-endpoint',?1,'tnt_default','intent-device','quic','quic://fixture.invalid/intent','[\"audio\"]','verified','fixture-proof',?2)", params![f.ids[0],crate::now()]).unwrap();
    }
    let mut r = f.request(id);
    r.connection_id = Some("conn-alice".into());
    r.intent = TransitionIntent::Replace {
        retained_connid: "conn-bob".into(),
        endpoint_id: Some("intent-endpoint".into()),
        candidate_connid: None,
        video: None,
    };
    transition_command(r)
}
fn read(f: &Fixture) -> Arc<crate::AuthorityGuardSet> {
    let scopes = {
        let c = f.store.connection().unwrap();
        crate::authority_admission::conversation_scopes(&c, &f.cid, AuthorityMode::Read).unwrap()
    };
    f.store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap()
}

#[test]
fn intent_replay_is_deferred_and_never_restarts_or_releases_reservations() {
    let f = Fixture::new();
    let command = endpoint(&f, "intent-deferred");
    let mut signed = request(&f, get("unused"));
    signed.command = PeerCommand::Transition(command.clone());
    let old = read(&f);
    let proof = admit(&f, &signed);
    let first = f
        .store
        .admit_transition_with_peer(&f.keys[0], command.clone(), &proof)
        .unwrap();
    let external = rusqlite::Connection::open(f.store.path()).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let replay = f
        .store
        .admit_transition_with_peer(&f.keys[0], command, &proof);
    external.execute_batch("ROLLBACK").unwrap();
    assert_eq!(replay.unwrap(), first);
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_resources WHERE operation_record_id=?1 AND fencing_generation=1", [&first.record_id], |r|r.get::<_,u64>(0)).unwrap(),3);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM journal WHERE operation_record_id=?1",
            [&first.record_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
            [&first.record_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(c);
    drop(proof);
    drop(old);
    f.cancel("intent-cancel", &first.operation_id);
    assert_eq!(f.get(&first.operation_id).state, OperationState::Cancelled);
    let packet = read(&f);
    // A real cancellation is not the pure untouched-namespace replay.
    assert_eq!(
        f.store
            .admit_peer_authority(&signed, &AuthorityWait::new(Duration::from_millis(20)))
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(packet);
}

#[test]
fn intent_no_started_boundary_is_exactly_noop_but_actual_loss_requires_writer() {
    let f = Fixture::new();
    let command = endpoint(&f, "intent-noop-witness");
    {
        let c = f.store.connection().unwrap();
        assert!(!crate::session_reconnect::connected_now(&c, &f.store, &f.sid).unwrap());
        assert!(
            crate::session_participant_lifecycle::reconcile_is_unchanged(&c, &f.store, &f.sid)
                .unwrap()
        );
        let before = c.total_changes();
        crate::session_participant_lifecycle::reconcile(&c, &f.store, &f.cid, &f.sid, None)
            .unwrap();
        assert_eq!(c.total_changes(), before);
    }
    let mut signed = request(&f, get("unused"));
    signed.command = PeerCommand::Transition(command);
    {
        let writer = fixture_operation_writer(&f);
        let c = f.store.connection_with_authority(&writer).unwrap();
        c.execute("UPDATE sessions SET state='active' WHERE id=?1", [&f.sid])
            .unwrap();
        let fact =
            serde_json::json!({"session":{"id":f.sid,"conversation_id":f.cid,"state":"active"}})
                .to_string();
        c.execute(
            "INSERT INTO session_started_facts VALUES(?1,?2,?3,'[]')",
            params![f.sid, f.cid, fact],
        )
        .unwrap();
        assert!(
            !crate::session_participant_lifecycle::reconcile_is_unchanged(&c, &f.store, &f.sid)
                .unwrap()
        );
    }
    let old = read(&f);
    assert_eq!(
        f.store
            .admit_peer_authority(&signed, &AuthorityWait::new(Duration::from_millis(20)))
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(old);
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='intent-noop-witness'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn intent_birth_permit_rejects_foreign_unit_forged_rows_and_unique_victims() {
    let f = Fixture::new();
    let command = endpoint(&f, "intent-permit-bound");
    let request = command.with_expiry(crate::now() + 300);
    let victim = f.admit("original-member-victim");
    let proof = read(&f);
    let (foreign, other) = {
        let other = Fixture::new();
        let foreign = read(&other);
        (foreign, other)
    };
    let record = "opr_intent_permit_bound";
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(f
        .store
        .authority_writer_boundary
        .replacement_intent_birth(&f.store, &tx, &foreign, &f.keys[0], &request, record)
        .is_err());
    assert!(other
        .store
        .authority_writer_boundary
        .replacement_intent_birth(&other.store, &tx, &proof, &f.keys[0], &request, record)
        .is_err());
    let permit = f
        .store
        .authority_writer_boundary
        .replacement_intent_birth(&f.store, &tx, &proof, &f.keys[0], &request, record)
        .unwrap();
    // An exact member tuple is insufficient without the exact new immutable root.
    tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,created_at,updated_at,expires_at,ledger_version) VALUES(?1,'tnt_default',?2,'forged-root','0000000000000000000000000000000000000000000000000000000000000000','connection.replace',?3,?4,'conn-alice',0,'{}','accepted',?5,?5,?6,1)", params![record,f.ids[0],f.cid,f.sid,crate::now(),request.expires_at]).unwrap();
    assert!(tx
        .execute(
            "INSERT INTO operation_members VALUES(?1,?2,?3,1)",
            params![record, f.parts[0], f.ids[0]]
        )
        .is_err());
    assert!(tx
        .execute(
            "INSERT INTO operation_endpoints VALUES(?1,'intent-endpoint',0)",
            [record]
        )
        .is_err());
    for sql in [
        "UPDATE credentials SET revoked=1",
        "UPDATE connections SET generation=generation+1",
        "UPDATE sessions SET state='ended'",
        "UPDATE operations SET state='preparing' WHERE record_id=?1",
        "INSERT OR REPLACE INTO operations SELECT * FROM operations WHERE record_id=?1",
    ] {
        let r = if sql.contains("?1") {
            tx.execute(sql, [record])
        } else {
            tx.execute(sql, [])
        };
        assert!(
            r.is_err(),
            "intent permit authorized unrelated mutation: {sql}"
        );
    }
    assert!(tx.execute("INSERT OR REPLACE INTO operation_members SELECT * FROM operation_members WHERE operation_record_id=?1",[&victim.record_id]).is_err());
    assert_eq!(
        tx.query_row(
            "SELECT COUNT(*) FROM operation_members WHERE operation_record_id=?1",
            [&victim.record_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        3
    );
    drop(permit);
    tx.rollback().unwrap();
    drop(c);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE record_id=?1",
                [record],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn intent_pending_departure_refuses_read_birth_even_without_a_started_boundary() {
    let f = Fixture::new();
    let command = endpoint(&f, "intent-pending-departure");
    let mut signed = request(&f, get("unused"));
    signed.command = PeerCommand::Transition(command);
    {
        let writer = fixture_operation_writer(&f);
        let c = f.store.connection_with_authority(&writer).unwrap();
        let fact=serde_json::json!({"kind":"session_participant_joined","data":{"fact_id":"prior-joined","session":{"id":f.sid,"conversation_id":f.cid}}}).to_string();
        c.execute("INSERT INTO session_participant_facts VALUES('prior-joined',?1,?2,?3,1,'session_participant_joined',?4,'[]')",params![f.cid,f.sid,f.parts[2],fact]).unwrap();
        c.execute(
            "INSERT INTO session_connected_presence VALUES(?1,?2,1,'joined','prior-joined')",
            params![f.sid, f.parts[2]],
        )
        .unwrap();
        c.execute(
            "UPDATE connections SET state='retired' WHERE id='conn-carol'",
            [],
        )
        .unwrap();
        assert!(
            !crate::session_participant_lifecycle::reconcile_is_unchanged(&c, &f.store, &f.sid)
                .unwrap()
        );
    }
    let original = read(&f);
    let refused = f
        .store
        .admit_peer_authority(&signed, &AuthorityWait::new(Duration::from_millis(20)));
    drop(original);
    assert_eq!(refused.unwrap_err().code, "authority-busy");
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='intent-pending-departure'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn intent_birth_never_uses_read_custody_to_bypass_current_credential_or_endpoint_refusal() {
    for revoke_credential in [false, true] {
        let f = Fixture::new();
        let command = endpoint(&f, "intent-current-refusal");
        let mut signed = request(&f, get("unused"));
        signed.command = PeerCommand::Transition(command);
        {
            let writer = fixture_operation_writer(&f);
            let c = f.store.connection_with_authority(&writer).unwrap();
            if revoke_credential {
                c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
                    .unwrap();
            } else {
                c.execute("UPDATE endpoints SET verification_state='revoked',revision=revision+1 WHERE id='intent-endpoint'",[]).unwrap();
            }
        }
        let e = f
            .store
            .admit_peer_authority(&signed, &AuthorityWait::new(Duration::from_millis(50)))
            .unwrap_err();
        assert_eq!(
            e.code,
            if revoke_credential {
                "unauthorized"
            } else {
                "forbidden"
            }
        );
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM operations WHERE operation_id='intent-current-refusal'",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
    }
}
