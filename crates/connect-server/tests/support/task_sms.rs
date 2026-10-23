//! Trusted SMS enrollment setup; task creation and sends use public clients.
#![allow(clippy::unwrap_used)]
use super::native_tasks::Harness;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Telnyx;
use connect_protocol::{Operation, ProviderDelivery};
use connect_server::providers::Worker;
use connect_store::{Route, Scope, SmsGrant, SmsGrantSnapshot};
use std::{sync::Arc, time::Duration};

pub const PHONE: &str = "+15555550101";
pub const SENDER: &str = "+15555550102";
pub const OTHER_PHONE: &str = "+15555550103";
pub const PURPOSE: &str = "Review the requested Connect draft";

pub struct Enrollment {
    pub route: Route,
    grants: Vec<SmsGrant>,
    now: i64,
}
impl Enrollment {
    pub fn install(h: &Harness) -> Self {
        let now = chrono::Utc::now().timestamp();
        let delegate = h
            .server
            .store
            .issue(
                &h.peer_id,
                Scope::messaging(vec![h.owner_config.identity_id.clone()]),
                now + 3600,
            )
            .unwrap();
        let route = Route {
            id: "task-sms".into(),
            identity_id: h.peer_id.clone(),
            credential_id: delegate.id,
            conversation_id: h.cid.clone(),
            connector: "telnyx".into(),
            target: PHONE.into(),
            source: Some(SENDER.into()),
            enabled: true,
            incoming_to: connect_protocol::messages::MessageAudience::all(),
        };
        h.server
            .store
            .configure_route(&h.owner_config.credential_id, &route)
            .unwrap();
        let evidence = |id: &str, phone: &str, purpose: &str| SmsGrant {
            id: id.into(),
            phone: phone.into(),
            sender: SENDER.into(),
            conversation_id: h.cid.clone(),
            purpose: purpose.into(),
            consent_id: format!("form-{id}"),
            disclosure_version: "offline-v1".into(),
            disclosure_text: "Explicit consent for the requested offline test".into(),
            submitted_at: now - 10,
            approved_at: now - 5,
            expires_at: now + 3600,
        };
        let grants = vec![
            evidence("selected", PHONE, PURPOSE),
            evidence("other", PHONE, "A different reviewed task"),
            evidence(
                "changed-route",
                OTHER_PHONE,
                "A different reviewed destination",
            ),
        ];
        let enrollment = Self { route, grants, now };
        enrollment.refresh(h, 1, true);
        enrollment
    }
    pub fn reference(&self) -> TaskSmsEnrollment {
        TaskSmsEnrollment {
            source: "reviewed-fixture".into(),
            grant_id: "selected".into(),
            purpose: PURPOSE.into(),
        }
    }
    pub fn refresh(&self, h: &Harness, revision: i64, selected: bool) {
        h.server
            .store
            .refresh_sms_grants(
                "reviewed-fixture",
                &SmsGrantSnapshot {
                    revision,
                    grants: self
                        .grants
                        .iter()
                        .filter(|g| selected || g.id != "selected")
                        .cloned()
                        .collect(),
                },
            )
            .unwrap();
    }
    pub fn control(&self, h: &Harness, keyword: &str, order: i64) {
        h.server
            .store
            .receive_sms_control(
                &format!("control-{order}"),
                &format!("digest-{order}"),
                PHONE,
                SENDER,
                keyword,
                self.now + order,
            )
            .unwrap();
    }
    pub fn target(&self, h: &Harness, phone: &str) -> connect_protocol::Result<()> {
        let mut route = self.route.clone();
        route.target = phone.into();
        h.server
            .store
            .configure_route(&h.owner_config.credential_id, &route)
    }
    pub fn enabled(&self, h: &Harness, enabled: bool) {
        let mut route = self.route.clone();
        route.enabled = enabled;
        h.server
            .store
            .configure_route(&h.owner_config.credential_id, &route)
            .unwrap();
    }
}
pub fn question(task: &TaskRecord, cid: &str, id: &str) -> TaskCommand {
    TaskCommand::Invoke {
        request_id: id.into(),
        task_id: task.id.clone(),
        step_id: id.into(),
        operation: Box::new(Operation::Send {
            request_id: id.into(),
            conversation_id: cid.into(),
            message: connect_protocol::messages::MessageDraft::text_all(format!(
                "Requested draft review: {id}"
            )),
        }),
        reply_to: None,
        wait_for_reply: false,
    }
}
pub async fn queue(client: &mut ConnectClient, command: TaskCommand) {
    assert!(matches!(
        client.task(command).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Pending { .. })
    ));
}
pub async fn dispatch(h: &mut Harness, driver: Arc<Telnyx>, jobs: usize) -> Vec<ProviderDelivery> {
    let worker = Worker::with_drivers(h.server.service.clone(), vec![driver])
        .await
        .unwrap();
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let observed = h.owner.provider_status(&h.cid).await.unwrap();
            if observed.len() == jobs
                && observed
                    .iter()
                    .all(|j| !matches!(j.state.as_str(), "pending" | "sending"))
            {
                break observed;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    worker.shutdown().await.unwrap();
    observed
}
