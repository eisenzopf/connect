use super::{fixture::Fixture, replies};
use crate::{Route, SmsGrant, SmsGrantSnapshot};
use connect_protocol::tasks::*;

pub(super) const PHONE: &str = "+15555550101";
pub(super) const SENDER: &str = "+15555550102";
pub(super) const PURPOSE: &str = "Review the requested draft";

pub(super) fn setup() -> (Fixture, String) {
    let (f, human, identity) = replies::setup();
    f.store
        .configure_route(
            &f.key.id,
            &Route {
                id: "sms".into(),
                identity_id: identity.clone(),
                credential_id: human.id,
                conversation_id: f.cid.clone(),
                connector: "telnyx".into(),
                target: PHONE.into(),
                source: Some(SENDER.into()),
                enabled: true,
            },
        )
        .unwrap();
    refresh(&f, 1, true);
    (f, identity)
}
pub(super) fn refresh(f: &Fixture, revision: i64, selected: bool) {
    let grant = |id: &str, purpose: &str| SmsGrant {
        id: id.into(),
        phone: PHONE.into(),
        sender: SENDER.into(),
        conversation_id: f.cid.clone(),
        purpose: purpose.into(),
        consent_id: "submitted-form".into(),
        disclosure_version: "fixture-v1".into(),
        disclosure_text: "Explicit fixture consent".into(),
        submitted_at: f.now - 10,
        approved_at: f.now - 5,
        expires_at: f.now + 3600,
    };
    let mut grants = vec![grant("other", "A different reviewed request")];
    if selected {
        grants.push(grant("selected", PURPOSE));
    }
    f.store
        .refresh_sms_grants("reviewed", &SmsGrantSnapshot { revision, grants })
        .unwrap();
}
pub(super) fn reference() -> TaskSmsEnrollment {
    TaskSmsEnrollment {
        source: "reviewed".into(),
        grant_id: "selected".into(),
        purpose: PURPOSE.into(),
    }
}
pub(super) fn create(
    f: &Fixture,
    human: &str,
    id: &str,
    references: Vec<TaskSmsEnrollment>,
) -> TaskRequest {
    TaskRequest::new(TaskCommand::Create {
        request_id: id.into(),
        definition: TaskDefinition {
            conversation_id: f.cid.clone(),
            scope: TaskScope {
                operations: vec![TaskPermission::Send, TaskPermission::History],
                recipients: vec![human.into()],
                sms_enrollments: references,
            },
            expires_at: f.now + 600,
            initiating_event_id: None,
        },
    })
}
pub(super) fn decision(f: &Fixture) -> (String, Option<String>, Option<String>) {
    f.store.connection().unwrap().query_row("SELECT state,sms_decision,sms_grant_id FROM provider_jobs ORDER BY sequence DESC LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
