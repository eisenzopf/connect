//! Reviewed purpose and current SMS policy at the public task/SDK boundary.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/task_sms.rs"]
mod task_sms;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Telnyx;
use native_tasks::{record, Harness};
use serde_json::json;
use std::sync::Arc;
use task_sms::{dispatch, question, queue, Enrollment, OTHER_PHONE, PHONE, SENDER};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test(flavor = "multi_thread")]
async fn reviewed_task_sms_rechecks_stop_route_purpose_lease_and_cancel_before_dispatch() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(body_partial_json(
            json!({"from":SENDER,"to":PHONE,"text":"Requested draft review: fresh"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"accepted-sms-one"}})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let driver = Arc::new(Telnyx::new("offline-fixture", &mock.uri()).unwrap());
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let enrollment = Enrollment::install(&h);
    let mut definition = h.definition();
    definition.scope.sms_enrollments = vec![enrollment.reference()];
    let task = record(
        h.owner
            .task(TaskCommand::Create {
                request_id: "reviewed-task".into(),
                definition,
            })
            .await
            .unwrap(),
    );
    let grant = grants::issue(
        &h,
        &task,
        "sms-grant",
        vec![TaskPermission::Send, TaskPermission::History],
        chrono::Utc::now().timestamp() + 300,
    )
    .await;
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let old = question(&task, &h.cid, "before-stop");
    queue(&mut client, old.clone()).await;
    enrollment.control(&h, "STOP", 1);
    let jobs = dispatch(&mut h, driver.clone(), 1).await;
    assert_eq!(jobs[0].state, "cancelled");
    assert_eq!(jobs[0].sms_decision.as_deref(), Some("opted-out"));
    // SMS suppression does not revoke history or end the task/conversation.
    assert!(matches!(
        client
            .task(TaskCommand::Recall {
                task_id: task.id.clone(),
                after: None,
                limit: 100
            })
            .await
            .unwrap(),
        TaskOutcome::Recall { .. }
    ));
    assert_eq!(
        record(
            h.owner
                .task(TaskCommand::Get {
                    task_id: task.id.clone()
                })
                .await
                .unwrap()
        )
        .state,
        TaskState::Running
    );
    enrollment.control(&h, "START", 2);
    assert!(client.task(old.clone()).await.is_err());
    // Queued work cannot be rebound to a different reviewed destination.
    queue(&mut client, question(&task, &h.cid, "disabled-route")).await;
    let before = connect_store::Store::inspect_snapshot(h.server.store.path()).unwrap();
    assert_eq!(
        enrollment.target(&h, OTHER_PHONE).unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(h.server.store.path()).unwrap(),
        before
    );
    enrollment.enabled(&h, false);
    let jobs = dispatch(&mut h, driver.clone(), 2).await;
    assert_eq!(jobs[0].state, "cancelled");
    assert!(jobs[0].sms_decision.is_none());
    enrollment.enabled(&h, true);
    // A second reviewed purpose on the same number cannot replace this one.
    enrollment.refresh(&h, 2, false);
    queue(&mut client, question(&task, &h.cid, "removed-purpose")).await;
    let jobs = dispatch(&mut h, driver.clone(), 3).await;
    assert_eq!(jobs[0].state, "cancelled");
    assert_eq!(
        jobs[0].sms_decision.as_deref(),
        Some("task-sms-enrollment-unavailable")
    );
    enrollment.refresh(&h, 3, true);
    queue(&mut client, question(&task, &h.cid, "stale-feed")).await;
    h.server.store.expire_sms_authorities().unwrap();
    let jobs = dispatch(&mut h, driver.clone(), 4).await;
    assert_eq!(jobs[0].state, "cancelled");
    assert_eq!(
        jobs[0].sms_decision.as_deref(),
        Some("task-sms-enrollment-unavailable")
    );
    enrollment.refresh(&h, 4, true);
    let fresh = question(&task, &h.cid, "fresh");
    queue(&mut client, fresh.clone()).await;
    let jobs = dispatch(&mut h, driver.clone(), 5).await;
    assert_eq!(jobs[0].state, "accepted");
    assert_eq!(
        jobs[0].sms_grant_id.as_deref(),
        Some("reviewed-fixture:selected")
    );
    assert!(matches!(
        client.task(fresh.clone()).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Pending { .. })
    ));
    // API acceptance alone is not delivery. Trusted callback ingestion supplies
    // the observed final status; its duplicate cannot create another send.
    assert!(h
        .server
        .store
        .delivery_sms(
            "delivered",
            "delivery-digest",
            "accepted-sms-one",
            "delivered"
        )
        .unwrap());
    assert!(!h
        .server
        .store
        .delivery_sms(
            "delivered",
            "delivery-digest",
            "accepted-sms-one",
            "delivered"
        )
        .unwrap());
    assert!(matches!(
        client.task(fresh).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    assert!(client.task(old).await.is_err());
    queue(&mut client, question(&task, &h.cid, "queued-before-cancel")).await;
    h.owner
        .task(TaskCommand::Cancel {
            request_id: "cancel".into(),
            task_id: task.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    // START changes suppression only; it cannot restore a cancelled delegation.
    enrollment.control(&h, "START", 3);
    assert_eq!(
        grants::code(
            client
                .task(question(&task, &h.cid, "after-cancel"))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    let jobs = dispatch(&mut h, driver, 6).await;
    assert_eq!(jobs[0].state, "cancelled");
    assert_eq!(jobs.iter().filter(|j| j.state == "delivered").count(), 1);
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    client.close().await;
    h.close().await;
}
