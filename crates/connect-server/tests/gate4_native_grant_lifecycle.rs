//! Reconnect/refresh does not bypass terminal, parent or elapsed grant authority.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
use connect_client::{tasks::*, ConnectClient};
use connect_protocol::Operation;
use native_tasks::Harness;

async fn ready() -> (Harness, TaskRecord) {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = grants::task(&mut h, "lifecycle").await;
    (h, task)
}
#[tokio::test]
async fn fresh_grant_preserves_one_logical_send_and_cancel_denies_current_peers() {
    let (mut h, task) = ready().await;
    let expiry = chrono::Utc::now().timestamp() + 180;
    let grant = grants::issue(
        &h,
        &task,
        "first",
        vec![TaskPermission::History, TaskPermission::Send],
        expiry,
    )
    .await;
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let send = grants::wrapped(
        &task.id,
        "once",
        Operation::Send {
            request_id: "once".into(),
            conversation_id: h.cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all("One logical question"),
        },
    );
    let first = client.task(send.clone()).await.unwrap();
    assert_eq!(client.task(send.clone()).await.unwrap(), first);
    let mut changed = send.clone();
    if let TaskCommand::Invoke { operation, .. } = &mut changed {
        if let Operation::Send { message, .. } = &mut **operation {
            message.body = "Changed intent".into();
        }
    }
    assert_eq!(
        grants::code(client.task(changed).await.unwrap_err()),
        "request-conflict"
    );
    let fresh = grants::issue(
        &h,
        &task,
        "fresh",
        vec![TaskPermission::History, TaskPermission::Send],
        expiry,
    )
    .await;
    let mut next = ConnectClient::open(h.owner_config.clone().with_task_grant(&fresh).unwrap())
        .await
        .unwrap();
    next.negotiate_task(&task.id).await.unwrap();
    h.server
        .service
        .revoke_task_grant(h.owner_config.credential_id.clone(), grant.grant.id.clone())
        .await
        .unwrap();
    assert_eq!(next.task(send.clone()).await.unwrap(), first);
    assert_eq!(
        h.peer
            .history(&h.cid, None, 10)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    h.owner
        .task(TaskCommand::Cancel {
            request_id: "cancel".into(),
            task_id: task.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    assert!(client.task(send.clone()).await.is_err());
    assert!(next.task(send).await.is_err());
    assert!(client
        .task(TaskCommand::Get { task_id: task.id })
        .await
        .is_err());
    assert_eq!(
        h.peer
            .history(&h.cid, None, 10)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    client.close().await;
    next.close().await;
    h.close().await;
}
#[tokio::test]
async fn parent_revocation_is_checked_after_native_authentication() {
    let (h, task) = ready().await;
    let grant = grants::issue(
        &h,
        &task,
        "parent",
        vec![TaskPermission::History],
        chrono::Utc::now().timestamp() + 180,
    )
    .await;
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let actor = h
        .server
        .store
        .revalidate(&h.owner_config.credential_id)
        .unwrap();
    h.server
        .store
        .revoke(&actor, &h.owner_config.credential_id)
        .unwrap();
    assert!(client
        .task(TaskCommand::Get {
            task_id: task.id.clone()
        })
        .await
        .is_err());
    assert!(grants::operation(
        &h,
        grant.expose_secret(),
        Operation::Task {
            request: Box::new(TaskRequest::new(TaskCommand::Get { task_id: task.id }))
        }
    )
    .await
    .result
    .is_err());
    client.close().await;
    h.close().await;
}
#[tokio::test]
async fn elapsed_grant_denies_an_existing_peer_and_an_accepted_read_replay() {
    let (h, task) = ready().await;
    let expiry = chrono::Utc::now().timestamp() + 4;
    let grant = grants::issue(&h, &task, "short", vec![TaskPermission::History], expiry).await;
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let read = grants::wrapped(
        &task.id,
        "short-read",
        Operation::History {
            conversation_id: h.cid.clone(),
            after: None,
            limit: 10,
        },
    );
    client.task(read.clone()).await.unwrap();
    let remaining = expiry - chrono::Utc::now().timestamp();
    tokio::time::sleep(std::time::Duration::from_secs(
        u64::try_from(remaining.max(0)).unwrap() + 1,
    ))
    .await;
    assert!(client.task(read).await.is_err());
    assert!(grants::operation(
        &h,
        grant.expose_secret(),
        Operation::Task {
            request: Box::new(TaskRequest::new(TaskCommand::Get { task_id: task.id }))
        }
    )
    .await
    .result
    .is_err());
    client.close().await;
    h.close().await;
}
