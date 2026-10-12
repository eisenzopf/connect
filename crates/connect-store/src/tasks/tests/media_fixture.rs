use super::{fixture::Fixture, replies};
use crate::{Scope, VoiceRoute};
use connect_protocol::{tasks::*, Operation, Outcome};

pub(super) fn task(f: &Fixture, recipient: &str, id: &str) -> TaskRecord {
    let mut request = f.create();
    let TaskCommand::Create {
        request_id,
        definition,
    } = &mut request.command
    else {
        panic!("create")
    };
    *request_id = id.into();
    definition.scope.recipients = vec![recipient.into()];
    definition.scope.operations = vec![
        TaskPermission::CallStart,
        TaskPermission::CallEnd,
        TaskPermission::SipDial,
        TaskPermission::SipEnd,
        TaskPermission::History,
    ];
    Fixture::record(f.run(&request).unwrap())
}
pub(super) fn invoke(t: &TaskRecord, id: &str, operation: Operation) -> TaskRequest {
    TaskRequest::new(TaskCommand::Invoke {
        request_id: id.into(),
        task_id: t.id.clone(),
        step_id: "media".into(),
        operation: Box::new(operation),
        reply_to: None,
        wait_for_reply: false,
    })
}
pub(super) fn call_start(f: &Fixture, id: &str) -> Operation {
    Operation::CallStart {
        request_id: id.into(),
        conversation_id: f.cid.clone(),
        route_id: "voice".into(),
        destination: "+15555550101".into(),
    }
}
pub(super) fn call_end(f: &Fixture, id: &str, call: &str) -> Operation {
    Operation::CallEnd {
        request_id: id.into(),
        conversation_id: f.cid.clone(),
        call_id: call.into(),
    }
}
pub(super) fn call_setup() -> (Fixture, TaskRecord) {
    let f = Fixture::new();
    f.store
        .configure_voice_route(
            &f.key.id,
            &VoiceRoute {
                id: "voice".into(),
                conversation_id: f.cid.clone(),
                identity_id: f.owner.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    let task = task(&f, &f.owner, "task");
    (f, task)
}
pub(super) fn active_call(f: &Fixture) -> String {
    let Outcome::Call(call) = f.store.execute(&f.key.id, &call_start(f, "start")).unwrap() else {
        panic!("call")
    };
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
    f.store
        .finish_voice_job(
            &job,
            "in-progress",
            Some("provider-call"),
            Some("https://example.test/end"),
            None,
        )
        .unwrap();
    call.id
}
pub(super) fn sip_setup() -> (Fixture, TaskRecord, String) {
    let (f, _, human) = replies::setup();
    let key = f
        .store
        .issue(
            &human,
            Scope::communications(vec![f.owner.clone()]),
            f.now + 3600,
        )
        .unwrap();
    let Outcome::Session(session) = f
        .store
        .execute(
            &f.key.id,
            &Operation::SessionInvite {
                request_id: "invite".into(),
                conversation_id: f.cid.clone(),
                participant_id: human.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!("session")
    };
    f.store
        .execute(
            &key.id,
            &Operation::SessionAccept {
                request_id: "accept".into(),
                conversation_id: f.cid.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    let task = task(&f, &human, "task");
    (f, task, session.id)
}
pub(super) fn sip_start(f: &Fixture, id: &str, session: &str) -> Operation {
    Operation::SipDial {
        request_id: id.into(),
        conversation_id: f.cid.clone(),
        session_id: session.into(),
        destination: "sip:peer@127.0.0.1:5099".into(),
    }
}
pub(super) fn sip_end(f: &Fixture, id: &str, leg: &str) -> Operation {
    Operation::SipEnd {
        request_id: id.into(),
        conversation_id: f.cid.clone(),
        sip_leg_id: leg.into(),
    }
}
pub(super) fn active_sip(f: &Fixture, session: &str) -> String {
    let Outcome::SipLeg(leg) = f
        .store
        .execute(&f.key.id, &sip_start(f, "start", session))
        .unwrap()
    else {
        panic!("leg")
    };
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    f.store.prepare_sip_leg(&job, "connection").unwrap();
    assert!(f.store.sip_dispatch_allowed(&job).unwrap());
    f.store.finish_sip_leg_job(&job, "active", None).unwrap();
    leg.id
}
pub(super) fn cancel(f: &Fixture, task: &TaskRecord) {
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
}
