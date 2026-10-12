//! Contract admission is deliberately tested separately from future runtime authority.
#![allow(clippy::unwrap_used)]
use connect_protocol::{tasks::*, Operation};
use serde_json::json;

fn definition() -> TaskDefinition {
    TaskDefinition {
        conversation_id: "conversation-a".into(),
        scope: TaskScope {
            sms_enrollments: Vec::new(),
            operations: vec![TaskPermission::Send, TaskPermission::History],
            recipients: vec!["human-a".into()],
        },
        expires_at: 1_300,
        initiating_event_id: None,
    }
}
#[test]
fn reviewed_sms_policy_is_explicit_bounded_and_legacy_scope_serialization_is_stable() {
    let mut scope = definition().scope;
    let legacy = json!({"operations":["send","history"],"recipients":["human-a"]});
    assert_eq!(serde_json::to_value(&scope).unwrap(), legacy);
    assert_eq!(serde_json::from_value::<TaskScope>(legacy).unwrap(), scope);
    let reference = TaskSmsEnrollment {
        source: "reviewed".into(),
        grant_id: "grant".into(),
        purpose: "Review the draft".into(),
    };
    scope.sms_enrollments = vec![reference.clone()];
    assert!(scope.validate().is_ok());
    scope.sms_enrollments.push(reference.clone());
    assert!(scope.validate().is_err());
    scope.sms_enrollments = vec![reference];
    scope.operations = vec![TaskPermission::History];
    assert!(scope.validate().is_err());
    scope.operations = vec![TaskPermission::Send];
    scope.sms_enrollments[0].purpose = " ".into();
    assert!(scope.validate().is_err());
    scope.sms_enrollments[0].purpose = "x".repeat(2001);
    assert!(scope.validate().is_err());
}
#[test]
fn every_task_state_transition_and_terminal_boundary_is_defined() {
    use TaskState::*;
    let states = [
        Ready, Running, Waiting, Completed, Cancelled, Failed, Expired,
    ];
    for from in states {
        for to in states {
            let expected = !from.is_terminal()
                && (matches!(to, Cancelled | Failed | Expired)
                    || matches!(
                        (from, to),
                        (Ready, Running) | (Running, Waiting | Completed) | (Waiting, Ready)
                    ));
            assert_eq!(from.transition(to).is_ok(), expected, "{from:?} → {to:?}");
        }
    }
    assert!(Completed.transition(Ready).is_err());
    assert!(Cancelled.transition(Running).is_err());
}
#[test]
fn uncertain_or_terminal_effect_can_never_be_redispatched() {
    use EffectState::*;
    let states = [
        Prepared, Dispatched, Succeeded, Failed, Uncertain, Cancelled,
    ];
    for from in states {
        for to in states {
            let expected = matches!(
                (from, to),
                (Prepared, Dispatched | Cancelled)
                    | (Dispatched, Succeeded | Failed | Uncertain)
                    | (Uncertain, Succeeded | Failed)
            );
            assert_eq!(from.transition(to).is_ok(), expected, "{from:?} → {to:?}");
        }
    }
    for state in states {
        assert_eq!(state.may_dispatch(), state == Prepared);
    }
}
#[test]
fn task_profile_is_explicit_and_old_control_is_unchanged() {
    let mut request = TaskRequest::new(TaskCommand::Create {
        request_id: "create-task".into(),
        definition: definition(),
    });
    request.validate(1_000).unwrap();
    request.task_profile = "connect-task/2".into();
    assert_eq!(
        request.validate(1_000).unwrap_err().code,
        "unsupported-profile"
    );
    let old: Operation = serde_json::from_value(
        json!({"op":"send","request_id":"old","conversation_id":"conversation-a","body":"hello"}),
    )
    .unwrap();
    assert_eq!(old.name(), "send");
    assert!(serde_json::from_value::<Operation>(json!({"op":"task","task_profile":"connect-task/1","command":{"action":"get","task_id":"task-a"}})).is_err(),"Integration remains a later item; the previous enum fails closed");
}
#[test]
fn claimed_owner_grant_and_wildcard_fields_are_rejected() {
    for key in ["owner_id", "grant", "credential_id", "call_id", "sender"] {
        let mut value = serde_json::to_value(TaskRequest::new(TaskCommand::Get {
            task_id: "task-a".into(),
        }))
        .unwrap();
        value["command"][key] = json!("claimed");
        assert!(
            serde_json::from_value::<TaskRequest>(value).is_err(),
            "{key}"
        );
    }
    let mut value = serde_json::to_value(definition()).unwrap();
    value["owner_id"] = json!("attacker");
    assert!(serde_json::from_value::<TaskDefinition>(value).is_err());
    assert!(serde_json::from_value::<TaskScope>(
        json!({"operations":["*"],"recipients":["human"]})
    )
    .is_err());
    assert!(serde_json::from_value::<TaskScope>(
        json!({"operations":["create"],"recipients":["human"]})
    )
    .is_err());
    let mut scope = definition().scope;
    scope.recipients = vec!["*".into()];
    assert!(scope.validate().is_err());
}
#[test]
fn empty_duplicate_and_excessive_scopes_cannot_delegate() {
    let base = definition().scope;
    let mut scope = base.clone();
    scope.operations.clear();
    assert!(scope.validate().is_err());
    let mut scope = base.clone();
    scope.operations.push(TaskPermission::Send);
    assert!(scope.validate().is_err());
    let mut scope = base.clone();
    scope.recipients.push("human-a".into());
    assert!(scope.validate().is_err());
    let mut scope = base;
    scope.recipients = (0..=MAX_SCOPE_ITEMS)
        .map(|i| format!("human-{i}"))
        .collect();
    assert!(scope.validate().is_err());
}
#[test]
fn expiry_boundaries_and_overflow_fail_admission() {
    let mut value = definition();
    value.expires_at = 1_000;
    assert!(value.validate(1_000).is_err());
    value.expires_at = 1_000 + MAX_TASK_TTL_SECONDS;
    value.validate(1_000).unwrap();
    value.expires_at += 1;
    assert!(value.validate(1_000).is_err());
    value.expires_at = i64::MAX;
    assert!(value.validate(1_000).is_err());
    value.expires_at = 1_300;
    value.conversation_id = "x".repeat(MAX_TASK_ID_BYTES + 1);
    assert!(value.validate(1_000).is_err());
    value.conversation_id = "line\nbreak".into();
    assert!(value.validate(1_000).is_err());
}
#[test]
fn wait_and_pagination_bounds_are_explicit() {
    let cmd = |limit, wait_ms| {
        TaskRequest::new(TaskCommand::Wait {
            task_id: "task-a".into(),
            after: None,
            limit,
            wait_ms,
        })
    };
    cmd(100, MAX_TOOL_WAIT_MS).validate(1_000).unwrap();
    assert!(cmd(0, 1).validate(1_000).is_err());
    assert!(cmd(101, 1).validate(1_000).is_err());
    assert!(cmd(1, MAX_TOOL_WAIT_MS + 1).validate(1_000).is_err());
    let request = TaskRequest::new(TaskCommand::Recall {
        task_id: "task-a".into(),
        after: Some("x".repeat(1025)),
        limit: 1,
    });
    assert!(request.validate(1_000).is_err());
}
#[test]
fn nested_admin_commands_and_rekeyed_mutations_are_denied() {
    let invoke = |operation| {
        TaskRequest::new(TaskCommand::Invoke {
            request_id: "logical-a".into(),
            task_id: "task-a".into(),
            step_id: "step-a".into(),
            operation: Box::new(operation),
            reply_to: None,
            wait_for_reply: false,
        })
    };
    let send = |id: &str| Operation::Send {
        request_id: id.into(),
        conversation_id: "conversation-a".into(),
        body: "ask".into(),
    };
    invoke(send("logical-a")).validate(1_000).unwrap();
    assert_eq!(
        invoke(send("different-logical-id"))
            .validate(1_000)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        invoke(Operation::Create {
            request_id: "logical-a".into(),
            participants: vec!["human-a".into()]
        })
        .validate(1_000)
        .unwrap_err()
        .code,
        "forbidden"
    );
    assert_eq!(
        invoke(Operation::Close {
            request_id: "logical-a".into(),
            conversation_id: "conversation-a".into()
        })
        .validate(1_000)
        .unwrap_err()
        .code,
        "forbidden"
    );
    assert!(TaskPermission::for_operation(&Operation::Health {}).is_err());
}

#[test]
fn explicit_question_intent_is_additive_but_only_messages_can_await_or_bind_replies() {
    let json = serde_json::json!({"task_profile":TASK_PROFILE,"command":{"action":"invoke","request_id":"ask","task_id":"task","step_id":"step","operation":{"op":"send","request_id":"ask","conversation_id":"conversation","body":"question"},"reply_to":null}});
    let mut request: TaskRequest = serde_json::from_value(json.clone()).unwrap();
    request.validate(1_000).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), json);
    let TaskCommand::Invoke {
        wait_for_reply,
        operation,
        ..
    } = &mut request.command
    else {
        panic!("invoke")
    };
    *wait_for_reply = true;
    **operation = Operation::History {
        conversation_id: "conversation".into(),
        after: None,
        limit: 10,
    };
    assert_eq!(request.validate(1_000).unwrap_err().code, "invalid-request");
    let TaskCommand::Invoke {
        wait_for_reply,
        reply_to,
        ..
    } = &mut request.command
    else {
        panic!("invoke")
    };
    *wait_for_reply = false;
    *reply_to = Some("event".into());
    assert_eq!(request.validate(1_000).unwrap_err().code, "invalid-request");
}

#[test]
fn effect_error_metadata_reads_older_results_without_fabricating_a_failure() {
    let old = serde_json::json!({"id":"effect","task_id":"task","step_id":"step","request_id":"send","state":"succeeded","outcome":null});
    let effect: TaskEffect = serde_json::from_value(old.clone()).unwrap();
    assert!(effect.error.is_none());
    assert_eq!(serde_json::to_value(&effect).unwrap(), old);
}

#[test]
fn runner_lease_commands_have_stable_owner_scopes_and_validated_identifiers() {
    let commands = [
        TaskCommand::Claim {
            request_id: "claim".into(),
            task_id: "task".into(),
            step_id: "step".into(),
            worker_id: "worker".into(),
            expected_revision: None,
        },
        TaskCommand::ClaimGet {
            task_id: "task".into(),
            step_id: "step".into(),
        },
        TaskCommand::ReleaseClaim {
            request_id: "release".into(),
            task_id: "task".into(),
            step_id: "step".into(),
            worker_id: "worker".into(),
            expected_revision: 0,
        },
    ];
    for (command, scope) in
        commands
            .into_iter()
            .zip(["task_claim", "task_claim_get", "task_release_claim"])
    {
        assert_eq!(command.required_scope(), scope);
        assert_eq!(command.task_id(), Some("task"));
        let request = TaskRequest::new(command);
        request.validate(1_000).unwrap();
        let copy: TaskRequest =
            serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(copy, request);
    }
    assert_eq!(
        TaskRequest::new(TaskCommand::ClaimGet {
            task_id: "task".into(),
            step_id: "../unsafe".into()
        })
        .validate(1_000)
        .unwrap_err()
        .code,
        "invalid-request"
    );
}
#[test]
fn pending_and_uncertain_results_do_not_claim_an_outcome() {
    let pending = TaskToolResult::Pending {
        task_id: "task-a".into(),
        effect_id: None,
    };
    let value = serde_json::to_value(&pending).unwrap();
    assert_eq!(value["status"], "pending");
    assert!(value.get("outcome").is_none());
    assert_eq!(
        serde_json::from_value::<TaskToolResult>(value).unwrap(),
        pending
    );
    let uncertain = TaskToolResult::Uncertain {
        task_id: "task-a".into(),
        effect_id: "effect-a".into(),
    };
    assert_eq!(
        serde_json::to_value(uncertain).unwrap()["status"],
        "uncertain"
    );
}
#[test]
fn all_command_families_have_stable_names_and_logical_ids() {
    let request = TaskRequest::new(TaskCommand::ConsumeReply {
        request_id: "consume-a".into(),
        task_id: "task-a".into(),
        event_id: "event-a".into(),
        expected_revision: 3,
    });
    let value = serde_json::to_value(&request).unwrap();
    let back: TaskRequest = serde_json::from_value(value).unwrap();
    back.validate(1_000).unwrap();
    assert_eq!(back, request);
    assert_eq!(back.command.name(), "task_consume_reply");
    assert_eq!(back.command.request_id(), Some("consume-a"));
    assert_eq!(back.command.task_id(), Some("task-a"));
}

#[test]
fn trusted_grant_shapes_do_not_invent_provider_authority() {
    let mut grant = GrantDefinition {
        task_id: "task-a".into(),
        binding: InvocationBinding {
            kind: InvocationKind::ProviderCall,
            adapter: "vapi".into(),
            external_id: "verified-call-a".into(),
        },
        operations: vec![TaskPermission::Send],
        expires_at: 1_300,
    };
    grant.validate(1_000).unwrap();
    grant.expires_at += 1;
    assert!(grant.validate(1_000).is_err());
    grant.expires_at = 1_000;
    assert!(grant.validate(1_000).is_err());
    grant.expires_at = 1_300;
    grant.operations.push(TaskPermission::Send);
    assert!(grant.validate(1_000).is_err());
    grant.operations.pop();
    grant.binding.external_id = "X-Call-Id\nforged".into();
    assert!(grant.validate(1_000).is_err());
    let mut command = serde_json::to_value(TaskCommand::Get {
        task_id: "task-a".into(),
    })
    .unwrap();
    command["binding"] = serde_json::json!({"kind":"provider_call","external_id":"claimed"});
    assert!(serde_json::from_value::<TaskCommand>(command).is_err());
}

#[test]
fn native_reply_uses_sender_authority_and_cannot_choose_task_ownership() {
    let request = TaskRequest::new(TaskCommand::Reply {
        request_id: "reply-a".into(),
        conversation_id: "conversation-a".into(),
        reply_to: "question-a".into(),
        body: "Use the revised draft".into(),
    });
    request.validate(1_000).unwrap();
    assert_eq!(request.command.required_scope(), "send");
    assert_eq!(
        request.command.task_id(),
        None,
        "The store derives the task from the verified question"
    );
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<TaskRequest>(value.clone()).unwrap(),
        request
    );
    for key in ["task_id", "owner_id", "credential_id", "binding"] {
        let mut forged = value.clone();
        forged["command"][key] = json!("claimed");
        assert!(
            serde_json::from_value::<TaskRequest>(forged).is_err(),
            "{key}"
        );
    }
    let invalid = TaskRequest::new(TaskCommand::Reply {
        request_id: "reply-a".into(),
        conversation_id: "conversation-a".into(),
        reply_to: "".into(),
        body: "answer".into(),
    });
    assert!(invalid.validate(1_000).is_err());
}
