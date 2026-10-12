//! Change actual protected state between reads at the lock-probe boundary.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::cli::{
    init::private_write,
    task_access::{codex, GrantFile},
};
use connect_protocol::tasks::*;

#[test]
fn concurrent_state_change_never_combines_session_metadata_or_retains_probe_lock() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("host");
    let definition = GrantDefinition {
        task_id: "task-id".into(),
        binding: InvocationBinding {
            kind: InvocationKind::Native,
            adapter: "diagnostic-test".into(),
            external_id: "fixture".into(),
        },
        operations: vec![TaskPermission::History],
        expires_at: chrono::Utc::now().timestamp() + 120,
    };
    let grant = ProtectedTaskGrant::new(
        InvocationGrant {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: definition.task_id.clone(),
            binding: definition.binding.clone(),
            operations: definition.operations.clone(),
            expires_at: definition.expires_at,
            revoked: false,
        },
        "unused-local-fixture-secret".into(),
    );
    let file = GrantFile {
        profile: TASK_ACCESS_PROFILE.into(),
        request: TaskAccessRequest::new(TaskAccessCommand::Issue {
            request_id: "fixture".into(),
            definition,
        }),
        grant: Some(grant),
    };
    let input = directory.path().join("grant.json");
    private_write(&input, &serde_json::to_vec(&file).unwrap()).unwrap();
    codex::export(&input, "127.0.0.1:12345".parse().unwrap(), &output).unwrap();
    let installation = install::load(&output).unwrap();
    let initial = state::State {
        profile: "connect-codex-host/1".into(),
        installation: installation.digest,
        task_id: "task-id".into(),
        grant_id: file.grant.as_ref().unwrap().grant.id.clone(),
        session_id: Some(uuid::Uuid::new_v4().to_string()),
        run_id: uuid::Uuid::new_v4().to_string(),
        prompt: "PRIVATE-PROMPT".into(),
        completed: false,
        attempts: 1,
    };
    initial.create(&output).unwrap();
    private_write(&output.join("host.lock"), b"").unwrap();
    let observation = inspect_with_lock_probe(Some(&output), "task-id", |path| {
        let held = lock(path)?;
        let mut changed = state::State::load(&output)?.unwrap();
        changed.completed = true;
        changed.attempts += 1;
        changed.save(&output)?;
        Ok(held)
    });
    assert_eq!(observation.state, HostState::ChangedDuringSample);
    assert!(
        observation.session_id.is_none()
            && observation.run_id.is_none()
            && observation.attempts.is_none()
    );
    let readiness = observation.readiness();
    assert_eq!(readiness.state, connect_protocol::ReadinessState::Uncertain);
    assert_eq!(readiness.reason, "worker-changed-during-sample");
    let encoded = serde_json::to_string(&observation).unwrap();
    assert!(
        !encoded.contains("PRIVATE") && !encoded.contains(initial.session_id.as_ref().unwrap())
    );
    let handle = std::fs::OpenOptions::new()
        .write(true)
        .open(output.join("host.lock"))
        .unwrap();
    handle.try_lock_exclusive().unwrap();
    FileExt::unlock(&handle).unwrap();
    assert_eq!(
        inspect(Some(&output), "task-id").state,
        HostState::TurnComplete
    );
}
