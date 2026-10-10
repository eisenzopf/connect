use super::{effect_helpers::send, fixture::Fixture};
use connect_protocol::tasks::*;

fn request(
    task: &TaskRecord,
    id: &str,
    step: &str,
    worker: &str,
    revision: Option<u64>,
) -> TaskRequest {
    TaskRequest::new(TaskCommand::Claim {
        request_id: id.into(),
        task_id: task.id.clone(),
        step_id: step.into(),
        worker_id: worker.into(),
        expected_revision: revision,
    })
}
fn claim(outcome: TaskOutcome) -> TaskStepClaim {
    let TaskOutcome::Claim(Some(claim)) = outcome else {
        panic!("claim")
    };
    claim
}
#[test]
fn renew_and_reclaim_require_current_revision_and_replay_does_not_extend_lifetime() {
    let f = Fixture::new();
    let task = f.task();
    let initial = request(&task, "claim", "step", "one", None);
    let first = claim(f.run(&initial).unwrap());
    assert_eq!(first.expires_at, f.now + TASK_CLAIM_SECONDS);
    assert_eq!(
        claim(
            f.store
                .execute_task_at(&f.key.id, &initial, f.now + 10)
                .unwrap()
        ),
        first
    );
    assert_eq!(
        f.run(&request(
            &task,
            "intruder",
            "step",
            "two",
            Some(first.revision)
        ))
        .unwrap_err()
        .code,
        "task-claim-conflict"
    );
    let renewed = claim(
        f.store
            .execute_task_at(
                &f.key.id,
                &request(&task, "renew", "step", "one", Some(first.revision)),
                f.now + 10,
            )
            .unwrap(),
    );
    assert_eq!(renewed.revision, 1);
    assert_eq!(renewed.expires_at, f.now + 40);
    let next = claim(
        f.store
            .execute_task_at(
                &f.key.id,
                &request(&task, "reclaim", "step", "two", Some(renewed.revision)),
                f.now + 40,
            )
            .unwrap(),
    );
    assert_eq!(next.worker_id, "two");
    assert_eq!(next.revision, 2);
    assert_eq!(
        f.store
            .execute_task_at(
                &f.key.id,
                &request(&task, "stale", "step", "one", Some(first.revision)),
                f.now + 41
            )
            .unwrap_err()
            .code,
        "task-claim-conflict"
    );
}
#[test]
fn global_sixteen_step_limit_allows_renewal_but_rejects_more_active_leases() {
    let f = Fixture::new();
    let task = f.task();
    for i in 0..MAX_RUNNING_TASKS {
        f.run(&request(
            &task,
            &format!("claim-{i}"),
            &format!("step-{i}"),
            "worker",
            None,
        ))
        .unwrap();
    }
    assert_eq!(
        f.run(&request(&task, "too-many", "excess", "worker", None))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    let current = claim(
        f.run(&request(&task, "renew", "step-0", "worker", Some(0)))
            .unwrap(),
    );
    assert_eq!(current.revision, 1);
    let more = claim(
        f.store
            .execute_task_at(
                &f.key.id,
                &request(&task, "later", "excess", "worker", None),
                f.now + TASK_CLAIM_SECONDS,
            )
            .unwrap(),
    );
    assert_eq!(more.step_id, "excess");
}
#[test]
fn lease_expiry_cannot_reclaim_a_step_that_has_a_durable_effect() {
    let f = Fixture::new();
    let task = f.task();
    let first = claim(
        f.run(&request(&task, "claim", "step", "one", None))
            .unwrap(),
    );
    f.run(&send(&f, &task, "mutation", "one", false)).unwrap();
    assert_eq!(
        f.store
            .execute_task_at(
                &f.key.id,
                &request(&task, "reclaim", "step", "two", Some(first.revision)),
                f.now + TASK_CLAIM_SECONDS
            )
            .unwrap_err()
            .code,
        "effect-reconciliation-required"
    );
    let release = TaskRequest::new(TaskCommand::ReleaseClaim {
        request_id: "release".into(),
        task_id: task.id.clone(),
        step_id: "step".into(),
        worker_id: "one".into(),
        expected_revision: first.revision,
    });
    let released = claim(f.run(&release).unwrap());
    assert_eq!(released.expires_at, f.now);
    assert_eq!(released.revision, 1);
    assert_eq!(f.run(&release).unwrap(), TaskOutcome::Claim(Some(released)));
}
#[test]
fn two_competing_connections_cannot_both_claim_a_fresh_step() {
    let f = Fixture::new();
    let task = f.task();
    let a = request(&task, "a", "step", "one", None);
    let b = request(&task, "b", "step", "two", None);
    let other = crate::Store::open(f.store.path(), connect_protocol::Limits::default()).unwrap();
    let key = f.key.id.clone();
    let now = f.now;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let start = barrier.clone();
    let worker = std::thread::spawn(move || {
        start.wait();
        other.execute_task_at(&key, &a, now)
    });
    barrier.wait();
    let mine = f.run(&b);
    let theirs = worker.join().unwrap();
    assert_ne!(mine.is_ok(), theirs.is_ok());
    assert_eq!(
        mine.err().or_else(|| theirs.err()).unwrap().code,
        "task-claim-conflict"
    );
}
#[test]
fn cancelled_task_and_narrow_invocation_cannot_acquire_worker_authority() {
    let f = Fixture::new();
    let task = f.task();
    let handle = super::effect_helpers::handle(&f, &task);
    let claim = request(&task, "claim", "step", "worker", None);
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &claim)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id,
        reason: None,
    }))
    .unwrap();
    assert_eq!(f.run(&claim).unwrap_err().code, "forbidden");
}
