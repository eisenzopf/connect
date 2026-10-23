//! Fresh-state conversation policy, shared receipts and current bounded listing.
#![allow(clippy::unwrap_used)]
use connect_protocol::{conversations::*, transitions::*, Limits, Operation, Outcome};
use connect_store::{Scope, Store};
use rusqlite::{params, Connection};
use serde_json::json;
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    owner: String,
    peer: String,
    token: String,
    peer_token: String,
}
impl Fixture {
    fn new(limits: Limits) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::initialize(&dir.path().join("state.sqlite3"), limits).unwrap();
        let owner = store.create_identity("owner").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let token = store
            .issue(
                &owner,
                Scope::collaboration(vec!["*".into()]),
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap()
            .id;
        let peer_token = store
            .issue(
                &peer,
                Scope::collaboration(vec!["*".into()]),
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap()
            .id;
        Self {
            dir,
            store,
            owner,
            peer,
            token,
            peer_token,
        }
    }
    fn sql(&self) -> Connection {
        Connection::open(self.dir.path().join("state.sqlite3")).unwrap()
    }
    fn create(&self, id: &str, request: CreateRequest) -> Conversation {
        let Outcome::Conversation(v) = self
            .store
            .execute(
                &self.token,
                &Operation::Create {
                    request_id: id.into(),
                    request,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        v
    }
    fn input(&self) -> CreateRequest {
        CreateRequest::persistent(vec![self.peer.clone()])
    }
    fn close(
        &self,
        id: &str,
        cid: &str,
        options: CloseOptions,
    ) -> connect_protocol::Result<Outcome> {
        self.store.execute(
            &self.token,
            &Operation::Close {
                request_id: id.into(),
                conversation_id: cid.into(),
                options,
            },
        )
    }
}
#[test]
fn conversation_policy_metadata_roles_and_current_receipt_survive_restart() {
    let f = Fixture::new(Limits::default());
    let mut input = f.input();
    input.tenant_id = Some("tnt_default".into());
    input.metadata = Some(json!({"label":"private metadata","number":1.0,"nested":{"z":true}}));
    input.initial_participants[0].role = "guest".into();
    let created = f.create("create", input.clone());
    assert_eq!(created.policy, ConversationPolicy::Persistent);
    assert_eq!(created.idle_close_secs, None);
    assert_eq!(created.metadata.as_ref().unwrap()["number"], 1);
    assert_eq!(
        created
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer)
            .unwrap()
            .role,
        "guest"
    );
    assert_ne!(
        created.participants[0].participant_id,
        created.participants[0].identity_id
    );
    assert_eq!(created.receipt.as_ref().unwrap().operation_id, "create");
    let close = CloseOptions {
        expected_revision: Some(created.revision),
        reason_code: Some(42),
        reason: Some("completed conversation".into()),
    };
    let closed = f.close("close", &created.id, close.clone()).unwrap();
    assert_eq!(closed, f.close("close", &created.id, close).unwrap());
    let Outcome::Conversation(closed) = closed else {
        panic!()
    };
    assert!(closed.closed);
    assert_eq!(closed.reason_code, Some(42));
    assert!(closed.closed_at.is_some());
    assert!(closed.revision > created.revision);
    let current = f.create("create", input.clone());
    assert!(current.closed);
    assert_eq!(current.id, created.id);
    assert_eq!(current.receipt, created.receipt);
    let record = f
        .store
        .execute_operation_action(
            &f.token,
            OperationAction::Get {
                operation_id: "create".into(),
            },
        )
        .unwrap();
    assert_eq!(record.result.unwrap()["data"]["closed"], true);
    let id = f
        .store
        .issue(
            &f.owner,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
        .id;
    drop(f.store);
    let store = Store::open(&f.dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let result = store
        .execute(
            &id,
            &Operation::Create {
                request_id: "create".into(),
                request: input,
            },
        )
        .unwrap();
    let Outcome::Conversation(v) = result else {
        panic!()
    };
    assert_eq!(v, current);
}
#[test]
fn conversation_create_rejects_spoof_duplicate_metadata_and_changed_same_id() {
    let f = Fixture::new(Limits::default());
    let mut request = f.input();
    request.tenant_id = Some("tnt_other".into());
    assert_eq!(
        f.store
            .execute(
                &f.token,
                &Operation::Create {
                    request_id: "spoof".into(),
                    request
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut request = f.input();
    request
        .initial_participants
        .push(request.initial_participants[0].clone());
    assert_eq!(
        f.store
            .execute(
                &f.token,
                &Operation::Create {
                    request_id: "duplicate".into(),
                    request
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let mut request = f.input();
    request.metadata = Some(json!("x".repeat(MAX_METADATA_BYTES)));
    assert_eq!(
        f.store
            .execute(
                &f.token,
                &Operation::Create {
                    request_id: "large".into(),
                    request
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let mut request = f.input();
    request.metadata = Some(json!(9007199254740992u64));
    assert_eq!(
        f.store
            .execute(
                &f.token,
                &Operation::Create {
                    request_id: "unsafe".into(),
                    request
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let created = f.create("same", f.input());
    let mut changed = f.input();
    changed.policy = ConversationPolicy::Ephemeral;
    assert_eq!(
        f.store
            .execute(
                &f.token,
                &Operation::Create {
                    request_id: "same".into(),
                    request: changed
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.create("same", f.input()).id, created.id);
    assert_eq!(
        f.sql()
            .query_row("SELECT COUNT(*) FROM conversations", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn conversation_close_revision_reason_and_retained_rejection_are_atomic() {
    let f = Fixture::new(Limits::default());
    let created = f.create("create", f.input());
    let bad = CloseOptions {
        expected_revision: Some(created.revision + 1),
        ..Default::default()
    };
    assert_eq!(
        f.close("stale", &created.id, bad.clone()).unwrap_err().code,
        "conflict"
    );
    assert!(!f.create("create", f.input()).closed);
    let good = CloseOptions {
        expected_revision: Some(created.revision),
        ..Default::default()
    };
    let closed = f.close("close", &created.id, good.clone()).unwrap();
    assert_eq!(closed, f.close("close", &created.id, good).unwrap());
    assert_eq!(
        f.close("stale", &created.id, bad).unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        f.close(
            "other",
            &created.id,
            CloseOptions {
                reason: Some("different".into()),
                ..Default::default()
            }
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        f.sql()
            .query_row(
                "SELECT COUNT(*) FROM journal WHERE kind='conversation_closed'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        2
    );
}
#[test]
fn conversation_list_filters_cursor_binding_and_live_membership() {
    let f = Fixture::new(Limits::default());
    let first = f.create("one", f.input());
    let second = f.create("two", f.input());
    let third = f.create("three", CreateRequest::persistent(vec![]));
    f.close("close", &second.id, Default::default()).unwrap();
    let query = ListQuery {
        filter: ListFilter {
            state: ConversationState::All,
            ..Default::default()
        },
        limit: Some(1),
        ..Default::default()
    };
    let page = f.store.list_conversations(&f.token, query.clone()).unwrap();
    assert_eq!(page.conversations[0].id, first.id);
    let cursor = page.next_cursor.unwrap();
    let mut next = query.clone();
    next.cursor = Some(cursor.clone());
    assert_eq!(
        f.store
            .list_conversations(&f.token, next.clone())
            .unwrap()
            .conversations[0]
            .id,
        second.id
    );
    assert_eq!(
        f.store
            .list_conversations(&f.peer_token, next.clone())
            .unwrap_err()
            .code,
        "invalid-cursor"
    );
    next.filter.state = ConversationState::Open;
    assert_eq!(
        f.store.list_conversations(&f.token, next).unwrap_err().code,
        "invalid-cursor"
    );
    let mut q = ListQuery::default();
    q.filter.identity_id = Some(f.peer.clone());
    let p = f.store.list_conversations(&f.token, q.clone()).unwrap();
    assert_eq!(p.conversations.len(), 1);
    assert_eq!(p.conversations[0].id, first.id);
    q.filter.state = ConversationState::Closed;
    assert_eq!(
        f.store
            .list_conversations(&f.token, q)
            .unwrap()
            .conversations[0]
            .id,
        second.id
    );
    f.sql()
        .execute(
            "UPDATE participants SET active=0 WHERE conversation_id=?1 AND identity_id=?2",
            params![first.id, f.owner],
        )
        .unwrap();
    let mut next = query;
    next.cursor = Some(cursor);
    let p = f.store.list_conversations(&f.token, next).unwrap();
    assert_eq!(p.conversations[0].id, second.id);
    assert_ne!(p.conversations[0].id, third.id);
}
#[test]
fn conversation_idle_policy_default_and_persistent_inert_configuration() {
    let f = Fixture::new(Limits::default());
    let mut ephemeral = f.input();
    ephemeral.policy = ConversationPolicy::Ephemeral;
    let e = f.create("ephemeral", ephemeral);
    assert_eq!(e.idle_close_secs, Some(60));
    let mut p = f.input();
    p.idle_close_secs = Some(1);
    let persistent = f.create("persistent", p);
    f.sql()
        .execute(
            "UPDATE conversations SET opened_at=?1",
            [chrono::Utc::now().timestamp() - 120],
        )
        .unwrap();
    assert_eq!(f.store.expire_idle_conversations(32).unwrap(), 1);
    assert!(
        f.create("ephemeral", {
            let mut r = f.input();
            r.policy = ConversationPolicy::Ephemeral;
            r
        })
        .closed
    );
    assert!(
        !f.create("persistent", {
            let mut r = f.input();
            r.idle_close_secs = Some(1);
            r
        })
        .closed
    );
    assert_eq!(f.store.expire_idle_conversations(32).unwrap(), 0);
    assert_eq!(
        f.store
            .list_conversations(&f.token, ListQuery::default())
            .unwrap()
            .conversations[0]
            .id,
        persistent.id
    );
}
#[test]
fn conversation_unknown_session_blocks_explicit_and_idle_close() {
    let f = Fixture::new(Limits::default());
    let mut r = f.input();
    r.policy = ConversationPolicy::Ephemeral;
    r.idle_close_secs = Some(1);
    let v = f.create("ephemeral", r);
    let c = f.sql();
    c.execute(
        "UPDATE conversations SET opened_at=?1 WHERE id=?2",
        params![chrono::Utc::now().timestamp() - 120, v.id],
    )
    .unwrap();
    c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES('unknown_session',?1,?2,?3,'unknown','[]',1,1)",params![v.id,f.owner,f.peer]).unwrap();
    assert_eq!(
        f.close("close", &v.id, Default::default())
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.store.expire_idle_conversations(32).unwrap(), 0);
}
#[test]
fn conversation_journal_capacity_rolls_back_create_and_idle_close() {
    let f = Fixture::new(Limits {
        journal_events: 2,
        ..Limits::default()
    });
    let mut r = f.input();
    r.policy = ConversationPolicy::Ephemeral;
    r.idle_close_secs = Some(1);
    let v = f.create("one", r);
    assert_eq!(
        f.store
            .execute(
                &f.token,
                &Operation::Create {
                    request_id: "two".into(),
                    request: f.input()
                }
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(
        f.sql()
            .query_row("SELECT COUNT(*) FROM conversations", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        1
    );
    f.sql()
        .execute("UPDATE conversations SET opened_at=1 WHERE id=?1", [v.id])
        .unwrap();
    assert_eq!(
        f.store.expire_idle_conversations(32).unwrap_err().code,
        "capacity-exceeded"
    );
    assert_eq!(
        f.sql()
            .query_row(
                "SELECT COUNT(*) FROM conversations WHERE closed=1",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn conversation_list_bounds_closed_facts_and_keeps_a_fixed_row_highwater() {
    let f = Fixture::new(Limits::default());
    for n in 0..66 {
        let c = f.create(&format!("create-{n}"), CreateRequest::persistent(vec![]));
        f.close(&format!("close-{n}"), &c.id, Default::default())
            .unwrap();
    }
    let query = ListQuery {
        filter: ListFilter {
            state: ConversationState::All,
            ..Default::default()
        },
        limit: Some(100),
        ..Default::default()
    };
    let page = f.store.list_conversations(&f.token, query.clone()).unwrap();
    assert_eq!(page.conversations.len(), 63);
    assert!(page.conversations.iter().all(|c| c.closed));
    let new = f.create("later", CreateRequest::persistent(vec![]));
    let next = f
        .store
        .list_conversations(
            &f.token,
            ListQuery {
                cursor: page.next_cursor,
                ..query
            },
        )
        .unwrap();
    assert_eq!(next.conversations.len(), 3);
    assert!(next.conversations.iter().all(|c| c.id != new.id));
    assert!(next.next_cursor.is_none());
}
#[test]
fn conversation_list_time_participant_tenant_and_recipient_filters_are_current() {
    let f = Fixture::new(Limits::default());
    let one = f.create("one", f.input());
    let two = f.create("two", f.input());
    let c = f.sql();
    c.execute(
        "UPDATE conversations SET opened_at=100 WHERE id=?1",
        [&one.id],
    )
    .unwrap();
    c.execute(
        "UPDATE conversations SET opened_at=200 WHERE id=?1",
        [&two.id],
    )
    .unwrap();
    let query = ListQuery {
        filter: ListFilter {
            since: chrono::DateTime::from_timestamp(150, 1),
            until: chrono::DateTime::from_timestamp(200, 0),
            ..Default::default()
        },
        ..Default::default()
    };
    let page = f.store.list_conversations(&f.token, query).unwrap();
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.conversations[0].id, two.id);
    let pid = one
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer)
        .unwrap()
        .participant_id
        .clone();
    let q = ListQuery {
        filter: ListFilter {
            participant_id: Some(pid),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        f.store
            .list_conversations(&f.token, q)
            .unwrap()
            .conversations[0]
            .id,
        one.id
    );
    assert_eq!(
        f.store
            .list_conversations(
                &f.token,
                ListQuery {
                    filter: ListFilter {
                        tenant_id: Some("tnt_foreign".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let token = f
        .store
        .issue(
            &f.owner,
            Scope::messaging(vec![]),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
        .id;
    let page = f
        .store
        .list_conversations(&token, ListQuery::default())
        .unwrap();
    assert!(page
        .conversations
        .iter()
        .all(|c| c.participants.len() == 1 && c.participants[0].identity_id == f.owner));
    assert_eq!(
        f.store
            .list_conversations(
                &token,
                ListQuery {
                    filter: ListFilter {
                        identity_id: Some(f.peer),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn conversation_concurrent_retry_commits_one_id_and_storage_failure_reserves_nothing() {
    let f = Fixture::new(Limits::default());
    let store = std::sync::Arc::new(f.store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let operation = Operation::Create {
        request_id: "concurrent-create".into(),
        request: CreateRequest::persistent(vec![f.peer]),
    };
    let mut jobs = vec![];
    for _ in 0..2 {
        let store = store.clone();
        let token = f.token.clone();
        let op = operation.clone();
        let b = barrier.clone();
        jobs.push(std::thread::spawn(move || {
            b.wait();
            store.execute(&token, &op).unwrap()
        }));
    }
    barrier.wait();
    let first = jobs.remove(0).join().unwrap();
    let second = jobs.remove(0).join().unwrap();
    assert_eq!(first, second);
    let c = Connection::open(f.dir.path().join("state.sqlite3")).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM conversations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    c.execute_batch("CREATE TRIGGER reject_conversation BEFORE INSERT ON conversations BEGIN SELECT RAISE(FAIL,'injected storage failure');END").unwrap();
    let op = Operation::Create {
        request_id: "storage-create".into(),
        request: CreateRequest::persistent(vec![]),
    };
    assert_eq!(
        store.execute(&f.token, &op).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='storage-create'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER reject_conversation").unwrap();
    assert!(store.execute(&f.token, &op).is_ok());
}
