//! Operational composition cannot turn absent or stale workers into readiness.
#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::{tasks::TaskState, Readiness, ReadinessState as R};
fn sample() -> TaskDiagnostics {
    TaskDiagnostics {
        task: DiagnosticTask {
            task_id: "t".into(),
            conversation_id: "c".into(),
            state: TaskState::Waiting,
            revision: 1,
            expires_at: 100,
            updated_at: 1,
        },
        changed_during_sample: false,
        journal: DiagnosticSection::Unavailable {
            code: "scope-denied".into(),
        },
        sources: DiagnosticSection::Unavailable {
            code: "scope-denied".into(),
        },
        unassigned: DiagnosticSection::Unavailable {
            code: "scope-denied".into(),
        },
        server_readiness: DiagnosticSection::Available(Readiness {
            media: None,
            state: R::Ready,
            components: vec![],
        }),
    }
}
#[test]
fn ready_server_never_masks_stopped_unknown_or_unconfigured_execution() {
    let task = sample();
    for condition in [
        WorkerCondition::Stopped,
        WorkerCondition::RecoveryRequired,
        WorkerCondition::InvalidMetadata,
    ] {
        let result = task.execution_readiness(&[condition.component("worker")]);
        assert_eq!(result.state, R::Unavailable);
        assert_eq!(result.components[0].state, R::Ready);
    }
    for condition in [
        WorkerCondition::Unconfigured,
        WorkerCondition::OwnershipObserved,
        WorkerCondition::ChangedDuringSample,
    ] {
        assert_eq!(
            task.execution_readiness(&[condition.component("worker")])
                .state,
            R::Uncertain
        );
    }
    assert_eq!(task.execution_readiness(&[]).state, R::Uncertain);
    let absent = WorkerCondition::Unconfigured.component("worker");
    assert!(!absent.configured);
    assert_eq!(absent.reason, "not-configured");
    assert_eq!(
        task.execution_readiness(&[WorkerCondition::ExplicitResumeRequired.component("host")])
            .state,
        R::Degraded
    );
}
#[test]
fn server_failure_and_sample_changes_remain_explicit_without_mutating_original_views() {
    let mut task = sample();
    task.changed_during_sample = true;
    let before = serde_json::to_vec(&task).unwrap();
    let result =
        task.execution_readiness(&[WorkerCondition::OwnershipObserved.component("worker")]);
    assert!(result
        .components
        .iter()
        .any(|c| c.reason == "task-changed-during-sample"));
    assert_eq!(serde_json::to_vec(&task).unwrap(), before);
    task.server_readiness = DiagnosticSection::Available(Readiness {
        media: None,
        state: R::Unavailable,
        components: vec![],
    });
    assert_eq!(
        task.execution_readiness(&[WorkerCondition::OwnershipObserved.component("worker")])
            .state,
        R::Unavailable
    );
    task.server_readiness = DiagnosticSection::Unavailable {
        code: "PRIVATE-REMOTE-DETAIL".into(),
    };
    let result =
        task.execution_readiness(&[WorkerCondition::OwnershipObserved.component("worker")]);
    assert_eq!(result.state, R::Uncertain);
    assert!(!serde_json::to_string(&result).unwrap().contains("PRIVATE"));
}
