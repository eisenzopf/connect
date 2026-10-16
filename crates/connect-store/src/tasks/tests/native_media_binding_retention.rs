use super::native_resource_fixture::MediaFixture;
use connect_protocol::tasks::*;

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
