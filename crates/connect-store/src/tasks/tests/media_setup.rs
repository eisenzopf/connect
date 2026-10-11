//! Physical call admission retains the current task, source and route boundary.
use super::native_resource_fixture::MediaFixture;
use crate::{Store, VoiceRoute};
use connect_protocol::tasks::*;

fn fixture() -> (
    MediaFixture,
    connect_protocol::Session,
    String,
    GrantDefinition,
    crate::VapiSessionRoute,
) {
    let m = MediaFixture::new();
    m.f.store
        .configure_voice_route(
            &m.f.key.id,
            &VoiceRoute {
                id: "media-bootstrap".into(),
                conversation_id: m.f.cid.clone(),
                identity_id: m.peer_id.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    let session = m.accepted_outbound("task-voice", true);
    let subject = m.grant(
        &m.task,
        "media-source",
        vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
    );
    let definition = GrantDefinition {
        task_id: m.task.id.clone(),
        binding: InvocationBinding {
            kind: InvocationKind::ProviderCall,
            adapter: "provider-bootstrap".into(),
            external_id: uuid::Uuid::new_v4().to_string(),
        },
        operations: vec![TaskPermission::SessionGet],
        expires_at: m.f.now + 120,
    };
    let route =
        m.f.store
            .authorize_vapi_task_session_setup(&m.f.key.id, &m.task.id, &session.id)
            .unwrap();
    (m, session, subject, definition, route)
}
#[test]
fn one_physical_call_admission_never_repeats_and_denied_source_cannot_borrow_owner() {
    let (m, s, source, d, route) = fixture();
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &route)
        .unwrap());
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert!(!m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &route)
        .unwrap());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    m.f.store
        .revoke_task_grant(&m.f.key.id, task_grant_id(&source).unwrap())
        .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let mut fresh = d;
    fresh.binding.external_id = uuid::Uuid::new_v4().to_string();
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &fresh, &source, &s.id, &route)
        .is_err());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
#[test]
fn foreign_task_scope_expiry_route_and_cancellation_leave_no_admission() {
    let (m, s, source, d, route) = fixture();
    let second = MediaFixture::create_task(&m.f, &m.peer_id, "other-media-task");
    let mut other = d.clone();
    other.task_id = second.id;
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert!(m
        .f
        .store
        .authorize_vapi_task_session_setup(&m.f.key.id, &other.task_id, &s.id)
        .is_err());
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &other, &source, &s.id, &route)
        .is_err());
    let mut wider = d.clone();
    wider.operations.push(TaskPermission::Send);
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &wider, &source, &s.id, &route)
        .is_err());
    let mut expired = d.clone();
    expired.expires_at = crate::now() - 1;
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &expired, &source, &s.id, &route)
        .is_err());
    let mut mismatch = route.clone();
    mismatch.assistant_id = "other".into();
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &mismatch)
        .is_err());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    m.f.run(&TaskRequest::new(TaskCommand::Cancel {
        task_id: m.task.id.clone(),
        request_id: "cancel-bootstrap".into(),
        reason: None,
    }))
    .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &route)
        .is_err());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
#[test]
fn sqlite_failure_rolls_back_physical_admission_and_exact_retry_is_possible() {
    let (m, s, source, d, route) = fixture();
    let c = m.f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER fail_call_admission BEFORE INSERT ON media_start_admissions BEGIN SELECT RAISE(ABORT,'test fault'); END;").unwrap();
    drop(c);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &route)
            .unwrap_err()
            .code,
        connect_protocol::Error::storage().code
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_call_admission;")
        .unwrap();
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &route)
        .unwrap());
}

#[test]
fn competing_store_handles_and_restart_keep_one_call_admission() {
    let (m, s, source, d, route) = fixture();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let store = Store::open(m.f.store.path(), connect_protocol::Limits::default()).unwrap();
        let (owner, d, source, session, route, barrier) = (
            m.f.key.id.clone(),
            d.clone(),
            source.clone(),
            s.id.clone(),
            route.clone(),
            barrier.clone(),
        );
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            store
                .admit_vapi_task_session_call(&owner, &d, &source, &session, &route)
                .unwrap()
        }));
    }
    assert_eq!(
        workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .filter(|admitted| *admitted)
            .count(),
        1
    );
    let reopened = Store::open(m.f.store.path(), connect_protocol::Limits::default()).unwrap();
    assert!(!reopened
        .admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &route)
        .unwrap());
}

#[test]
fn expiry_during_lock_wait_denies_physical_admission_without_mutation() {
    let (m, s, source, mut d, route) = fixture();
    d.expires_at = crate::now() + 3;
    let expires = d.expires_at;
    let store = std::sync::Arc::new(m.f.store);
    let before = Store::inspect_snapshot(store.path()).unwrap();
    let guard = store.connection().unwrap();
    let (sent, received) = std::sync::mpsc::sync_channel(0);
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || {
        sent.send(crate::now()).unwrap();
        worker_store.admit_vapi_task_session_call(&m.f.key.id, &d, &source, &s.id, &route)
    });
    assert!(
        received
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            < expires
    );
    while crate::now() <= expires {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!worker.is_finished());
    drop(guard);
    assert!(worker.join().unwrap().is_err());
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
}
