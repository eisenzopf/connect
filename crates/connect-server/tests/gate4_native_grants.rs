//! Signed task principals over actual QUIC/HTTP, with no broad owner fallback.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::*, ConnectClient};
use connect_protocol::{Operation, Outcome};
use grants::{code, wrapped};
use native_tasks::Harness;
use rvoip_auth_core::BearerValidator;
use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};

#[tokio::test]
async fn narrow_native_grant_negotiates_exact_task_and_enforces_current_scope_on_replay() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let a = grants::task(&mut h, "task-a").await;
    let b = grants::task(&mut h, "task-b").await;
    let grant = grants::issue(
        &h,
        &a,
        "read-only",
        vec![TaskPermission::History],
        chrono::Utc::now().timestamp() + 180,
    )
    .await;
    let config = h.owner_config.clone().with_task_grant(&grant).unwrap();
    assert_ne!(config.credential, h.owner_config.credential);
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    assert_eq!(
        code(
            client
                .task(TaskCommand::Get {
                    task_id: a.id.clone()
                })
                .await
                .unwrap_err()
        ),
        "unsupported-profile"
    );
    assert_eq!(
        code(
            client
                .negotiate_tasks("connect:agent@local", Some(&h.cid))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(client.negotiate_task(&b.id).await.unwrap_err()),
        "forbidden"
    );
    assert_eq!(client.negotiate_task(&a.id).await.unwrap().id, a.id);
    assert_eq!(
        code(
            client
                .task(TaskCommand::Get {
                    task_id: b.id.clone()
                })
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(
            client
                .send("primitive", &h.cid, "forbidden")
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    let read = wrapped(
        &a.id,
        "read",
        Operation::History {
            conversation_id: h.cid.clone(),
            after: None,
            limit: 10,
        },
    );
    let result = client.task(read.clone()).await.unwrap();
    assert_eq!(client.task(read.clone()).await.unwrap(), result);
    assert!(matches!(
        result,
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    let wider = wrapped(
        &a.id,
        "wider",
        Operation::Send {
            request_id: "wider".into(),
            conversation_id: h.cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all("must not send"),
        },
    );
    assert_eq!(code(client.task(wider).await.unwrap_err()), "forbidden");
    let principal = connect_service::StoreValidator::new(h.server.store.clone())
        .validate_principal(grant.expose_secret())
        .await
        .unwrap();
    assert_eq!(
        principal.scopes,
        vec![connect_protocol::REQUIRED_SCOPE.to_string()]
    );
    assert!(connect_service::StoreValidator::for_credential(
        h.server.store.clone(),
        h.owner_config.credential_id.clone()
    )
    .validate_principal(grant.expose_secret())
    .await
    .is_err());
    let mut raw = support::Peer::quic(&config).await;
    assert!(raw.authenticate(grant.expose_secret()).await);
    let denied = raw
        .exchange(
            UctpEnvelope::new(
                MessageType::SessionInvite,
                serde_json::json!({
                    "from": h.owner_config.identity_id,
                    "to": ["connect:agent@local"],
                    "medium": "voice",
                    "intent": "connect-session",
                    "capabilities_offer": {"audio": ["g.711-mu"]}
                }),
            )
            .with_cid(&h.cid)
            .with_sid("grant-must-not-open-media"),
        )
        .await;
    assert_eq!(denied.msg_type, MessageType::Error);
    assert_eq!(denied.payload["code"], 403);
    assert_eq!(denied.payload["reason"], "insufficient-scope");
    raw.close().await;
    let get = Operation::Task {
        request: Box::new(TaskRequest::new(TaskCommand::Get {
            task_id: a.id.clone(),
        })),
    };
    assert!(matches!(
        grants::operation(&h, grant.expose_secret(), get.clone())
            .await
            .result,
        Ok(Outcome::Task(_))
    ));
    let response = grants::http()
        .post(format!("{}/v1/operations", grants::url(&h)))
        .bearer_auth(grant.expose_secret())
        .header("x-call-id", "spoof")
        .json(&get)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    h.server
        .service
        .revoke_task_grant(h.owner_config.credential_id.clone(), grant.grant.id.clone())
        .await
        .unwrap();
    assert_eq!(code(client.task(read).await.unwrap_err()), "forbidden");
    assert!(grants::operation(&h, grant.expose_secret(), get)
        .await
        .result
        .is_err());
    client.close().await;
    assert!(h
        .peer
        .history(&h.cid, None, 10)
        .await
        .unwrap()
        .messages
        .is_empty());
    h.close().await;
}

#[tokio::test]
async fn native_entry_rejects_host_provider_corrupt_and_wrong_subject_tokens() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = grants::task(&mut h, "origins").await;
    for (kind, id) in [
        (InvocationKind::HostSession, "host"),
        (InvocationKind::ProviderCall, "call"),
        (InvocationKind::ProviderChat, "chat"),
    ] {
        let binding = InvocationBinding {
            kind,
            adapter: "verified-test-composition".into(),
            external_id: id.into(),
        };
        h.server
            .service
            .register_task_binding(
                h.owner_config.credential_id.clone(),
                task.id.clone(),
                binding.clone(),
            )
            .await
            .unwrap();
        let grant = h
            .server
            .service
            .issue_task_grant(
                h.owner_config.credential_id.clone(),
                format!("issue-{id}"),
                GrantDefinition {
                    task_id: task.id.clone(),
                    binding,
                    operations: vec![TaskPermission::History],
                    expires_at: chrono::Utc::now().timestamp() + 180,
                },
            )
            .await
            .unwrap();
        let mut raw = support::Peer::quic(&h.owner_config).await;
        assert!(!raw.authenticate(grant.expose_secret()).await);
        raw.close().await;
        let result = grants::operation(
            &h,
            grant.expose_secret(),
            Operation::Task {
                request: Box::new(TaskRequest::new(TaskCommand::Get {
                    task_id: task.id.clone(),
                })),
            },
        )
        .await;
        assert_eq!(result.result.unwrap_err().code, "unauthorized");
    }
    let grant = grants::issue(
        &h,
        &task,
        "native",
        vec![TaskPermission::History],
        chrono::Utc::now().timestamp() + 180,
    )
    .await;
    let mut wrong = h.owner_config.clone().with_task_grant(&grant).unwrap();
    wrong.credential = h.owner_config.credential.clone();
    assert!(ConnectClient::open(wrong).await.is_err());
    let mut corrupt = grant.expose_secret().to_owned();
    corrupt.push('0');
    let mut raw = support::Peer::quic(&h.owner_config).await;
    assert!(!raw.authenticate(&corrupt).await);
    raw.close().await;
    h.close().await;
}
