use super::*;
use connect_protocol::{membership::*, tasks::*};

fn conversation(f: &Fixture) -> String {
    let Outcome::Conversation(c) = f
        .store
        .execute(
            &f.admin,
            &Operation::Create {
                request_id: "conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    c.id
}
fn artifact(f: &Fixture, cid: &str) -> String {
    let Outcome::Artifact(a) = f
        .store
        .execute(
            &f.admin,
            &Operation::ArtifactBegin {
                request_id: "artifact".into(),
                conversation_id: cid.into(),
                to: connect_protocol::messages::MessageAudience::all(),
                name: "one.txt".into(),
                media_type: "text/plain".into(),
                size: 1,
                sha256: "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881".into(),
                expires_at: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute(
            &f.admin,
            &Operation::ArtifactAppend {
                request_id: "append".into(),
                conversation_id: cid.into(),
                artifact_id: a.id.clone(),
                offset: 0,
                data: "eA==".into(),
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.admin,
            &Operation::ArtifactFinish {
                request_id: "finish".into(),
                conversation_id: cid.into(),
                artifact_id: a.id.clone(),
            },
        )
        .unwrap();
    a.id
}
fn task(f: &Fixture, cid: &str) -> TaskRecord {
    let TaskOutcome::Record(task) = f
        .store
        .execute_task(
            &f.admin,
            &TaskRequest::new(TaskCommand::Create {
                request_id: "task".into(),
                definition: TaskDefinition {
                    conversation_id: cid.into(),
                    scope: TaskScope {
                        operations: vec![TaskPermission::ArtifactDelete, TaskPermission::Send],
                        recipients: vec![f.identity.clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: crate::now() + 300,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap()
    else {
        panic!()
    };
    *task
}
fn definition(task: &TaskRecord, external: &str) -> GrantDefinition {
    GrantDefinition {
        task_id: task.id.clone(),
        binding: InvocationBinding {
            kind: InvocationKind::Native,
            adapter: "fixture".into(),
            external_id: external.into(),
        },
        operations: vec![TaskPermission::ArtifactDelete],
        expires_at: crate::now() + 120,
    }
}
fn hold(f: &Fixture, cid: &str) -> Arc<AuthorityGuardSet> {
    f.store
        .read_authority(
            &f.user,
            Some(cid),
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap()
}

#[test]
fn ordinary_close_privacy_and_artifact_deletion_wait_for_current_delivery() {
    for action in ["close", "privacy", "delete"] {
        let f = Fixture::new();
        let cid = conversation(&f);
        let op = match action {
            "close" => Operation::Close {
                request_id: "close".into(),
                conversation_id: cid.clone(),
                options: Default::default(),
            },
            "privacy" => Operation::ReceiptPreferencesSet {
                request_id: "privacy".into(),
                conversation_id: cid.clone(),
                delivery: false,
                read: false,
            },
            _ => Operation::ArtifactDelete {
                request_id: "delete".into(),
                conversation_id: cid.clone(),
                artifact_id: artifact(&f, &cid),
            },
        };
        let read = hold(&f, &cid);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let peer = f.peer();
        let admin = f.admin.clone();
        let writer = std::thread::spawn(move || peer.execute(&admin, &op));
        waiting(&f.store, &f.scope(AuthorityMode::Write));
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        let outcome = writer.join().unwrap().unwrap();
        match outcome {
            Outcome::Conversation(c) => assert!(c.closed),
            Outcome::ReceiptPreferences(p) => assert!(!p.read && !p.delivery),
            Outcome::Artifact(a) => assert_eq!(a.state, "deleted"),
            _ => panic!("unexpected outcome"),
        }
        assert_ne!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}

#[test]
fn held_write_proof_reuses_exact_scope_and_refuses_read_upgrade_partial_and_foreign_proofs() {
    let f = Fixture::new();
    let cid = conversation(&f);
    let op = Operation::Close {
        request_id: "close".into(),
        conversation_id: cid.clone(),
        options: Default::default(),
    };
    let read = hold(&f, &cid);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_authority_mutation(&f.admin, &op, &read)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(read);
    let partial = f
        .store
        .acquire_authority(
            &[f.scope(AuthorityMode::Write)],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_authority_mutation(&f.admin, &op, &partial)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(partial);
    let foreign = Fixture::new();
    let wrong = foreign
        .store
        .acquire_authority(
            &[
                f.scope(AuthorityMode::Write),
                AuthorityScope::conversation(&cid, AuthorityMode::Write).unwrap(),
            ],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_authority_mutation(&f.admin, &op, &wrong)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let write = f.store.operation_authority(&f.admin, &op, None).unwrap();
    let first = f
        .store
        .execute_authority_mutation(&f.admin, &op, &write)
        .unwrap();
    assert_eq!(
        f.store
            .execute_authority_mutation(&f.admin, &op, &write)
            .unwrap(),
        first
    );
    // A sufficient proof is neither a new scope nor a cached principal grant.
    assert_eq!(
        f.store
            .execute_authority_mutation("unknown", &op, &write)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert_eq!(
        f.store
            .execute_authority_mutation(&f.admin, &Operation::Health {}, &write)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn membership_and_provisioning_reuse_sufficient_write_proofs_without_reentering() {
    let f = Fixture::new();
    let cid = conversation(&f);
    let snapshot = f.store.membership_snapshot(&f.admin, &cid).unwrap();
    let request = MembershipRequest {
        operation_id: "role".into(),
        conversation_id: cid.clone(),
        expected_revision: snapshot.revision,
        change: MembershipAction::UpdateRole {
            participant_id: snapshot.participants[0].participant_id.clone(),
            role: "speaker".into(),
        },
    };
    let command = ProvisioningCommand::Endpoint {
        operation_id: "desktop".into(),
        identity_id: f.identity.clone(),
        device: DeviceSpec {
            id: "desktop".into(),
            kind: "desktop".into(),
            platform: "fixture".into(),
        },
        transport: EndpointTransport::Quic,
        address: "quic://localhost:4444".into(),
        capabilities: vec!["data".into()],
        secret_ref: None,
    };
    let read = hold(&f, &cid);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_membership_with_authority(&f.admin, request.clone(), &read)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_provisioning_with_authority(&f.admin, command.clone(), &read)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(read);
    let scopes = {
        let c = f.store.connection().unwrap();
        crate::authority_admission::conversation_scopes(&c, &cid, AuthorityMode::Write).unwrap()
    };
    let write = f
        .store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    f.store
        .execute_membership_with_authority(&f.admin, request, &write)
        .unwrap();
    assert_eq!(
        f.store
            .membership_snapshot_with_authority(&f.admin, &cid, &write)
            .unwrap()
            .participants[0]
            .role,
        "speaker"
    );
    f.store
        .execute_provisioning_with_authority(&f.admin, command, &write)
        .unwrap();
}

#[test]
fn owner_and_narrow_invocation_artifact_deletion_share_the_ordinary_write_fence() {
    for delegated in [false, true] {
        let f = Fixture::new();
        let cid = conversation(&f);
        let aid = artifact(&f, &cid);
        let task = task(&f, &cid);
        let def = definition(&task, "native");
        let grant = f
            .store
            .issue_local_task_grant(&f.admin, "grant", &def)
            .unwrap();
        let invocation = f
            .store
            .authenticate_local_task_grant(grant.expose_secret())
            .unwrap();
        let request = TaskRequest::new(TaskCommand::Invoke {
            request_id: "delete-task".into(),
            task_id: task.id,
            step_id: "delete".into(),
            operation: Box::new(Operation::ArtifactDelete {
                request_id: "delete-task".into(),
                conversation_id: cid.clone(),
                artifact_id: aid.clone(),
            }),
            reply_to: None,
            wait_for_reply: false,
        });
        let read = hold(&f, &cid);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let peer = f.peer();
        let admin = f.admin.clone();
        let writer = std::thread::spawn(move || {
            if delegated {
                peer.execute_task_invocation(&invocation, &request)
            } else {
                peer.execute_task(&admin, &request)
            }
        });
        waiting(&f.store, &f.scope(AuthorityMode::Write));
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) =
            writer.join().unwrap().unwrap()
        else {
            panic!()
        };
        assert!(matches!(*outcome, Outcome::Artifact(a) if a.id==aid && a.state=="deleted"));
    }
}

#[test]
fn task_cancel_grant_retirement_and_new_issuance_wait_for_current_delivery() {
    for action in ["cancel", "revoke", "issue"] {
        let f = Fixture::new();
        let cid = conversation(&f);
        let task = task(&f, &cid);
        let grant = f
            .store
            .issue_local_task_grant(&f.admin, "grant", &definition(&task, "native"))
            .unwrap();
        let invocation = f
            .store
            .authenticate_local_task_grant(grant.expose_secret())
            .unwrap();
        let read = hold(&f, &cid);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let peer = f.peer();
        let admin = f.admin.clone();
        let tid = task.id.clone();
        let gid = grant.grant.id.clone();
        let new_definition = definition(&task, "native-new");
        let writer = std::thread::spawn(move || match action {
            "cancel" => peer
                .execute_task(
                    &admin,
                    &TaskRequest::new(TaskCommand::Cancel {
                        request_id: "cancel".into(),
                        task_id: tid,
                        reason: None,
                    }),
                )
                .map(|_| ()),
            "revoke" => peer.revoke_task_grant(&admin, &gid),
            _ => peer
                .issue_local_task_grant(&admin, "new-grant", &new_definition)
                .map(|_| ()),
        });
        waiting(&f.store, &f.scope(AuthorityMode::Write));
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        writer.join().unwrap().unwrap();
        let result = f.store.execute_task_invocation(
            &invocation,
            &TaskRequest::new(TaskCommand::Get { task_id: task.id }),
        );
        if action == "issue" {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().code, "forbidden");
        }
    }
}

#[test]
fn exact_current_grant_replay_is_read_only_during_delivery_and_never_refreshes_denied_authority() {
    let f = Fixture::new();
    let cid = conversation(&f);
    let task = task(&f, &cid);
    let definition = definition(&task, "native");
    let grant = f
        .store
        .issue_local_task_grant(&f.admin, "grant", &definition)
        .unwrap();
    let read = hold(&f, &cid);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let replay = f
        .store
        .issue_local_task_grant(&f.admin, "grant", &definition)
        .unwrap();
    assert_eq!(replay.grant, grant.grant);
    assert_eq!(replay.expose_secret(), grant.expose_secret());
    let mut changed = definition.clone();
    changed.expires_at -= 1;
    assert_eq!(
        f.store
            .issue_local_task_grant(&f.admin, "grant", &changed)
            .unwrap_err()
            .code,
        "request-conflict"
    );
    assert_eq!(
        f.store
            .issue_local_task_grant(&f.admin, "new-grant", &definition)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(read);
    f.store
        .revoke_task_grant(&f.admin, &grant.grant.id)
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .issue_local_task_grant(&f.admin, "grant", &definition)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn shared_engine_refuses_authority_mutation_without_a_current_owned_write_proof() {
    let f = Fixture::new();
    let cid = conversation(&f);
    let op = Operation::Close {
        request_id: "bypass".into(),
        conversation_id: cid,
        options: Default::default(),
    };
    let actor = f.store.revalidate(&f.admin).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    {
        let c = f.store.connection().unwrap();
        let result = crate::operations::execute(
            &c,
            &actor,
            &op,
            &crate::DiscoveryRuntime::default(),
            &f.store,
            None,
        );
        assert_eq!(result.err().unwrap().code, "forbidden");
    }
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn ordinary_session_authority_changes_wait_for_delivery_and_recheck_original_state() {
    for action in ["accept", "reject", "cancel", "end"] {
        let f = Fixture::new();
        let other = f.store.create_identity("other").unwrap().id;
        let other_key = f
            .store
            .issue(&other, Scope::operator(), crate::now() + 300)
            .unwrap();
        let Outcome::Conversation(c) = f
            .store
            .execute(
                &f.admin,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        other.clone(),
                    ]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let Outcome::Session(session) = f
            .store
            .execute(
                &f.admin,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: c.id.clone(),
                    participant_id: other.clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!()
        };
        if action == "end" {
            f.store
                .execute(
                    &other_key.id,
                    &Operation::SessionAccept {
                        request_id: "accepted".into(),
                        conversation_id: c.id.clone(),
                        session_id: session.id.clone(),
                    },
                )
                .unwrap();
        }
        let credential = if matches!(action, "accept" | "reject") {
            other_key.id
        } else {
            f.admin.clone()
        };
        let op = match action {
            "accept" => Operation::SessionAccept {
                request_id: "change".into(),
                conversation_id: c.id.clone(),
                session_id: session.id.clone(),
            },
            "reject" => Operation::SessionReject {
                request_id: "change".into(),
                conversation_id: c.id.clone(),
                session_id: session.id.clone(),
                reason: None,
            },
            "cancel" => Operation::SessionCancel {
                request_id: "change".into(),
                conversation_id: c.id.clone(),
                session_id: session.id.clone(),
            },
            _ => Operation::SessionEnd {
                request_id: "change".into(),
                conversation_id: c.id.clone(),
                session_id: session.id.clone(),
                reason: None,
            },
        };
        let read = hold(&f, &c.id);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let peer = f.peer();
        let writer = std::thread::spawn(move || peer.execute(&credential, &op));
        waiting(
            &f.store,
            &AuthorityScope::identity(std::cmp::min(&other, &f.identity), AuthorityMode::Write)
                .unwrap(),
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        let Outcome::Session(updated) = writer.join().unwrap().unwrap() else {
            panic!()
        };
        assert_eq!(
            updated.state,
            match action {
                "accept" => "accepted",
                "reject" => "rejected",
                "cancel" => "cancelled",
                _ => "ended",
            }
        );
    }
}

#[test]
fn trusted_bind_unbind_and_session_end_wait_for_current_delivery() {
    for action in ["bind", "unbind", "end"] {
        let f = Fixture::new();
        let other = f.store.create_identity("other").unwrap().id;
        let other_key = f
            .store
            .issue(&other, Scope::operator(), crate::now() + 300)
            .unwrap();
        let Outcome::Conversation(c) = f
            .store
            .execute(
                &f.admin,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        other.clone(),
                    ]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let Outcome::Session(session) = f
            .store
            .execute(
                &f.admin,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: c.id.clone(),
                    participant_id: other.clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!()
        };
        f.store
            .execute(
                &other_key.id,
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: c.id.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        if action != "bind" {
            f.store
                .bind_session_connection(&f.admin, &session.id, "physical")
                .unwrap();
        }
        let read = hold(&f, &c.id);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let peer = f.peer();
        let key = f.admin.clone();
        let writer = std::thread::spawn(move || match action {
            "bind" => peer.bind_session_connection(&key, &session.id, "physical"),
            "unbind" => peer
                .unbind_session_connection("physical")
                .map(|v| v.unwrap()),
            _ => peer.end_routed_inbound_session(&session.id, "fixture-end"),
        });
        waiting(
            &f.store,
            &AuthorityScope::identity(std::cmp::min(&other, &f.identity), AuthorityMode::Write)
                .unwrap(),
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        let updated = writer.join().unwrap().unwrap();
        assert_eq!(
            updated.state,
            match action {
                "bind" => "ready",
                "unbind" => "accepted",
                _ => "ended",
            }
        );
    }
}

#[test]
fn contact_pairing_changes_wait_for_both_current_identity_authorities() {
    use connect_protocol::contacts::OwnershipState;
    for action in ["start", "complete", "revoke"] {
        let f = Fixture::new();
        let other = f.store.create_identity("other").unwrap().id;
        let peer_key = f
            .store
            .issue(&other, Scope::operator(), crate::now() + 300)
            .unwrap();
        let start = Operation::ContactPairingStart {
            request_id: "start".into(),
            peer_identity_id: other.clone(),
            expires_at: crate::now() + 300,
        };
        let op = if action == "start" {
            start
        } else {
            let Outcome::ContactPairing(pair) = f.store.execute(&f.admin, &start).unwrap() else {
                panic!()
            };
            if action == "complete" {
                Operation::ContactPairingComplete {
                    request_id: "complete".into(),
                    pairing_id: pair.id,
                    challenge: pair.challenge.unwrap(),
                }
            } else {
                Operation::ContactPairingRevoke {
                    request_id: "revoke".into(),
                    pairing_id: pair.id,
                }
            }
        };
        let read = f
            .store
            .read_authority(&f.user, None, &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let peer = f.peer();
        let key = if action == "complete" {
            peer_key.id
        } else {
            f.admin.clone()
        };
        let writer = std::thread::spawn(move || peer.execute(&key, &op));
        waiting(&f.store, &f.scope(AuthorityMode::Write));
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        let Outcome::ContactPairing(pair) = writer.join().unwrap().unwrap() else {
            panic!()
        };
        assert_eq!(
            pair.state,
            match action {
                "start" => OwnershipState::Pending,
                "complete" => OwnershipState::VerifiedNative,
                _ => OwnershipState::Revoked,
            }
        );
    }
}
