//! Preserve old rows while refusing to invent missing end-command authority.
use super::{fixture::Fixture, media_fixture::*};
use crate::{Limits, Store, SCHEMA_VERSION};

fn schema_thirteen(f: &Fixture) {
    f.store.connection().unwrap().execute_batch(
        "DROP TABLE vapi_session_setups; DROP INDEX session_bindings_task_grant; ALTER TABLE session_bindings DROP COLUMN task_grant_id; DROP TABLE media_end_intents; DROP TABLE media_start_admissions; DROP INDEX task_effect_media_resource; PRAGMA user_version=13;",
    ).unwrap();
}

fn migrate(f: &Fixture) {
    schema_thirteen(f);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let backup = f._dir.path().join("thirteen.sqlite3");
    f.store.backup(&backup).unwrap();
    assert_eq!(
        Store::migrate(f.store.path(), &backup, &before)
            .unwrap()
            .schema,
        SCHEMA_VERSION
    );
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    Store::open(f.store.path(), Limits::default())
        .unwrap()
        .integrity_check()
        .unwrap();
}

#[test]
fn ordinary_pending_voice_end_retains_its_known_issuer_after_upgrade() {
    let (f, _) = call_setup();
    let call = active_call(&f);
    f.store
        .execute(&f.key.id, &call_end(&f, "end", &call))
        .unwrap();
    let before = f
        .store
        .execute(
            &f.key.id,
            &connect_protocol::Operation::CallStatus {
                conversation_id: f.cid.clone(),
                call_id: call.clone(),
            },
        )
        .unwrap();
    migrate(&f);
    assert_eq!(
        f.store
            .execute(
                &f.key.id,
                &connect_protocol::Operation::CallStatus {
                    conversation_id: f.cid.clone(),
                    call_id: call
                }
            )
            .unwrap(),
        before
    );
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert_eq!(job.action, crate::VoiceAction::End);
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
}

#[test]
fn unresolved_task_end_stays_uncertain_until_a_fresh_authorized_end() {
    let (f, task) = call_setup();
    let call = active_call(&f);
    f.run(&invoke(&task, "end", call_end(&f, "end", &call)))
        .unwrap();
    migrate(&f);
    let c = f.store.connection().unwrap();
    let (state, current): (String, bool) = c
        .query_row(
            "SELECT state,current FROM media_end_intents WHERE effect_id IS NOT NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "legacy");
    assert!(!current);
    drop(c);
    assert!(f.store.claim_voice_job().unwrap().is_none());
    f.store
        .execute(&f.key.id, &call_end(&f, "fresh", &call))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
}

#[test]
fn old_sip_end_requires_a_new_issuer_instead_of_inheriting_the_dialer() {
    let (f, _, s) = sip_setup();
    let leg = active_sip(&f, &s);
    f.store
        .execute(&f.key.id, &sip_end(&f, "old", &leg))
        .unwrap();
    migrate(&f);
    assert!(f.store.claim_sip_leg_job().unwrap().is_none());
    f.store
        .execute(&f.key.id, &sip_end(&f, "fresh", &leg))
        .unwrap();
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert_eq!(job.credential_id, f.key.id);
    assert!(f.store.sip_dispatch_allowed(&job).unwrap());
}

#[test]
fn media_upgrade_failure_rolls_back_new_tables_and_preserves_the_verified_backup() {
    let (f, _) = call_setup();
    active_call(&f);
    schema_thirteen(&f);
    // Conflict after the first table is created must roll the entire migration back.
    f.store
        .connection()
        .unwrap()
        .execute_batch("CREATE TABLE media_start_admissions(conflict TEXT)")
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let backup = f._dir.path().join("failed-upgrade.sqlite3");
    f.store.backup(&backup).unwrap();
    assert_eq!(
        Store::migrate(f.store.path(), &backup, &before)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let exists: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='media_end_intents')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists);
}
