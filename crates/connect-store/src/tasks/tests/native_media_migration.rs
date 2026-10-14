use super::native_resource_fixture::MediaFixture;
use crate::{Store, SCHEMA_VERSION};
use connect_protocol::tasks::*;

fn downgrade(m: &MediaFixture) {
    m.f.store
        .connection()
        .unwrap()
        .execute_batch(
            "DROP TABLE connector_observations; DROP TABLE connector_receipt_events; DROP TABLE connector_incoming; DROP TABLE connector_voice_observations; DROP TABLE contact_pairings; DROP TABLE connector_manifests; DROP TABLE vapi_session_setups; DROP INDEX session_bindings_task_grant;
         ALTER TABLE session_bindings DROP COLUMN task_grant_id;
         PRAGMA user_version=14;",
        )
        .unwrap();
}

#[test]
fn backup_gated_native_media_upgrade_preserves_owner_binding_and_existing_authority() {
    let m = MediaFixture::new();
    let session = m.accepted_outbound("session", true);
    m.f.store
        .bind_session_connection(&m.f.key.id, &session.id, "owner-connection")
        .unwrap();
    let subject = m.grant(
        &m.task,
        "grant",
        vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
    );
    downgrade(&m);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let backup = m.f._dir.path().join("schema14.sqlite3");
    m.f.store.backup(&backup).unwrap();
    let after = Store::migrate(m.f.store.path(), &backup, &before).unwrap();
    assert_eq!(after.schema, SCHEMA_VERSION);
    assert_eq!(after.installation_id, before.installation_id);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let c = m.f.store.connection().unwrap();
    let (credential, grant): (String, Option<String>) = c.query_row(
        "SELECT credential_id,task_grant_id FROM session_bindings WHERE connection_id='owner-connection'", [], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(credential, m.f.key.id);
    assert!(grant.is_none());
    drop(c);
    assert!(m
        .f
        .store
        .authorize_native_task_session_binding(&subject, &session.id)
        .is_ok());
    m.f.store
        .bind_session_connection(&subject, &session.id, "native-connection")
        .unwrap();
    let c = m.f.store.connection().unwrap();
    let (credential, grant): (String, String) = c.query_row(
        "SELECT credential_id,task_grant_id FROM session_bindings WHERE connection_id='native-connection'", [], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(credential, m.f.key.id);
    assert_eq!(grant, task_grant_id(&subject).unwrap());
    drop(c);
    assert_eq!(
        m.f.store
            .unbind_session_connection("native-connection")
            .unwrap()
            .unwrap()
            .id,
        session.id
    );
}

#[test]
fn native_media_migration_failure_rolls_back_column_and_preserves_verified_backup() {
    let m = MediaFixture::new();
    downgrade(&m);
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("CREATE INDEX session_bindings_task_grant ON session_bindings(session_id)")
        .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let backup = m.f._dir.path().join("schema14-failed.sqlite3");
    m.f.store.backup(&backup).unwrap();
    assert_eq!(
        Store::migrate(m.f.store.path(), &backup, &before)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let exists: bool = m.f.store.connection().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('session_bindings') WHERE name='task_grant_id')", [], |r| r.get(0),
    ).unwrap();
    assert!(!exists);
}

#[test]
fn retained_binding_metadata_cannot_turn_a_deleted_grant_into_owner_authority() {
    let m = MediaFixture::new();
    let session = m.accepted_outbound("session", true);
    let subject = m.grant(
        &m.task,
        "grant",
        vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
    );
    m.f.store
        .bind_session_connection(&subject, &session.id, "native")
        .unwrap();
    let c = m.f.store.connection().unwrap();
    c.execute(
        "DELETE FROM task_grants WHERE id=?1",
        [task_grant_id(&subject).unwrap()],
    )
    .unwrap();
    let metadata: Option<String> = c
        .query_row(
            "SELECT task_grant_id FROM session_bindings WHERE connection_id='native'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(metadata.is_none());
    drop(c);
    assert!(m
        .f
        .store
        .authorize_native_task_session_binding(&subject, &session.id)
        .is_err());
    assert!(m
        .f
        .store
        .bind_session_connection(&subject, &session.id, "replacement")
        .is_err());
    assert!(m
        .f
        .store
        .authorize_session_binding(&m.f.key.id, &session.id)
        .is_ok());
    m.f.store.unbind_session_connection("native").unwrap();
}
