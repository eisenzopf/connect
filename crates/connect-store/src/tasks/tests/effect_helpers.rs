use super::{fixture::Fixture, grants};
use connect_protocol::{tasks::*, Operation, Outcome};

pub(super) fn send(
    f: &Fixture,
    task: &TaskRecord,
    id: &str,
    body: &str,
    wait: bool,
) -> TaskRequest {
    TaskRequest::new(TaskCommand::Invoke {
        request_id: id.into(),
        task_id: task.id.clone(),
        step_id: "step".into(),
        operation: Box::new(Operation::Send {
            request_id: id.into(),
            conversation_id: f.cid.clone(),
            body: body.into(),
        }),
        reply_to: None,
        wait_for_reply: wait,
    })
}
pub(super) fn count(f: &Fixture, table: &str) -> u64 {
    f.store
        .connection()
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
pub(super) fn effect(f: &Fixture, task: &TaskRecord) -> TaskEffect {
    let id: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM task_effects WHERE task_id=?1",
            [&task.id],
            |r| r.get(0),
        )
        .unwrap();
    let TaskOutcome::Effect(effect) = f
        .run(&TaskRequest::new(TaskCommand::EffectGet {
            task_id: task.id.clone(),
            effect_id: id,
        }))
        .unwrap()
    else {
        panic!("effect")
    };
    effect
}
pub(super) fn message(outcome: &TaskOutcome) -> String {
    let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = outcome else {
        panic!("completed")
    };
    let Outcome::Accepted { message_id, .. } = outcome.as_ref() else {
        panic!("message")
    };
    message_id.clone()
}
pub(super) fn handle(f: &Fixture, task: &TaskRecord) -> crate::TaskInvocation {
    let grant = grants::grant(f, task);
    f.store
        .authenticate_task_grant(grant.expose_secret(), &grant.grant.binding)
        .unwrap()
}
