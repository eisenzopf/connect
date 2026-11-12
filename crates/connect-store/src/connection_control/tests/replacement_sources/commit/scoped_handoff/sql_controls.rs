//! Actual all-flow Store fixture plus opaque same-Unit Begin witness. No packets
//! or decoded media are produced; existing OLD4 covers the four real Writers.
use super::*;
use crate::authority_writer_boundary::phase_ledger::{
    scoped::{self, Plan, Purpose, Row, Table, Transition, Witness},
    Key, Stage, FUNCTION,
};
use rusqlite::{types::Value, Connection};

fn text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}
fn plan<'a>(
    m: &'a Movement,
    c: &Connection,
    read: &'a crate::NativeReplacementSceneRead,
) -> Plan<'a> {
    plan_at(m, c, read, crate::now())
}
fn plan_at<'a>(
    m: &'a Movement,
    c: &Connection,
    read: &'a crate::NativeReplacementSceneRead,
    at: i64,
) -> Plan<'a> {
    // No test IDs authorize anything: this reconstructs the production builder
    // only AFTER the real origin rechecks all three confirmed private steps and
    // current full scene/credential/source facts in this exact transaction.
    let facts = m
        .origin
        .scoped_commit_begin_facts(&m.f.store, c, read)
        .unwrap();
    let attempt = "scoped-sql-attempt";
    let effect = "scoped-sql-effect";
    let expected = facts.expected.unwrap();
    let prepared = Row::new(
        Table::Effects,
        vec![
            text(effect),
            text(&facts.record),
            text("native-replacement-commit"),
            text(effect),
            text("none"),
            text("prepared"),
            Value::Integer(0),
            Value::Null,
            text(&expected),
            Value::Integer(0),
            Value::Null,
        ],
    )
    .unwrap();
    let step = Row::new(
        Table::ReplacementSteps,
        vec![
            text(&facts.record),
            text("commit"),
            text(attempt),
            text(effect),
            Value::Integer(0),
            Value::Null,
        ],
    )
    .unwrap();
    let mut started = step.clone();
    started.set("started", Value::Integer(1)).unwrap();
    let mut submitting = prepared.clone();
    submitting.set("state", text("submitting")).unwrap();
    submitting.set("attempt", Value::Integer(1)).unwrap();
    submitting
        .set("submission_generation", Value::Integer(1))
        .unwrap();
    let old = scoped::row_snapshot(c, Table::Operations, &[text(&facts.record)])
        .unwrap()
        .unwrap();
    let revision = match old.get("revision").unwrap() {
        Value::Integer(n) => *n,
        _ => panic!("actual revision must be INTEGER"),
    };
    let mut touched = old.clone();
    touched
        .set("revision", Value::Integer(revision.checked_add(1).unwrap()))
        .unwrap();
    touched.set("updated_at", Value::Integer(at)).unwrap();
    let transitions = vec![
        Transition::new(c, None, Some(&prepared)).unwrap(),
        Transition::new(c, None, Some(&step)).unwrap(),
        Transition::new(c, Some(&step), Some(&started)).unwrap(),
        Transition::new(c, Some(&prepared), Some(&submitting)).unwrap(),
        Transition::new(c, Some(&old), Some(&touched)).unwrap(),
    ];
    Plan {
        witness: Witness::Begin {
            origin: &m.origin,
            read,
        },
        key: Key {
            record: facts.record,
            phase: "scoped-commit-begin",
            attempt: attempt.into(),
            effect: effect.into(),
            expected,
            runtime: facts.runtime,
            reservation: facts.reservation.unwrap(),
            digest: facts.digest.unwrap(),
            stage: Stage::Begin,
        },
        purpose: Purpose::CommitBegin,
        source: facts.source,
        candidate: facts.candidate,
        required: facts.required,
        until: facts.until,
        transitions,
    }
}
fn denied(c: &Connection, sql: &str, values: &[Value]) {
    assert!(
        c.execute(sql, rusqlite::params_from_iter(values.iter()))
            .is_err(),
        "scoped permit admitted unrelated SQL: {sql}"
    );
}
fn active(c: &Connection) -> i64 {
    c.query_row(&format!("SELECT {FUNCTION}('scoped-row-active','operation_effects','insert','scoped-handoff',NULL,NULL,NULL)"), [], |r|r.get(0)).unwrap()
}

#[test]
fn scoped_begin_requires_exact_unit_store_purpose_and_live_opaque_witness() {
    let setup = setup(true);
    let m = &setup.movement;
    let other = Fixture::new();
    let read =
        m.f.store
            .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
            .unwrap();
    let proof = read.snapshot_interval().clone();
    let boundary = &m.f.store.authority_writer_boundary;
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    // A real foreign SQLite transaction cannot borrow the primary Unit's slot.
    let mut foreign = Connection::open(m.f.store.path()).unwrap();
    let ft = foreign
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(boundary
        .native_scoped_mutation_ledger(&m.f.store, &ft, &proof, plan(m, &ft, &read))
        .is_err());
    ft.rollback().unwrap();
    drop(foreign);
    let mut c = m.f.store.connection().unwrap();
    assert!(
        boundary
            .native_scoped_mutation_ledger(&m.f.store, &c, &proof, plan(m, &c, &read))
            .is_err(),
        "autocommit borrowed Unit"
    );
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(other
        .store
        .authority_writer_boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, plan(m, &tx, &read))
        .is_err());
    assert!(boundary
        .native_scoped_mutation_ledger(&other.store, &tx, &proof, plan(m, &tx, &read))
        .is_err());
    let mut expired = plan(m, &tx, &read);
    expired.until = Some(crate::now());
    assert!(boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, expired)
        .is_err());
    let mut wrong = plan(m, &tx, &read);
    wrong.purpose = Purpose::FinalReady;
    wrong.key.phase = "scoped-final-ready";
    wrong.key.stage = Stage::Confirm;
    assert!(
        boundary
            .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, wrong)
            .is_err(),
        "Begin witness minted FinalReady authority"
    );
    let mut foreign_row = plan(m, &tx, &read);
    foreign_row.source = setup.healthy_source.connection_id().into();
    assert!(boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, foreign_row)
        .is_err());
    let incomplete =
        m.f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &m.f.cid,
                crate::AuthorityMode::Read,
            )
            .unwrap()])
            .unwrap();
    assert!(boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &incomplete, plan(m, &tx, &read))
        .is_err());
    drop(incomplete);
    // A valid witness and exact full mixed proof MUST pass; denial tests above
    // cannot qualify merely because no usable permit exists in this fixture.
    let permit = boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, plan(m, &tx, &read))
        .unwrap();
    assert_eq!(active(&tx), 1);
    drop(permit);
    assert_eq!(active(&tx), 0);
    tx.rollback().unwrap();
    assert_eq!(active(&c), 0);
    drop(c);
    drop(proof);
    drop(read);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}

#[test]
fn scoped_begin_exact_five_deltas_reject_extra_columns_foreign_rows_replace_and_delete() {
    let setup = setup(true);
    let m = &setup.movement;
    let read =
        m.f.store
            .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
            .unwrap();
    let proof = read.snapshot_interval().clone();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let mut c = m.f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let at = crate::now();
    let p = plan_at(m, &tx, &read, at);
    let record = p.key.record.clone();
    let effect = p.key.effect.clone();
    let attempt = p.key.attempt.clone();
    let expected = p.key.expected.clone();
    let old_revision: i64 = tx
        .query_row(
            "SELECT revision FROM operations WHERE record_id=?1",
            [&record],
            |r| r.get(0),
        )
        .unwrap();
    // Use the exact timestamp already pinned in the immutable Operations row.
    let permit =
        m.f.store
            .authority_writer_boundary
            .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, p)
            .unwrap();
    denied(&tx,"UPDATE operations SET revision=revision+1,updated_at=?2,cancel_requested=1 WHERE record_id=?1",&[text(&record),Value::Integer(at)]);
    let foreign: String = tx
        .query_row(
            "SELECT operation_record_id FROM native_connection_lifetimes WHERE connection_id=?1",
            [setup.healthy_source.connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    denied(
        &tx,
        "UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1",
        &[text(foreign), Value::Integer(at)],
    );
    // The effect has no TEXT affinity conversion loophole: the unchanged exact
    // transition tuple is also exercised by actual inserts/updates below.
    let parsed: serde_json::Value = serde_json::from_str(&expected).unwrap();
    let reordered = format!(
        "{{{}}}",
        parsed
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| format!("{}:{}", serde_json::to_string(key).unwrap(), value))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_ne!(reordered, expected);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reordered).unwrap(),
        parsed
    );
    for altered in [
        Value::Text(reordered),
        Value::Text(format!(" {expected}")),
        Value::Blob(expected.as_bytes().to_vec()),
        Value::Null,
        Value::Integer(0),
    ] {
        denied(&tx,"INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,provider_ref,observation_json,submission_generation,compensates_effect_id) VALUES(?1,?2,'native-replacement-commit',?1,'none','prepared',0,NULL,?3,0,NULL)",&[text(&effect),text(&record),altered]);
    }
    assert_eq!(tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,provider_ref,observation_json,submission_generation,compensates_effect_id) VALUES(?1,?2,'native-replacement-commit',?1,'none','prepared',0,NULL,?3,0,NULL)",params![effect,record,expected]).unwrap(),1);
    // REPLACE would delete the now-existing exact declared row. A valid insert
    // tuple must not authorize its unique victim's deletion.
    denied(
        &tx,
        "INSERT OR REPLACE INTO operation_effects SELECT * FROM operation_effects WHERE id=?1",
        &[text(&effect)],
    );
    denied(
        &tx,
        "DELETE FROM operation_effects WHERE id=?1",
        &[text(&effect)],
    );
    denied(&tx,"UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1,provider_ref='foreign' WHERE id=?1",&[text(&effect)]);
    assert_eq!(tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id,started,observation_json) VALUES(?1,'commit',?2,?3,0,NULL)",params![record,attempt,effect]).unwrap(),1);
    assert_eq!(tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='commit'",[&record]).unwrap(),1);
    assert_eq!(tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).unwrap(),1);
    assert_eq!(
        tx.execute(
            "UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1",
            params![record, at]
        )
        .unwrap(),
        1
    );
    assert_eq!(
        tx.query_row(
            "SELECT revision FROM operations WHERE record_id=?1",
            [&record],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        old_revision + 1
    );
    assert_eq!(tx.query_row("SELECT count(*) FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='commit' AND s.started=1 AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1",[&record],|r|r.get::<_,u64>(0)).unwrap(),1);
    drop(permit);
    assert_eq!(active(&tx), 0);
    // No scoped authority escapes rollback into a later transaction.
    tx.rollback().unwrap();
    assert_eq!(active(&c), 0);
    let later = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    denied(
        &later,
        "UPDATE operations SET revision=revision+1 WHERE record_id=?1",
        &[text(record)],
    );
    later.rollback().unwrap();
    drop(c);
    drop(proof);
    drop(read);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}

#[test]
fn scoped_actual_commit_begin_persists_one_effect_and_started_attempt_with_full_current_facts() {
    let setup = setup(true);
    let m = &setup.movement;
    let read =
        m.f.store
            .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
            .unwrap();
    let root_record: String =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT record_id FROM operations WHERE operation_id=?1",
                [&m.command.operation_id],
                |r| r.get(0),
            )
            .unwrap();
    let actual = read.snapshot_interval();
    assert!(
        m.f.store.authority_covers(
            actual,
            &[
                crate::AuthorityScope::connection(
                    m.sources[0].connection_id(),
                    crate::AuthorityMode::Write
                )
                .unwrap(),
                crate::AuthorityScope::connection(
                    &read.media().target().connection.connection_id,
                    crate::AuthorityMode::Write
                )
                .unwrap(),
                crate::AuthorityScope::record(&root_record, crate::AuthorityMode::Write).unwrap(),
            ]
        ),
        "actual dedicated writer omitted changed connections/root"
    );
    assert!(
        !m.f.store.authority_covers(
            actual,
            &[crate::AuthorityScope::conversation(&m.f.cid, crate::AuthorityMode::Write).unwrap()]
        ),
        "broad CID fallback cannot qualify scoped Begin DML"
    );
    assert!(
        !m.f.store.authority_covers(
            actual,
            &[crate::AuthorityScope::connection(
                setup.healthy_source.connection_id(),
                crate::AuthorityMode::Write
            )
            .unwrap()]
        ),
        "unaffected AV source obtained a Writer"
    );
    let attempt =
        m.f.store
            .try_begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap()
            .expect("actual SQL Unit available");
    {
        let c = m.f.store.connection().unwrap();
        assert_eq!(c.query_row("SELECT count(*) FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='commit' AND s.started=1 AND e.provider='native-replacement-commit' AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1",[attempt.activation().origin().offer_record_id()],|r|r.get::<_,u64>(0)).unwrap(),1);
    }
    let refusal = attempt.not_submitted();
    drop(read);
    m.f.store
        .record_native_replacement_commit_not_submitted(&refusal)
        .unwrap();
}

#[test]
fn scoped_rollback_and_expiry_cannot_revive_rows_or_clear_a_new_nonce() {
    let setup = setup(true);
    let m = &setup.movement;
    let read =
        m.f.store
            .try_native_replacement_commit_scene_write(&m.origin, &m.sources)
            .unwrap();
    let proof = read.snapshot_interval().clone();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let mut c = m.f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let boundary = &m.f.store.authority_writer_boundary;
    let p = plan(m, &tx, &read);
    let record = p.key.record.clone();
    let effect = p.key.effect.clone();
    let expected = p.key.expected.clone();
    let old = boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, p)
        .unwrap();
    assert_eq!(active(&tx), 1);
    // Keep the old Rust Permit alive across a legal SQLite rollback/new-TX
    // boundary. The rollback hook must invalidate the old exact row program.
    tx.execute_batch("ROLLBACK; BEGIN IMMEDIATE").unwrap();
    assert_eq!(active(&tx), 0);
    denied(&tx,"INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,provider_ref,observation_json,submission_generation,compensates_effect_id) VALUES(?1,?2,'native-replacement-commit',?1,'none','prepared',0,NULL,?3,0,NULL)",&[text(&effect),text(&record),text(&expected)]);
    let fresh = boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, plan(m, &tx, &read))
        .unwrap();
    assert_eq!(active(&tx), 1);
    drop(old);
    assert_eq!(active(&tx), 1, "stale Permit Drop cleared newer nonce");
    assert_eq!(tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,provider_ref,observation_json,submission_generation,compensates_effect_id) VALUES(?1,?2,'native-replacement-commit',?1,'none','prepared',0,NULL,?3,0,NULL)",params![effect,record,expected]).unwrap(),1);
    // COMMIT with an active row permit must refuse and roll back, never leak
    // a partially executed five-row phase into the journal.
    assert!(tx.execute_batch("COMMIT").is_err());
    assert_eq!(active(&tx), 0);
    drop(fresh);
    // Do not ask the Transaction RAII wrapper to commit an already rolled-back
    // transaction. Re-enter only while holding the same exact Unit/proof.
    tx.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut expiring = plan(m, &tx, &read);
    let expires = crate::now() + 1;
    assert!(expiring.until.is_some_and(|limit| expires < limit));
    expiring.until = Some(expires);
    let permit = boundary
        .native_scoped_mutation_ledger(&m.f.store, &tx, &proof, expiring)
        .unwrap();
    assert_eq!(active(&tx), 1);
    let watchdog = Instant::now() + Duration::from_secs(2);
    while crate::now() < expires && Instant::now() < watchdog {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        crate::now() >= expires,
        "clock failed to cross actual original permit cutoff"
    );
    assert_eq!(active(&tx), 0);
    denied(&tx,"INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,provider_ref,observation_json,submission_generation,compensates_effect_id) VALUES(?1,?2,'native-replacement-commit',?1,'none','prepared',0,NULL,?3,0,NULL)",&[text(&effect),text(&record),text(&expected)]);
    drop(permit);
    tx.rollback().unwrap();
    drop(c);
    drop(proof);
    drop(read);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
