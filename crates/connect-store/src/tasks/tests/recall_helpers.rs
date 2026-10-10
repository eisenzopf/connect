use super::{fixture::Fixture, grants};
use crate::{IssuedTaskGrant, TaskInvocation};
use connect_protocol::{tasks::*, Operation, Outcome};

pub(super) fn request(task: &TaskRecord, after: Option<String>, limit: u32) -> TaskRequest {
    TaskRequest::new(TaskCommand::Recall {
        task_id: task.id.clone(),
        after,
        limit,
    })
}
pub(super) fn page(outcome: TaskOutcome) -> (Vec<TaskSource>, Option<String>, u64) {
    assert!(serde_json::to_vec(&outcome).unwrap().len() <= TASK_CONTEXT_BYTES);
    let TaskOutcome::Recall {
        sources,
        next,
        omitted,
    } = outcome
    else {
        panic!("recall")
    };
    (sources, next, omitted)
}
pub(super) fn send(f: &Fixture, cid: &str, id: &str, body: &str) -> String {
    let Outcome::Accepted { message_id, .. } = f
        .store
        .execute(
            &f.key.id,
            &Operation::Send {
                request_id: id.into(),
                conversation_id: cid.into(),
                body: body.into(),
            },
        )
        .unwrap()
    else {
        panic!("message")
    };
    message_id
}
pub(super) fn grant(f: &Fixture, task: &TaskRecord) -> (IssuedTaskGrant, TaskInvocation) {
    let binding = grants::binding("recall", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &task.id, &binding)
        .unwrap();
    let issued = f
        .store
        .issue_task_grant(
            &f.key.id,
            "recall-grant",
            &GrantDefinition {
                task_id: task.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::History],
                expires_at: f.now + 120,
            },
        )
        .unwrap();
    let invocation = f
        .store
        .authenticate_task_grant(issued.expose_secret(), &binding)
        .unwrap();
    (issued, invocation)
}
