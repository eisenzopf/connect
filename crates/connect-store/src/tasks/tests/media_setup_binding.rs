use super::media_setup::fixture;
use crate::{Limits, Store, SCHEMA_VERSION};

#[test]
fn original_setup_survives_reopen_and_cannot_be_replaced_or_borrowed() {
    let (m, session, caller, definition, route) = fixture();
    let reopened = Store::open(m.f.store.path(), Limits::default()).unwrap();
    assert!(reopened
        .vapi_session_requires_task_setup(&session.id)
        .unwrap());
    let before = Store::inspect_snapshot(reopened.path()).unwrap();
    let mut replacement = definition.clone();
    replacement.binding.external_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        reopened
            .register_vapi_task_session_setup(&m.f.key.id, &replacement, &session.id, &route,)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        reopened
            .admit_vapi_task_session_call(&m.f.key.id, &replacement, &caller, &session.id, &route,)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(reopened.path()).unwrap(), before);
    assert!(reopened
        .admit_vapi_task_session_call(&m.f.key.id, &definition, &caller, &session.id, &route,)
        .unwrap());
    assert!(!reopened
        .admit_vapi_task_session_call(&m.f.key.id, &definition, &caller, &session.id, &route,)
        .unwrap());
}

#[test]
fn failed_setup_registration_rolls_back_and_preserves_retry_identity() {
    let (m, _, caller, mut definition, mut route) = fixture();
    let session = m.accepted_outbound("new-session", true);
    route.session_id = session.id.clone();
    definition.binding.external_id = uuid::Uuid::new_v4().to_string();
    m.f.store.connection().unwrap().execute_batch(
        "CREATE TRIGGER fail_setup BEFORE INSERT ON vapi_session_setups BEGIN SELECT RAISE(ABORT,'fixture'); END;",
    ).unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .register_vapi_task_session_setup(&m.f.key.id, &definition, &session.id, &route,)
            .unwrap_err()
            .code,
        connect_protocol::Error::storage().code
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    // An uncommitted setup cannot admit this protected provider intent.
    // No marker has been installed to change the session's voice-only behavior.
    assert!(!m
        .f
        .store
        .vapi_session_requires_task_setup(&session.id)
        .unwrap());
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &definition, &caller, &session.id, &route,)
        .is_err());
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_setup")
        .unwrap();
    m.f.store
        .register_vapi_task_session_setup(&m.f.key.id, &definition, &session.id, &route)
        .unwrap();
    assert!(m
        .f
        .store
        .admit_vapi_task_session_call(&m.f.key.id, &definition, &caller, &session.id, &route,)
        .unwrap());
}

#[test]
fn schema_fifteen_upgrade_preserves_sessions_without_inventing_lost_setup_intent() {
    let (m, session, caller, definition, route) = fixture();
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TABLE vapi_session_setups; PRAGMA user_version=15;")
        .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let backup = m.f._dir.path().join("before-setup-migration.sqlite3");
    m.f.store.backup(&backup).unwrap();
    let after = Store::migrate(m.f.store.path(), &backup, &before).unwrap();
    assert_eq!(after.schema, SCHEMA_VERSION);
    assert_eq!(after.installation_id, before.installation_id);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let reopened = Store::open(m.f.store.path(), Limits::default()).unwrap();
    assert!(reopened
        .vapi_session_requires_task_setup(&session.id)
        .unwrap());
    assert_eq!(
        reopened
            .authorize_vapi_task_session_setup(&m.f.key.id, &definition.task_id, &session.id,)
            .unwrap(),
        route
    );
    assert_eq!(
        reopened
            .register_vapi_task_session_setup(&m.f.key.id, &definition, &session.id, &route,)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        reopened
            .admit_vapi_task_session_call(&m.f.key.id, &definition, &caller, &session.id, &route,)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(reopened.path()).unwrap(), after);
}

#[test]
fn setup_migration_failure_rolls_back_and_does_not_modify_verified_backup() {
    let (m, _, _, _, _) = fixture();
    m.f.store.connection().unwrap().execute_batch(
        "DROP TABLE vapi_session_setups; CREATE TABLE vapi_session_setups(conflict TEXT); PRAGMA user_version=15;",
    ).unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let backup = m.f._dir.path().join("failed-setup-migration.sqlite3");
    m.f.store.backup(&backup).unwrap();
    assert!(Store::migrate(m.f.store.path(), &backup, &before).is_err());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
}

#[test]
fn competing_registrations_keep_one_original_intent_across_store_handles() {
    let (m, _, caller, definition, mut route) = fixture();
    let session = m.accepted_outbound("competing-session", true);
    route.session_id = session.id.clone();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut attempts = Vec::new();
    for _ in 0..2 {
        let store = Store::open(m.f.store.path(), Limits::default()).unwrap();
        let (credential, session, mut proposed, route, barrier) = (
            m.f.key.id.clone(),
            session.id.clone(),
            definition.clone(),
            route.clone(),
            barrier.clone(),
        );
        proposed.binding.external_id = uuid::Uuid::new_v4().to_string();
        attempts.push(std::thread::spawn(move || {
            barrier.wait();
            let result =
                store.register_vapi_task_session_setup(&credential, &proposed, &session, &route);
            (proposed, result)
        }));
    }
    let outcomes: Vec<_> = attempts
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(
        outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
        1
    );
    for (proposed, result) in outcomes {
        let admission = m.f.store.admit_vapi_task_session_call(
            &m.f.key.id,
            &proposed,
            &caller,
            &session.id,
            &route,
        );
        if let Err(error) = result {
            assert_eq!(error.code, "conflict");
            assert_eq!(admission.unwrap_err().code, "forbidden");
        } else {
            assert!(admission.unwrap());
        }
    }
}

#[test]
fn one_private_invocation_cannot_be_registered_for_two_sessions() {
    let (m, _, _, definition, mut route) = fixture();
    let second = m.accepted_outbound("second-session", true);
    route.session_id = second.id.clone();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .register_vapi_task_session_setup(&m.f.key.id, &definition, &second.id, &route,)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert!(!m
        .f
        .store
        .vapi_session_requires_task_setup(&second.id)
        .unwrap());
}
