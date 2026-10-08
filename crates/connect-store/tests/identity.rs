use connect_protocol::Limits;
use connect_store::{Scope, Store};

#[test]
fn identities_scopes_and_revocation_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let store = Store::initialize(&path, Limits::default()).unwrap();
    let installation = store.installation_id().unwrap();
    let app = store.create_identity("app").unwrap();
    let agent = store.create_identity("agent").unwrap();
    assert_eq!(store.create_identity("app").unwrap_err().code, "conflict");
    let app_key = store
        .issue(
            &app.id,
            Scope::messaging(vec![agent.id.clone()]),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let operator = store.create_identity("operator").unwrap();
    let operator_key = store
        .issue(
            &operator.id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let actor = store.authenticate(&app_key.token).unwrap();
    assert_eq!(
        store.resolve(&actor, "connect:agent@local").unwrap().id,
        agent.id
    );
    assert_eq!(
        store
            .resolve(&actor, "connect:operator@local")
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        store
            .resolve(&actor, "connect:missing@local")
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        store.revoke(&actor, &operator_key.id).unwrap_err().code,
        "forbidden"
    );
    assert!(Store::initialize(&path, Limits::default()).is_err());
    drop(store);
    let store = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(store.installation_id().unwrap(), installation);
    assert_eq!(store.identity(&app.id).unwrap(), app);
    let operator = store.authenticate(&operator_key.token).unwrap();
    store.revoke(&operator, &app_key.id).unwrap();
    assert_eq!(
        store.revalidate(&actor.credential_id).unwrap_err().code,
        "unauthorized"
    );
    assert_eq!(
        store.authenticate(&app_key.token).unwrap_err().code,
        "unauthorized"
    );
    store.integrity_check().unwrap();
}

#[test]
fn refuses_unknown_schema_without_altering_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    drop(Store::initialize(&path, Limits::default()).unwrap());
    let c = rusqlite::Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", 999).unwrap();
    drop(c);
    assert!(Store::open(&path, Limits::default()).is_err());
    let c = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        c.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        999
    );
}
