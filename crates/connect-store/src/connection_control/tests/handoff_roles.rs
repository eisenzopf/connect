//! Controlled captured Core facts qualify Store projection only, never live IO.
use super::*;
use crate::{NativeHandoffApplied, NativeHandoffOrigin, NativeMediaOrigin};
use connect_protocol::transitions::{
    EffectState, OperationAction, OperationState, OutgoingDisposition, RoleScope,
    TransitionCommand, TransitionIntent,
};
use std::sync::Arc;
pub(super) fn capture(
    f: &Fixture,
    sources: &[Arc<NativeMediaOrigin>],
    origin: &NativeHandoffOrigin,
    disposition: OutgoingDisposition,
) -> NativeHandoffApplied {
    let lease = handoff_custody::prepare(f, sources, origin, disposition);
    let p = lease.plan().unwrap();
    let attempt = handoff_custody::begin(f, lease);
    let receipt = attempt
        .applied(
            (handoff_custody::observation(sources, origin, &p)).clone(),
            crate::native_stream_projection::unchanged_audience_metadata_for_test(
                &(handoff_custody::observation(sources, origin, &p)),
            ),
        )
        .unwrap();
    f.store.record_native_handoff_applied(&receipt).unwrap();
    receipt
}
fn change_scope(
    f: &Fixture,
    origin: &NativeHandoffOrigin,
    scope: RoleScope,
) -> NativeHandoffOrigin {
    f.store
        .execute_operation_action(
            &f.admin,
            OperationAction::Cancel {
                operation_id: "cancel-original-session-scope".into(),
                target_operation_id: origin.request().operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
    let mut intent = origin.request().intent.clone();
    let rev: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [&f.cid],
            |r| r.get(0),
        )
        .unwrap();
    if let TransitionIntent::Handoff {
        role_scope,
        expected_conversation_revision,
        ..
    } = &mut intent
    {
        *role_scope = scope;
        *expected_conversation_revision = Some(rev);
    }
    let c = TransitionCommand {
        operation_id: "conversation-default-handoff".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: origin.request().connection_id.clone(),
        expected_revision: 3,
        intent,
        payload_extensions: origin.request().payload_extensions.clone(),
    };
    f.store
        .admit_transition_command(&f.keys[0], c.clone())
        .unwrap();
    f.store.native_handoff_origin(&f.keys[0], c).unwrap()
}
fn roles(f: &Fixture, sid: &str, pid: &str) -> (String, String) {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT role,state FROM session_participants WHERE session_id=?1 AND participant_id=?2",
            params![sid, pid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}
#[test]
fn handoff_captured_projection_commits_scope_and_disposition_once_without_changing_other_sessions()
{
    for scope in [RoleScope::Session, RoleScope::Conversation] {
        for disposition in [OutgoingDisposition::Observe, OutgoingDisposition::Leave] {
            let (f, sources, mut origin) = handoff_custody::fixture(disposition);
            f.store
                .execute(
                    &f.keys[0],
                    &Operation::SessionControl {
                        request: Request {
                            operation_id: "other-engagement".into(),
                            conversation_id: f.cid.clone(),
                            session_id: "other-session".into(),
                            by_participant: None,
                            expected_revision: None,
                            action: Action::Invite {
                                participants: vec![f.pids[2].clone()],
                                medium: "voice".into(),
                                intent: "conference".into(),
                                capabilities: json!({"audio":["opus"]}),
                            },
                        },
                    },
                )
                .unwrap();
            f.store
                .execute(
                    &f.keys[2],
                    &Operation::SessionControl {
                        request: Request {
                            operation_id: "accept-other-engagement".into(),
                            conversation_id: f.cid.clone(),
                            session_id: "other-session".into(),
                            by_participant: None,
                            expected_revision: Some(0),
                            action: Action::Accept {
                                capabilities: json!({"audio":["opus"]}),
                            },
                        },
                    },
                )
                .unwrap();
            {
                let c = f.store.connection().unwrap();
                c.execute("UPDATE session_participants SET role=CASE WHEN participant_id=?1 THEN 'agent' ELSE 'observer' END WHERE session_id='session' AND participant_id IN (?1,?2)",params![f.pids[0],f.pids[2]]).unwrap();
                c.execute("UPDATE participants SET role=CASE WHEN participant_id=?1 THEN 'agent' ELSE 'observer' END WHERE participant_id IN (?1,?2)",params![f.pids[0],f.pids[2]]).unwrap();
            }
            if scope == RoleScope::Conversation {
                origin = change_scope(&f, &origin, scope);
            }
            let others = [
                roles(&f, "other-session", &f.pids[0]),
                roles(&f, "other-session", &f.pids[2]),
            ];
            let receipt = capture(&f, &sources, &origin, disposition);
            let record = f.store.complete_native_handoff(&receipt).unwrap().unwrap();
            assert_eq!(record.state, OperationState::Completed);
            assert_eq!(record.effects[0].state, EffectState::Confirmed);
            assert_eq!(
                roles(&f, "session", &f.pids[2]),
                ("agent".into(), "present".into())
            );
            assert_eq!(
                roles(&f, "session", &f.pids[0]),
                (
                    "observer".into(),
                    if disposition == OutgoingDisposition::Observe {
                        "present".into()
                    } else {
                        "left".into()
                    }
                )
            );
            assert_eq!(
                others,
                [
                    roles(&f, "other-session", &f.pids[0]),
                    roles(&f, "other-session", &f.pids[2])
                ]
            );
            let defaults:(String,String)=f.store.connection().unwrap().query_row("SELECT a.role,b.role FROM participants a,participants b WHERE a.participant_id=?1 AND b.participant_id=?2",params![f.pids[0],f.pids[2]],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(
                defaults,
                if scope == RoleScope::Conversation {
                    ("observer".into(), "agent".into())
                } else {
                    ("agent".into(), "observer".into())
                }
            );
            let result = record.result.as_ref().unwrap();
            assert_eq!(result["session_revision"], 4);
            assert_eq!(
                result["retained_connection_ids"],
                json!([sources[1].connection_id()])
            );
            let remote: String = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT state FROM connections WHERE id=?1",
                    [sources[1].connection_id()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(remote, "ready");
            let role_events:u64=f.store.connection().unwrap().query_row("SELECT COUNT(*) FROM journal WHERE operation_record_id=?1 AND kind='conversation_participant_updated'",[origin.record_id()],|r|r.get(0)).unwrap();
            assert_eq!(
                role_events,
                if scope == RoleScope::Conversation {
                    8
                } else {
                    0
                }
            );
            let journal = f.count("journal");
            assert_eq!(
                f.store.complete_native_handoff(&receipt).unwrap().unwrap(),
                record
            );
            assert_eq!(f.count("journal"), journal);
            assert!(f.store.try_native_handoff_read(&origin, &sources).is_err());
        }
    }
}
#[test]
fn handoff_captured_projection_rolls_back_roles_result_and_events_on_capacity_failure() {
    let (mut f, sources, origin) = handoff_custody::fixture(OutgoingDisposition::Observe);
    let origin = change_scope(&f, &origin, RoleScope::Conversation);
    let receipt = capture(&f, &sources, &origin, OutgoingDisposition::Observe);
    let journal = f.count("journal");
    let limit = f.store.limits.journal_events;
    f.store.limits.journal_events = journal;
    assert_eq!(
        f.store.complete_native_handoff(&receipt).unwrap_err().code,
        "capacity-exceeded"
    );
    assert_eq!(roles(&f, "session", &f.pids[0]).0, "participant");
    assert_eq!(f.count("journal"), journal);
    let revision: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revision, 3);
    f.store.limits.journal_events = limit;
    assert_eq!(
        f.store
            .complete_native_handoff(&receipt)
            .unwrap()
            .unwrap()
            .state,
        OperationState::Completed
    );
}
#[test]
fn handoff_captured_projection_survives_late_cancel_and_caller_revocation_without_reexecuting() {
    let (f, sources, origin) = handoff_custody::fixture(OutgoingDisposition::Observe);
    let receipt = capture(&f, &sources, &origin, OutgoingDisposition::Observe);
    let cancel = f
        .store
        .execute_operation_action(
            &f.admin,
            OperationAction::Cancel {
                operation_id: "late-cancel-known-handoff".into(),
                target_operation_id: origin.request().operation_id.clone(),
                reason: None,
            },
        )
        .unwrap();
    assert!(cancel
        .result
        .unwrap()
        .to_string()
        .contains("reconciliation_required"));
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    let record = f.store.complete_native_handoff(&receipt).unwrap().unwrap();
    assert_eq!(record.state, OperationState::Completed);
    assert!(record.cancel_requested);
    assert_eq!(record.effects[0].attempt, 1);
    assert_eq!(roles(&f, "session", &f.pids[0]).0, "observer");
}
#[test]
fn handoff_captured_leave_projects_after_exact_native_source_retirement_without_adopting_new_incarnations(
) {
    for change in [
        "exact",
        "missing-retirement",
        "later-generation",
        "other-runtime",
        "bindings-immutable",
        "observer",
    ] {
        let disposition = if change == "observer" {
            OutgoingDisposition::Observe
        } else {
            OutgoingDisposition::Leave
        };
        let (f, sources, origin) = handoff_custody::fixture(disposition);
        let receipt = capture(&f, &sources, &origin, disposition);
        let original = &sources[0];
        {
            let c = f.store.connection().unwrap();
            // Controlled equivalent of settle_native_offer_cleanup's durable
            // physical observation; no engine IO is performed by this fixture.
            if change != "missing-retirement" {
                c.execute(
                    "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
                    [original.connection_id()],
                )
                .unwrap();
            }
            c.execute("UPDATE connections SET state='retired',generation=generation+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1", [original.connection_id()]).unwrap();
            match change {
                "later-generation" => {
                    c.execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [original.connection_id()],
                    )
                    .unwrap();
                }
                "other-runtime" => {
                    c.execute(
                        "UPDATE connections SET observed_runtime_id='other-runtime' WHERE id=?1",
                        [original.connection_id()],
                    )
                    .unwrap();
                }
                "bindings-immutable" => {
                    for sql in [
                        "UPDATE connections SET endpoint_id='different-endpoint' WHERE id=?1",
                        "UPDATE connection_offers SET runtime_connection_id='different-physical' WHERE connection_id=?1",
                        "UPDATE native_connection_lifetimes SET runtime_epoch='different-runtime' WHERE connection_id=?1",
                    ] {
                        let error = c.execute(sql, [original.connection_id()]).unwrap_err();
                        assert_eq!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::ConstraintViolation));
                    }
                }
                _ => {}
            }
        }
        let journal = f.count("journal");
        let result = f.store.complete_native_handoff(&receipt).unwrap();
        if matches!(change, "exact" | "bindings-immutable") {
            let result = result.unwrap();
            assert_eq!(result.state, OperationState::Completed);
            assert_eq!(result.effects[0].attempt, 1);
            assert_eq!(roles(&f, "session", &f.pids[0]).1, "left");
            let c = f.store.connection().unwrap();
            let state: (String, u64) = c
                .query_row(
                    "SELECT state,generation FROM connections WHERE id=?1",
                    [original.connection_id()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(state, ("retired".into(), original.generation() + 1));
            drop(c);
            let events = f.count("journal");
            assert_eq!(
                f.store.complete_native_handoff(&receipt).unwrap().unwrap(),
                result
            );
            assert_eq!(f.count("journal"), events);
        } else {
            assert!(result.is_none(), "{change}");
            assert_eq!(f.count("journal"), journal);
            assert_eq!(roles(&f, "session", &f.pids[0]).1, "present");
        }
    }
}
#[test]
fn handoff_captured_projection_never_overwrites_newer_topology_conversation_or_member_epochs() {
    for change in [
        "session",
        "conversation",
        "member",
        "generation",
        "physical-loss",
    ] {
        let (f, sources, origin) = handoff_custody::fixture(OutgoingDisposition::Observe);
        let origin = change_scope(&f, &origin, RoleScope::Conversation);
        let receipt = capture(&f, &sources, &origin, OutgoingDisposition::Observe);
        {
            let c = f.store.connection().unwrap();
            match change {
                "session" => {
                    c.execute("UPDATE sessions SET topology_revision=topology_revision+1 WHERE id='session'",[]).unwrap();
                }
                "conversation" => {
                    c.execute(
                        "UPDATE conversations SET revision=revision+1 WHERE id=?1",
                        [&f.cid],
                    )
                    .unwrap();
                }
                "member" => {
                    c.execute(
                        "UPDATE participants SET active=0 WHERE participant_id=?1",
                        [&f.pids[2]],
                    )
                    .unwrap();
                }
                "generation" => {
                    c.execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [sources[2].connection_id()],
                    )
                    .unwrap();
                }
                "physical-loss" => {
                    c.execute(
                        "UPDATE connections SET state='failed' WHERE id=?1",
                        [sources[2].connection_id()],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        let journal = f.count("journal");
        assert!(
            f.store.complete_native_handoff(&receipt).unwrap().is_none(),
            "{change}"
        );
        assert_eq!(f.count("journal"), journal);
        assert_eq!(roles(&f, "session", &f.pids[0]).0, "participant");
        let record = f
            .store
            .execute_operation_action(
                &f.admin,
                OperationAction::Get {
                    operation_id: origin.request().operation_id.clone(),
                },
            )
            .unwrap();
        assert!(record.result.is_none());
        assert_eq!(record.effects[0].state, EffectState::Confirmed);
    }
}
