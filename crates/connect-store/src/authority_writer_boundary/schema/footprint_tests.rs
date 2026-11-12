use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

#[test]
fn combined_mutation_footprint_checks_all_subjects_and_keeps_victims_unconditional() {
    let c = Connection::open_in_memory().unwrap();
    let calls = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let refused = Arc::new(Mutex::new(None::<String>));
    let skip = Arc::new(AtomicBool::new(false));
    let skip_calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let denied = refused.clone();
    c.create_scalar_function(FUNCTION, 2, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let kind = ctx.get::<String>(0)?;
        let id = ctx.get::<Option<String>>(1)?;
        observed.lock().unwrap().push((kind.clone(), id.clone()));
        if kind == "unresolved-row"
            || id.as_ref() == denied.lock().unwrap().as_ref() && id.is_some()
        {
            return Err(rusqlite::Error::UserFunctionError(Box::new(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "test subject refused"),
            )));
        }
        Ok(1_i64)
    })
    .unwrap();
    let selected = skip.clone();
    let counted = skip_calls.clone();
    c.create_scalar_function(
        "test_exact_skip",
        0,
        FunctionFlags::SQLITE_UTF8,
        move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(i64::from(selected.load(Ordering::Relaxed)))
        },
    )
    .unwrap();
    c.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, owner TEXT, cid TEXT); INSERT INTO items VALUES(1,'old-owner','old-cid'),(2,'victim-owner','victim-cid');").unwrap();
    let policy = Table {
        table: "items".into(),
        must_resolve_base_scope: true,
        scope_queries_for_each_row: vec![
            ScopeQuery {
                kind: "Conversation".into(),
                id_sql: "SELECT {row}.cid AS id WHERE {row}.cid IS NOT NULL".into(),
                role: "base".into(),
            },
            ScopeQuery {
                kind: "Identity".into(),
                id_sql: "SELECT {row}.owner AS id".into(),
                role: "owner".into(),
            },
            ScopeQuery {
                kind: "Identity".into(),
                id_sql:
                    "SELECT {row}.owner || '-a' AS id UNION ALL SELECT {row}.owner || '-b' AS id"
                        .into(),
                role: "delegate".into(),
            },
        ],
    };
    let own = footprint_many(&policy, &["OLD", "NEW"], "test_exact_skip()").unwrap();
    // Use the same unskipped victim generator and correlated scalar wrapper as
    // definitions(): an exact own-row exception never covers another old row.
    let victim = footprint(&policy, "existing", "0").unwrap();
    let mut body = format!("SELECT {};{own}", guard("bookkeeping", "NULL"));
    for statement in victim.split(';').filter(|s| !s.is_empty()) {
        body.push_str(&format!(
            "SELECT ({statement}) FROM items existing WHERE existing.id=2;"
        ));
    }
    c.execute_batch(&format!(
        "CREATE TRIGGER checked BEFORE UPDATE ON items WHEN OLD.id=1 BEGIN {body} END;"
    ))
    .unwrap();

    c.execute(
        "UPDATE items SET owner='new-owner',cid='new-cid' WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(skip_calls.load(Ordering::Relaxed), 1);
    let observed = calls.lock().unwrap().clone();
    for id in [
        "old-cid",
        "old-owner",
        "old-owner-a",
        "old-owner-b",
        "new-cid",
        "new-owner",
        "new-owner-a",
        "new-owner-b",
        "victim-cid",
        "victim-owner",
        "victim-owner-a",
        "victim-owner-b",
    ] {
        assert_eq!(
            observed
                .iter()
                .filter(|(_, value)| value.as_deref() == Some(id))
                .count(),
            1,
            "every OLD/NEW and delegated/victim subject is visited: {id}"
        );
    }
    assert_eq!(observed.len(), 13); // Twelve exact subjects plus bookkeeping.

    // A later delegated row may refuse; earlier successful guards cannot make
    // the scalar expression short-circuit or permit the mutation.
    for id in ["new-owner-b", "next-owner-b"] {
        calls.lock().unwrap().clear();
        *refused.lock().unwrap() = Some(id.into());
        assert!(c
            .execute(
                "UPDATE items SET owner='next-owner',cid='next-cid' WHERE id=1",
                []
            )
            .is_err());
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, value)| value.as_deref() == Some(id)));
        assert_eq!(
            c.query_row("SELECT owner FROM items WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "new-owner"
        );
    }
    *refused.lock().unwrap() = None;
    assert!(c
        .execute("UPDATE items SET cid=NULL WHERE id=1", [])
        .is_err());
    assert_eq!(
        c.query_row("SELECT cid FROM items WHERE id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "new-cid"
    );

    skip.store(true, Ordering::Relaxed);
    skip_calls.store(0, Ordering::Relaxed);
    calls.lock().unwrap().clear();
    *refused.lock().unwrap() = Some("new-owner-b".into());
    c.execute("UPDATE items SET owner='skipped-owner' WHERE id=1", [])
        .unwrap();
    assert_eq!(skip_calls.load(Ordering::Relaxed), 1);
    assert_eq!(calls.lock().unwrap().len(), 5); // Bookkeeping plus all four victim subjects.
    *refused.lock().unwrap() = Some("victim-owner-b".into());
    assert!(c
        .execute("UPDATE items SET owner='forbidden' WHERE id=1", [])
        .is_err());
    assert_eq!(
        c.query_row("SELECT owner FROM items WHERE id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "skipped-owner"
    );

    // Zero delegated rows remain zero, not NULL, so empty aggregates cannot
    // suppress other terms through NULL arithmetic.
    let empty = Table {
        table: "items".into(),
        must_resolve_base_scope: false,
        scope_queries_for_each_row: vec![ScopeQuery {
            kind: "Identity".into(),
            id_sql: "SELECT owner AS id FROM items WHERE 0".into(),
            role: "delegate".into(),
        }],
    };
    assert_eq!(
        c.query_row(&footprint(&empty, "unused", "0").unwrap(), [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );

    // Installation has no dormant/scoped exception, including on UPDATE.
    calls.lock().unwrap().clear();
    skip_calls.store(0, Ordering::Relaxed);
    let installation = Table {
        table: "installation".into(),
        must_resolve_base_scope: false,
        scope_queries_for_each_row: Vec::new(),
    };
    assert_eq!(
        c.query_row(
            &footprint_many(&installation, &["OLD", "NEW"], "test_exact_skip()").unwrap(),
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(skip_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            ("exclusive-installation".into(), None),
            ("exclusive-installation".into(), None)
        ]
    );
}
