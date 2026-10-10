use super::{effect_helpers::*, fixture::Fixture, grants};
use crate::{Limits, Store, SCHEMA_VERSION};
use connect_protocol::tasks::*;

fn schema_eleven(f: &Fixture) {
    f.store.connection().unwrap().execute_batch("DROP TABLE media_end_intents; DROP TABLE media_start_admissions; DROP INDEX task_effect_media_resource; DROP TABLE task_effect_links; DROP TABLE task_effect_authority; DROP INDEX task_effect_question_lookup; PRAGMA user_version=11;").unwrap();
}
#[test]
fn additive_schema_eleven_upgrade_preserves_tasks_grants_and_existing_state() {
    let f = Fixture::new();
    let task = f.task();
    let issued = grants::grant(&f, &task);
    schema_eleven(&f);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(before.schema, 11);
    let backup = f._dir.path().join("eleven.sqlite3");
    f.store.backup(&backup).unwrap();
    let migrated = Store::migrate(f.store.path(), &backup, &before).unwrap();
    assert_eq!(migrated.schema, SCHEMA_VERSION);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let current = Store::open(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        Fixture::record(
            current
                .execute_task(&f.key.id, &grants::get(&task))
                .unwrap()
        ),
        task
    );
    let handle = current
        .authenticate_task_grant(issued.expose_secret(), &issued.grant.binding)
        .unwrap();
    assert!(matches!(
        current
            .execute_task_invocation(
                &handle,
                &send(&f, &task, "new-effect", "A new schema 12 effect", false)
            )
            .unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
}
#[test]
fn failed_schema_eleven_upgrade_is_transactional_and_stale_backup_is_rejected() {
    let f = Fixture::new();
    f.task();
    schema_eleven(&f);
    // An interrupted/manual conflicting schema object makes the additive migration fail.
    f.store
        .connection()
        .unwrap()
        .execute_batch("CREATE TABLE task_effect_authority(conflict TEXT);")
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let backup = f._dir.path().join("eleven.sqlite3");
    f.store.backup(&backup).unwrap();
    assert_eq!(
        Store::migrate(f.store.path(), &backup, &before)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row::<u64, _, _>(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='task_effect_links'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    c.execute_batch("DROP TABLE task_effect_authority; UPDATE tasks SET reason='newer intent';")
        .unwrap();
    drop(c);
    assert_eq!(
        Store::migrate(f.store.path(), &backup, &before)
            .unwrap_err()
            .code,
        "invalid-backup"
    );
}

#[test]
fn legacy_effect_without_dispatch_metadata_cannot_be_promoted_or_reexecuted() {
    let f = Fixture::new();
    let task = f.task();
    let request = send(&f, &task, "old-effect", "Legacy outcome", false);
    f.run(&request).unwrap();
    schema_eleven(&f);
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE task_effects SET state='dispatched'", [])
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let backup = f._dir.path().join("legacy-effect.sqlite3");
    f.store.backup(&backup).unwrap();
    Store::migrate(f.store.path(), &backup, &before).unwrap();
    let unchanged = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(effect(&f, &task).state, EffectState::Dispatched);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), unchanged);
    assert_eq!(f.run(&request).unwrap_err().code, "forbidden");
    assert_eq!(count(&f, "messages"), 1);
}
