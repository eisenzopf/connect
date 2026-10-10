//! A revoked task grant must cancel an actual unanswered loopback SIP INVITE.
#![allow(clippy::unwrap_used)]
mod gate3_support;
#[path = "support/outbound_sip.rs"]
mod outbound_sip;
use connect_protocol::{tasks::*, Operation, Outcome};
use outbound_sip::OutboundHarness;
use rvoip_sip::{Config, StreamPeer};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn task_cancel_after_invite_reaches_the_peer_and_preserves_conversation() {
    let mut h = OutboundHarness::start().await;
    let address = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut peer = StreamPeer::with_config(Config::local("task-peer", address.port()))
        .await
        .unwrap();
    let now = chrono::Utc::now().timestamp();
    let TaskOutcome::Record(task) = h
        .server
        .store
        .execute_task(
            &h.owner_credential_id,
            &TaskRequest::new(TaskCommand::Create {
                request_id: "task".into(),
                definition: TaskDefinition {
                    conversation_id: h.conversation_id.clone(),
                    scope: TaskScope {
                        operations: vec![TaskPermission::SipDial],
                        recipients: vec![h.participant_id.clone()],
                        sms_enrollments: Vec::new(),
                    },
                    expires_at: now + 600,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap()
    else {
        panic!("task")
    };
    let binding = InvocationBinding {
        kind: InvocationKind::Native,
        adapter: "fixture".into(),
        external_id: "sip-task-invocation".into(),
    };
    h.server
        .store
        .register_task_binding(&h.owner_credential_id, &task.id, &binding)
        .unwrap();
    let grant = h
        .server
        .store
        .issue_task_grant(
            &h.owner_credential_id,
            "grant",
            &GrantDefinition {
                task_id: task.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::SipDial],
                expires_at: now + 120,
            },
        )
        .unwrap();
    let invocation = h
        .server
        .store
        .authenticate_task_grant(grant.expose_secret(), &binding)
        .unwrap();
    let request = TaskRequest::new(TaskCommand::Invoke {
        request_id: "task-dial".into(),
        task_id: task.id.clone(),
        step_id: "call".into(),
        operation: Box::new(Operation::SipDial {
            request_id: "task-dial".into(),
            conversation_id: h.conversation_id.clone(),
            session_id: h.session_id.clone(),
            destination: format!("sip:peer@{address}"),
        }),
        reply_to: None,
        wait_for_reply: false,
    });
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect_id),
        ..
    }) = h
        .server
        .store
        .execute_task_invocation(&invocation, &request)
        .unwrap()
    else {
        panic!("pending")
    };
    let read = TaskRequest::new(TaskCommand::EffectGet {
        task_id: task.id.clone(),
        effect_id,
    });
    let TaskOutcome::Effect(effect) = h
        .server
        .store
        .execute_task(&h.owner_credential_id, &read)
        .unwrap()
    else {
        panic!("effect")
    };
    let Outcome::SipLeg(leg) = *effect.outcome.unwrap() else {
        panic!("leg")
    };
    let incoming = tokio::time::timeout(Duration::from_secs(5), peer.wait_for_incoming())
        .await
        .unwrap()
        .unwrap()
        .defer(Duration::from_secs(10));
    h.server
        .store
        .execute_task(
            &h.owner_credential_id,
            &TaskRequest::new(TaskCommand::Cancel {
                request_id: "cancel".into(),
                task_id: task.id.clone(),
                reason: None,
            }),
        )
        .unwrap();
    incoming
        .wait_for_cancelled(Some(Duration::from_secs(5)))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = h
                .client
                .sip_status(&h.conversation_id, &leg.id)
                .await
                .unwrap();
            if current.state == "cancelled" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        h.server
            .store
            .execute_task_invocation(&invocation, &request)
            .unwrap_err()
            .code,
        "forbidden"
    );
    // A call/task terminal state does not end the persistent conversation.
    assert!(h
        .client
        .send(
            "after-cancel",
            &h.conversation_id,
            "The conversation continues."
        )
        .await
        .is_ok());
    drop(incoming);
    peer.shutdown().await.unwrap();
    h.shutdown().await;
}
