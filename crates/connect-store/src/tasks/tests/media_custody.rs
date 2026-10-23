//! Current physical authority and captured results are separate lifetimes.
use super::{fixture::Fixture, media_fixture::*};
use crate::{Limits, Store};
use connect_connector_api::{lifecycle::*, RemoteCall, VoiceReceipt};
use std::{sync::mpsc, time::Duration};
fn receipt(id: &str) -> VoiceReceipt {
    VoiceReceipt {
        call: RemoteCall {
            id: "captured-remote-call".into(),
            state: "in-progress".into(),
            control_url: Some("https://fixture.invalid/control".into()),
            ended_reason: None,
        },
        observation: Observation {
            mutation_id: id.into(),
            external_id: Some("captured-remote-call".into()),
            event_id: "captured-response".into(),
            state: State::Voice(VoiceState::Active),
            verification: Verification::AuthenticatedResponse,
        },
    }
}
fn state(f: &Fixture) -> (String, Option<String>) {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state,provider_id FROM voice_calls LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}
#[test]
fn retained_voice_transport_blocks_revoke_then_captured_result_records_once_after_withdrawal() {
    let (f, _) = call_setup();
    f.store.execute(&f.key.id, &call_start(&f, "dial")).unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let dispatch = f.store.begin_voice_dispatch(&job).unwrap().unwrap();
    let other = Store::open(f.store.path(), Limits::default()).unwrap();
    let actor = f.store.revalidate(&f.key.id).unwrap();
    let key = f.key.id.clone();
    let (sent, received) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        other.revoke(&actor, &key).unwrap();
        sent.send(()).unwrap();
    });
    assert!(received.recv_timeout(Duration::from_millis(150)).is_err());
    let captured = dispatch.capture(Ok(receipt(&job.call.id)));
    received.recv_timeout(Duration::from_secs(3)).unwrap();
    writer.join().unwrap();
    assert!(f.store.revalidate(&f.key.id).is_err());
    f.store.record_voice_dispatch(&captured).unwrap();
    let snapshot = Store::inspect_snapshot(f.store.path()).unwrap();
    f.store.record_voice_dispatch(&captured).unwrap();
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), snapshot);
    assert_eq!(
        state(&f),
        ("in-progress".into(), Some("captured-remote-call".into()))
    );
    assert!(f.store.begin_voice_dispatch(&job).unwrap().is_none());
}
#[test]
fn withdrawal_between_durable_admission_and_transport_read_never_issues_a_permit() {
    let (f, _) = call_setup();
    f.store.execute(&f.key.id, &call_start(&f, "dial")).unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let other = Store::open(f.store.path(), Limits::default()).unwrap();
    let (entered, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        crate::voice::pause_after_admission(entered, wait);
        assert!(other.begin_voice_dispatch(&job).unwrap().is_none());
    });
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    let actor = f.store.revalidate(&f.key.id).unwrap();
    f.store.revoke(&actor, &f.key.id).unwrap();
    release.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(state(&f), ("failed".into(), None));
    assert!(f.store.claim_voice_job().unwrap().is_none());
    let n: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM media_start_admissions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 1);
}
#[test]
fn forged_voice_route_or_poll_cannot_consume_admission_or_rebind_an_attempt() {
    let (f, _) = call_setup();
    f.store.execute(&f.key.id, &call_start(&f, "dial")).unwrap();
    let mut job = f.store.claim_voice_job().unwrap().unwrap();
    let original = job.route.assistant_id.clone();
    job.route.assistant_id = "substituted-assistant".into();
    assert_eq!(
        f.store.begin_voice_dispatch(&job).unwrap_err().code,
        "forbidden"
    );
    job.route.assistant_id = original;
    job.action = crate::VoiceAction::Poll;
    assert_eq!(
        f.store.begin_voice_dispatch(&job).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(state(&f), ("starting".into(), None));
    let n: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM media_start_admissions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0);
    job.action = crate::VoiceAction::Start;
    let dispatch = f.store.begin_voice_dispatch(&job).unwrap().unwrap();
    let mut wrong = receipt(&job.call.id);
    wrong.observation.mutation_id = "foreign-call".into();
    f.store
        .record_voice_dispatch(&dispatch.capture(Ok(wrong)))
        .unwrap();
    assert_eq!(state(&f), ("uncertain".into(), None));
    assert!(f.store.begin_voice_dispatch(&job).unwrap().is_none());
}
#[test]
fn captured_voice_recording_outage_preserves_same_result_through_revoke_and_duplicate_retry() {
    let (f, _) = call_setup();
    f.store.execute(&f.key.id, &call_start(&f, "dial")).unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let captured = f
        .store
        .begin_voice_dispatch(&job)
        .unwrap()
        .unwrap()
        .capture(Ok(receipt(&job.call.id)));
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_capture BEFORE INSERT ON connector_voice_observations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store.record_voice_dispatch(&captured).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_capture")
        .unwrap();
    let actor = f.store.revalidate(&f.key.id).unwrap();
    f.store.revoke(&actor, &f.key.id).unwrap();
    f.store.record_voice_dispatch(&captured).unwrap();
    f.store.record_voice_dispatch(&captured).unwrap();
    assert_eq!(
        state(&f),
        ("in-progress".into(), Some("captured-remote-call".into()))
    );
    let foreign =
        Store::initialize(&f._dir.path().join("foreign.sqlite3"), Limits::default()).unwrap();
    assert_eq!(
        foreign.record_voice_dispatch(&captured).unwrap_err().code,
        "forbidden"
    );
}
#[test]
fn voice_transport_expiry_is_clipped_to_original_task_instead_of_account_or_worker_budget() {
    use connect_protocol::tasks::*;
    let (f, _) = call_setup();
    let mut definition = f.create();
    if let TaskCommand::Create {
        request_id,
        definition,
    } = &mut definition.command
    {
        *request_id = "short-task".into();
        definition.expires_at = crate::now() + 10;
        definition.scope.operations = vec![TaskPermission::CallStart];
    }
    let t = Fixture::record(f.run(&definition).unwrap());
    f.run(&invoke(&t, "short-dial", call_start(&f, "short-dial")))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let dispatch = f.store.begin_voice_dispatch(&job).unwrap().unwrap();
    assert_eq!(dispatch.expires_at(), t.expires_at);
    assert!(dispatch.current());
}

#[test]
fn voice_transport_expiry_preserves_the_narrow_invocation_grant_ceiling() {
    use connect_protocol::tasks::*;
    let (f, t) = call_setup();
    let binding = super::grants::binding("short-voice", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &t.id, &binding)
        .unwrap();
    let expires = crate::now() + 10;
    let grant = f
        .store
        .issue_task_grant(
            &f.key.id,
            "short-issue",
            &GrantDefinition {
                task_id: t.id.clone(),
                binding: binding.clone(),
                operations: vec![TaskPermission::CallStart],
                expires_at: expires,
            },
        )
        .unwrap();
    let invocation = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &binding)
        .unwrap();
    f.store
        .execute_task_invocation(
            &invocation,
            &invoke(&t, "grant-dial", call_start(&f, "grant-dial")),
        )
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let dispatch = f.store.begin_voice_dispatch(&job).unwrap().unwrap();
    assert_eq!(dispatch.expires_at(), expires);
}
#[test]
fn disabled_start_route_does_not_erase_a_current_end_request_or_its_captured_result() {
    let (f, _) = call_setup();
    let call = active_call(&f);
    f.store
        .execute(&f.key.id, &call_end(&f, "end", &call))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let mut route = job.route.clone();
    route.enabled = false;
    f.store.configure_voice_route(&f.key.id, &route).unwrap();
    let dispatch = f.store.begin_voice_dispatch(&job).unwrap().unwrap();
    let mut result = receipt(&call);
    result.call.id = "provider-call".into();
    result.call.state = "ended".into();
    result.observation.external_id = Some("provider-call".into());
    result.observation.state = State::Voice(VoiceState::Ended);
    let captured = dispatch.capture(Ok(result));
    let actor = f.store.revalidate(&f.key.id).unwrap();
    f.store.revoke(&actor, &f.key.id).unwrap();
    f.store.record_voice_dispatch(&captured).unwrap();
    assert_eq!(state(&f), ("ended".into(), Some("provider-call".into())));
}

#[test]
fn retired_runtime_stops_voice_permit_and_original_custody_blocks_a_second_owner_until_drop() {
    let (f, _) = call_setup();
    f.store.execute(&f.key.id, &call_start(&f, "dial")).unwrap();
    let path = f.store.path().to_owned();
    let owner = f.store.require_runtime_owner().unwrap();
    let store = Store::open_with_ownership(&path, Limits::default(), &owner).unwrap();
    let job = store.claim_voice_job().unwrap().unwrap();
    let dispatch = store.begin_voice_dispatch(&job).unwrap().unwrap();
    assert!(dispatch.current());
    drop(f.store);
    owner.retire();
    drop(owner);
    assert!(!dispatch.current());
    assert!(dispatch.job().is_err());
    assert_eq!(
        crate::InstallationOwnership::acquire(&path)
            .unwrap_err()
            .code,
        "installation-busy"
    );
    let captured = dispatch.capture(Err(connect_connector_api::Failure::Uncertain));
    assert!(store.record_voice_dispatch(&captured).is_err());
    assert_eq!(
        crate::InstallationOwnership::acquire(&path)
            .unwrap_err()
            .code,
        "installation-busy"
    );
    drop(captured);
    let next = crate::InstallationOwnership::acquire(&path).unwrap();
    let next_store = Store::open_with_ownership(&path, Limits::default(), &next).unwrap();
    assert_ne!(store.runtime_epoch(), next_store.runtime_epoch());
    next_store.recover_voice_calls().unwrap();
    assert!(next_store.claim_voice_job().unwrap().is_none());
}

#[test]
fn captured_voice_recording_never_waits_on_current_delivery_or_a_foreign_sql_writer() {
    let (f, _) = call_setup();
    f.store.execute(&f.key.id, &call_start(&f, "dial")).unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let captured = f
        .store
        .begin_voice_dispatch(&job)
        .unwrap()
        .unwrap()
        .capture(Ok(receipt(&job.call.id)));
    let read = f
        .store
        .read_authority(
            &f.key.id,
            Some(&f.cid),
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        f.store.record_voice_dispatch(&captured).unwrap_err().code,
        "authority-busy"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(read);
    let c = rusqlite::Connection::open(f.store.path()).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        f.store.record_voice_dispatch(&captured).unwrap_err().code,
        "storage-unavailable"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    // Failed immediate attempts retain neither the authority writer nor SQL mutex.
    let read = f
        .store
        .read_authority(
            &f.key.id,
            Some(&f.cid),
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    drop(read);
    c.execute_batch("ROLLBACK").unwrap();
    f.store.record_voice_dispatch(&captured).unwrap();
    assert_eq!(
        state(&f),
        ("in-progress".into(), Some("captured-remote-call".into()))
    );
}
