use super::{effect_helpers::*, fixture::Fixture, sms_fixture::*};
use crate::Store;
use connect_protocol::{tasks::*, Operation};

#[test]
fn task_dispatch_uses_its_selected_purpose_for_owner_and_invocation() {
    for invocation in [false, true] {
        let (f, human) = setup();
        let task = Fixture::record(
            f.run(&create(&f, &human, "task", vec![reference()]))
                .unwrap(),
        );
        let request = send(&f, &task, "send", "Please review the draft", false);
        if invocation {
            f.store
                .execute_task_invocation(&handle(&f, &task), &request)
                .unwrap();
        } else {
            f.run(&request).unwrap();
        }
        assert!(f.store.claim_provider_job().unwrap().is_some());
        assert_eq!(
            decision(&f),
            (
                "sending".into(),
                Some("allowed".into()),
                Some("reviewed:selected".into())
            )
        );
    }
}
#[test]
fn task_without_sms_ceiling_cannot_borrow_conversation_enrollment() {
    let (f, human) = setup();
    let task = Fixture::record(f.run(&create(&f, &human, "task", vec![])).unwrap());
    f.run(&send(
        &f,
        &task,
        "send",
        "Native message remains useful",
        false,
    ))
    .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(decision(&f).1.as_deref(), Some("task-sms-purpose-required"));
    assert_eq!(count(&f, "messages"), 1);
    assert!(f
        .run(&TaskRequest::new(TaskCommand::Recall {
            task_id: task.id,
            after: None,
            limit: 10
        }))
        .is_ok());
    // Existing non-task clients retain Gate 3's reviewed enrollment policy.
    f.store
        .execute(
            &f.key.id,
            &Operation::Send {
                request_id: "ordinary".into(),
                conversation_id: f.cid.clone(),
                body: "Requested ordinary send".into(),
            },
        )
        .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_some());
}
#[test]
fn wrong_or_missing_reviewed_purpose_rolls_back_admission() {
    let (f, human) = setup();
    for missing in [false, true] {
        let mut r = reference();
        if missing {
            r.grant_id = "missing".into();
        } else {
            r.purpose = "Unreviewed task".into();
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert_eq!(
            f.run(&create(&f, &human, "task", vec![r]))
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}
#[test]
fn enrollment_for_another_conversation_cannot_be_attached_to_the_task() {
    let (f, human) = setup();
    let connect_protocol::Outcome::Conversation(other) = f
        .store
        .execute(
            &f.key.id,
            &Operation::Create {
                request_id: "other-conversation".into(),
                participants: vec![human.clone()],
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    let mut request = create(&f, &human, "task", vec![reference()]);
    let TaskCommand::Create { definition, .. } = &mut request.command else {
        panic!("create")
    };
    definition.conversation_id = other.id;
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(f.run(&request).unwrap_err().code, "forbidden");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn removal_and_stale_lease_cannot_fall_back_to_another_eligible_purpose() {
    for unavailable in 0..3 {
        let (f, human) = setup();
        let task = Fixture::record(
            f.run(&create(&f, &human, "task", vec![reference()]))
                .unwrap(),
        );
        f.run(&send(&f, &task, "send", "Requested draft", false))
            .unwrap();
        match unavailable {
            0 => f.store.expire_sms_authorities().unwrap(),
            1 => refresh(&f, 2, false),
            _ => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE sms_grants SET expires_at=?1 WHERE id='selected'",
                        [f.now],
                    )
                    .unwrap();
            }
        }
        assert!(f.store.claim_provider_job().unwrap().is_none());
        assert_eq!(
            decision(&f).1.as_deref(),
            Some("task-sms-enrollment-unavailable")
        );
        // SMS unavailability is not revocation of task history/native authority.
        assert!(f
            .run(&TaskRequest::new(TaskCommand::Recall {
                task_id: task.id,
                after: None,
                limit: 10
            }))
            .is_ok());
    }
}
#[test]
fn stop_and_start_do_not_revive_the_old_effect_or_cancel_the_task() {
    let (f, human) = setup();
    let task = Fixture::record(
        f.run(&create(&f, &human, "task", vec![reference()]))
            .unwrap(),
    );
    let old = send(&f, &task, "old", "Requested draft", false);
    f.run(&old).unwrap();
    f.store
        .receive_sms_control("stop", "stop-digest", PHONE, SENDER, "STOP", f.now)
        .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(decision(&f).0, "cancelled");
    f.store
        .receive_sms_control("start", "start-digest", PHONE, SENDER, "START", f.now + 1)
        .unwrap();
    assert!(f.run(&old).is_err()); // retained provider failure; never redispatch
    assert!(f.store.claim_provider_job().unwrap().is_none());
    f.run(&send(
        &f,
        &task,
        "new",
        "A newly authorized follow-up",
        false,
    ))
    .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_some());
    assert_eq!(count(&f, "provider_jobs"), 2);
    assert_eq!(
        Fixture::record(
            f.run(&TaskRequest::new(TaskCommand::Get { task_id: task.id }))
                .unwrap()
        )
        .state,
        TaskState::Running
    );
}
#[test]
fn schema_twelve_upgrade_preserves_old_grant_digest_and_task_receipts() {
    let f = Fixture::new();
    let task = f.task();
    let grant = super::grants::grant(&f, &task);
    let original = f.run(&super::grants::get(&task)).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TABLE connector_observations; DROP TABLE connector_receipt_events; DROP TABLE connector_incoming; DROP TABLE connector_voice_observations; DROP TABLE contact_pairings; DROP TABLE connector_manifests; DROP TABLE vapi_session_setups; DROP INDEX session_bindings_task_grant; ALTER TABLE session_bindings DROP COLUMN task_grant_id; DROP TABLE media_end_intents; DROP TABLE media_start_admissions; DROP INDEX task_effect_media_resource; PRAGMA user_version=12")
        .unwrap();
    assert_eq!(
        Store::open(f.store.path(), connect_protocol::Limits::default())
            .err()
            .unwrap()
            .code,
        "unsupported-schema"
    );
    let backup = f._dir.path().join("schema12.sqlite3");
    let expected = f.store.backup(&backup).unwrap();
    let after = Store::migrate(f.store.path(), &backup, &expected).unwrap();
    assert_eq!(after.schema, crate::SCHEMA_VERSION);
    let upgraded = Store::open(f.store.path(), connect_protocol::Limits::default()).unwrap();
    assert_eq!(
        upgraded
            .execute_task(&f.key.id, &super::grants::get(&task))
            .unwrap(),
        original
    );
    assert!(upgraded
        .authenticate_task_grant(grant.expose_secret(), &grant.grant.binding)
        .is_ok());
    assert_eq!(
        upgraded.execute_task(&f.key.id, &f.create()).unwrap(),
        original
    );
}
#[test]
fn start_cannot_restore_a_cancelled_sms_task_or_its_invocation() {
    let (f, human) = setup();
    let task = Fixture::record(
        f.run(&create(&f, &human, "task", vec![reference()]))
            .unwrap(),
    );
    let invocation = handle(&f, &task);
    let request = send(&f, &task, "send", "Requested draft", false);
    f.store
        .execute_task_invocation(&invocation, &request)
        .unwrap();
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
    f.store
        .receive_sms_control("start", "start-digest", PHONE, SENDER, "START", f.now)
        .unwrap();
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(decision(&f).0, "cancelled");
    assert_eq!(
        f.store
            .execute_task_invocation(&invocation, &request)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.run(&send(&f, &task, "new", "Still forbidden", false))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn sms_storage_failure_preserves_pending_job_without_dispatch_ack() {
    let (f, human) = setup();
    let task = Fixture::record(
        f.run(&create(&f, &human, "task", vec![reference()]))
            .unwrap(),
    );
    f.run(&send(&f, &task, "send", "Requested draft", false))
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TABLE sms_grants")
        .unwrap();
    assert_eq!(
        f.store.claim_provider_job().unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(decision(&f).0, "pending");
}
