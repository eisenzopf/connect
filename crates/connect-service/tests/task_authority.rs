#![allow(clippy::unwrap_used)]
use connect_protocol::{tasks::*, Limits, Operation, Outcome};
use connect_service::Service;
use connect_store::{Scope, Store};
use std::sync::Arc;

#[tokio::test]
async fn facade_revalidates_grants_and_exposes_only_non_secret_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let owner = store.create_identity("owner").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let key = store.issue(&owner, Scope::operator(), now + 600).unwrap();
    let cid = match store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "conversation".into(),
                participants: vec![],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("conversation"),
    };
    let service = Service::new(store);
    let task = match service
        .execute_task(
            key.id.clone(),
            TaskRequest::new(TaskCommand::Create {
                request_id: "task".into(),
                definition: TaskDefinition {
                    conversation_id: cid,
                    scope: TaskScope {
                        sms_enrollments: Vec::new(),
                        operations: vec![TaskPermission::Send],
                        recipients: vec![owner],
                    },
                    expires_at: now + 300,
                    initiating_event_id: None,
                },
            }),
        )
        .await
        .unwrap()
    {
        TaskOutcome::Record(record) => *record,
        _ => panic!("task"),
    };
    let binding = InvocationBinding {
        kind: InvocationKind::HostSession,
        adapter: "fixture".into(),
        external_id: "session-one".into(),
    };
    service
        .register_task_binding(key.id.clone(), task.id.clone(), binding.clone())
        .await
        .unwrap();
    let grant = service
        .issue_task_grant(
            key.id.clone(),
            "issue".into(),
            GrantDefinition {
                task_id: task.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::Send],
                expires_at: now + 120,
            },
        )
        .await
        .unwrap();
    let handle = service
        .authenticate_task_grant(grant.expose_secret().into(), binding)
        .await
        .unwrap();
    let metadata = service.task_invocation_grant(handle.clone()).await.unwrap();
    assert!(!serde_json::to_string(&metadata)
        .unwrap()
        .contains(grant.expose_secret()));
    let get = TaskRequest::new(TaskCommand::Get { task_id: task.id });
    assert!(service
        .execute_task_invocation(handle.clone(), get.clone())
        .await
        .is_ok());
    service
        .revoke_task_grant(key.id, metadata.id)
        .await
        .unwrap();
    assert!(service.task_invocation_grant(handle.clone()).await.is_err());
    assert!(service.execute_task_invocation(handle, get).await.is_err());
}

#[tokio::test]
async fn facade_projects_bounded_sources_and_revalidates_recall_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let owner = store.create_identity("reader").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let key = store.issue(&owner, Scope::operator(), now + 600).unwrap();
    let Outcome::Conversation(conversation) = store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "room".into(),
                participants: vec![],
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    for (id, body) in [
        ("old", "The durable decision is 42"),
        ("new", "Continue the work"),
    ] {
        store
            .execute(
                &key.id,
                &Operation::Send {
                    request_id: id.into(),
                    conversation_id: conversation.id.clone(),
                    body: body.into(),
                },
            )
            .unwrap();
    }
    let service = Service::new(store);
    let TaskOutcome::Record(task) = service
        .execute_task(
            key.id.clone(),
            TaskRequest::new(TaskCommand::Create {
                request_id: "task".into(),
                definition: TaskDefinition {
                    conversation_id: conversation.id,
                    scope: TaskScope {
                        sms_enrollments: Vec::new(),
                        operations: vec![TaskPermission::History],
                        recipients: vec![owner],
                    },
                    expires_at: now + 300,
                    initiating_event_id: None,
                },
            }),
        )
        .await
        .unwrap()
    else {
        panic!("task")
    };
    let binding = InvocationBinding {
        kind: InvocationKind::HostSession,
        adapter: "fixture".into(),
        external_id: "recall-session".into(),
    };
    service
        .register_task_binding(key.id.clone(), task.id.clone(), binding.clone())
        .await
        .unwrap();
    let grant = service
        .issue_task_grant(
            key.id.clone(),
            "grant".into(),
            GrantDefinition {
                task_id: task.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::History],
                expires_at: now + 120,
            },
        )
        .await
        .unwrap();
    let invocation = service
        .authenticate_task_grant(grant.expose_secret().into(), binding)
        .await
        .unwrap();
    let result = service
        .execute_task_invocation(
            invocation.clone(),
            TaskRequest::new(TaskCommand::Recall {
                task_id: task.id.clone(),
                after: None,
                limit: 1,
            }),
        )
        .await
        .unwrap();
    assert!(serde_json::to_vec(&result).unwrap().len() <= TASK_CONTEXT_BYTES);
    let TaskOutcome::Recall {
        sources,
        next,
        omitted,
    } = result
    else {
        panic!("recall")
    };
    assert_eq!(
        sources[0].body.as_deref(),
        Some("The durable decision is 42")
    );
    assert_eq!(omitted, 1);
    service
        .revoke_task_grant(key.id, grant.grant.id)
        .await
        .unwrap();
    assert_eq!(
        service
            .execute_task_invocation(
                invocation,
                TaskRequest::new(TaskCommand::Recall {
                    task_id: task.id.clone(),
                    after: next,
                    limit: 1,
                })
            )
            .await
            .unwrap_err()
            .code,
        "forbidden"
    );
}
