//! Actual public task authority must observe time after waiting for the DB lock.
use super::fixture::Fixture;
use crate::{Scope, Store};
use connect_protocol::{tasks::*, Operation};
use std::{
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

type OperationUnderLock = Box<dyn FnOnce() -> bool + Send>;

fn scenario(
    f: Fixture,
    name: &str,
    expires: i64,
) -> (tempfile::TempDir, Arc<Store>, OperationUnderLock) {
    let task = f.task();
    let short = f.store.issue(&f.owner, Scope::operator(), expires).unwrap();
    let binding = InvocationBinding {
        kind: if matches!(name, "provider-issue" | "registered-issue") {
            InvocationKind::ProviderCall
        } else {
            InvocationKind::Native
        },
        adapter: "lock-expiry-test".into(),
        external_id: "resource-one".into(),
    };
    let definition = GrantDefinition {
        task_id: task.id.clone(),
        binding: binding.clone(),
        operations: vec![TaskPermission::History, TaskPermission::Send],
        expires_at: expires,
    };
    if name == "registered-issue" {
        f.store
            .register_task_binding(&f.key.id, &task.id, &binding)
            .unwrap();
    }
    let token = if matches!(
        name,
        "authenticate-exact" | "authenticate-local" | "invocation-effect"
    ) {
        Some(
            f.store
                .issue_local_task_grant(&f.key.id, "initial", &definition)
                .unwrap()
                .expose_secret()
                .to_owned(),
        )
    } else {
        None
    };
    let invocation = if name == "invocation-effect" {
        Some(
            f.store
                .authenticate_task_grant(token.as_ref().unwrap(), &binding)
                .unwrap(),
        )
    } else {
        None
    };
    let shared = Arc::new(f.store);
    let store = shared.clone();
    let run: OperationUnderLock = match name {
        "owner-read" => Box::new(move || {
            store
                .execute_task(
                    &short.id,
                    &TaskRequest::new(TaskCommand::Get { task_id: task.id }),
                )
                .is_err()
        }),
        "register" => Box::new(move || {
            store
                .register_task_binding(&short.id, &task.id, &binding)
                .is_err()
        }),
        "provider-issue" => {
            let parent = f.key.id.clone();
            Box::new(move || {
                store
                    .issue_verified_provider_task_grant(&parent, "blocked-issue", &definition)
                    .is_err()
            })
        }
        "local-issue" => {
            let parent = f.key.id.clone();
            Box::new(move || {
                store
                    .issue_local_task_grant(&parent, "blocked-issue", &definition)
                    .is_err()
            })
        }
        "registered-issue" => {
            let parent = f.key.id.clone();
            Box::new(move || {
                store
                    .issue_task_grant(&parent, "blocked-issue", &definition)
                    .is_err()
            })
        }
        _ => {
            let token = token.unwrap();
            match name {
                "authenticate-exact" => {
                    Box::new(move || store.authenticate_task_grant(&token, &binding).is_err())
                }
                "authenticate-local" => {
                    Box::new(move || store.authenticate_local_task_grant(&token).is_err())
                }
                "invocation-effect" => {
                    let invocation = invocation.unwrap();
                    let request = TaskRequest::new(TaskCommand::Invoke {
                        request_id: "expired-effect".into(),
                        task_id: task.id,
                        step_id: "send".into(),
                        wait_for_reply: false,
                        reply_to: None,
                        operation: Box::new(Operation::Send {
                            request_id: "expired-effect".into(),
                            conversation_id: f.cid.clone(),
                            message: connect_protocol::messages::MessageDraft::text_all(
                                "Must not send after expiry",
                            ),
                        }),
                    });
                    Box::new(move || {
                        store
                            .execute_task_invocation(&invocation, &request)
                            .is_err()
                    })
                }
                _ => panic!("unknown scenario"),
            }
        }
    };
    (f._dir, shared, run)
}

#[test]
fn expiry_while_waiting_for_the_store_lock_denies_public_authority_and_effects() {
    let mut results = Vec::new();
    for name in [
        "owner-read",
        "register",
        "provider-issue",
        "local-issue",
        "registered-issue",
        "authenticate-exact",
        "authenticate-local",
        "invocation-effect",
    ] {
        let f = Fixture::new();
        let expires = crate::now() + 3;
        let (_dir, store, operation) = scenario(f, name, expires);
        let before = Store::inspect_snapshot(store.path()).unwrap();
        let guard = store.connection().unwrap();
        let (started, received) = mpsc::sync_channel(0);
        let worker = thread::spawn(move || {
            started.send(crate::now()).unwrap();
            operation()
        });
        assert!(received.recv_timeout(Duration::from_secs(2)).unwrap() < expires);
        while crate::now() <= expires {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !worker.is_finished(),
            "{name} bypassed the retained store lock"
        );
        drop(guard);
        let denied = worker.join().unwrap();
        let unchanged = Store::inspect_snapshot(store.path()).unwrap() == before;
        results.push((name, denied, unchanged));
    }
    assert!(
        results
            .iter()
            .all(|(_, denied, unchanged)| *denied && *unchanged),
        "Expired authority after lock wait: {results:?}"
    );
}
