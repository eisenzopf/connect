#![allow(clippy::unwrap_used)]
use super::*;
use crate::{Scope, SmsGrant, SmsGrantSnapshot};
use connect_protocol::{
    conversations::CreateRequest, membership::*, provisioning::*, Limits, Operation, Outcome,
};
struct F {
    _tmp: tempfile::TempDir,
    s: Store,
    root: String,
    account: String,
    owner: String,
    delegate: String,
    peer: String,
    observer: String,
    rooms: Vec<String>,
    policies: Vec<ProviderMessageEndpoint>,
}
impl F {
    fn new(n: usize, all: bool) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::initialize(&tmp.path().join("state.sqlite3"), Limits::default()).unwrap();
        let account = s.create_identity("account").unwrap().id;
        let alice = s.create_identity("alice").unwrap().id;
        let peer = s.create_identity("peer").unwrap().id;
        let observer = s.create_identity("observer").unwrap().id;
        let root = s
            .issue(&account, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let owner = s
            .issue(&alice, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let delegate = s
            .issue(
                &alice,
                Scope::messaging(vec![peer.clone(), observer.clone()]),
                crate::now() + 600,
            )
            .unwrap()
            .id;
        let ep = |identity: &str, name: &str, address: &str| {
            let ProvisioningOutcome::Endpoint { endpoint_id, .. } = s
                .execute_provisioning(
                    &root,
                    ProvisioningCommand::Endpoint {
                        operation_id: format!("ep-{name}"),
                        identity_id: identity.into(),
                        device: DeviceSpec {
                            id: format!("dev-{name}"),
                            kind: "service".into(),
                            platform: "sms-custody-fixture".into(),
                        },
                        transport: EndpointTransport::Sms,
                        address: address.into(),
                        capabilities: vec!["messaging".into()],
                        secret_ref: Some("SMS_ROOT".into()),
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            s.execute_provisioning(
                &root,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: format!("verify-{name}"),
                    endpoint_id: endpoint_id.clone(),
                    expected_revision: 0,
                    evidence_ref: "isolated-fixture".into(),
                },
            )
            .unwrap();
            endpoint_id
        };
        let target = ep(&alice, "target", "+15555550101");
        let source = ep(&account, "source", "+15555550102");
        let mut rooms = vec![];
        let mut policies = vec![];
        for i in 0..n {
            let Outcome::Conversation(room) = s
                .execute(
                    &owner,
                    &Operation::Create {
                        request_id: format!("room-{i}"),
                        request: CreateRequest::persistent(vec![peer.clone(), observer.clone()]),
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            let peer_member =
                crate::audience::member(&s.connection().unwrap(), &peer, &room.id, true).unwrap();
            let route = Route {
                id: format!("sms-{i}"),
                identity_id: alice.clone(),
                credential_id: delegate.clone(),
                conversation_id: room.id.clone(),
                connector: "telnyx".into(),
                target: "+15555550101".into(),
                source: Some("+15555550102".into()),
                enabled: true,
                incoming_to: if all {
                    MessageAudience::all()
                } else {
                    MessageAudience::Participants(vec![peer_member.participant_id])
                },
            };
            s.configure_route(&root, &route).unwrap();
            policies.push(ProviderMessageEndpoint {
                route_id: route.id,
                connector: "telnyx".into(),
                endpoint_id: target.clone(),
                source_endpoint_id: Some(source.clone()),
                account_credential_id: root.clone(),
                secret_reference: "SMS_ROOT".into(),
            });
            rooms.push(room.id);
        }
        s.bind_provider_message_endpoints(&root, &policies).unwrap();
        let f = Self {
            _tmp: tmp,
            s,
            root,
            account,
            owner,
            delegate,
            peer,
            observer,
            rooms,
            policies,
        };
        f.enroll(1, true);
        f
    }
    fn enroll(&self, revision: i64, enabled: bool) {
        self.s
            .refresh_sms_grants(
                "fixture",
                &SmsGrantSnapshot {
                    revision,
                    grants: if enabled {
                        self.rooms
                            .iter()
                            .enumerate()
                            .map(|(i, cid)| SmsGrant {
                                id: format!("grant-{i}"),
                                phone: "+15555550101".into(),
                                sender: "+15555550102".into(),
                                conversation_id: cid.clone(),
                                purpose: "Isolated requested incoming SMS".into(),
                                consent_id: "reviewed-form".into(),
                                disclosure_version: "v1".into(),
                                disclosure_text: "Explicit isolated consent".into(),
                                submitted_at: crate::now() - 100,
                                approved_at: crate::now() - 90,
                                expires_at: crate::now() + 600,
                            })
                            .collect()
                    } else {
                        vec![]
                    },
                },
            )
            .unwrap();
    }
    fn receive(&self, event: &str, resource: &str, body: &str) -> Result<bool> {
        let digest =
            crate::providers::event_digest(format!("{event}\0{resource}\0{body}").as_bytes());
        self.s.receive_sms_for_routes(
            &SmsObservation {
                event_id: event,
                event_digest: &digest,
                provider_id: resource,
                from: "+15555550101",
                to: "+15555550102",
                body,
                occurred_at: None,
            },
            &self
                .policies
                .iter()
                .map(|p| p.route_id.clone())
                .collect::<Vec<_>>(),
        )
    }
    fn held(&self) -> IncomingContact {
        self.s
            .incoming_contacts(&self.root, None, 32)
            .unwrap()
            .contacts
            .into_iter()
            .find(|c| c.state == IncomingContactState::Held)
            .unwrap()
    }
    fn count(&self, t: &str) -> u64 {
        self.s
            .connection()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
            .unwrap()
    }
    fn change(&self, cid: &str, id: &str, change: MembershipAction) {
        let revision = self
            .s
            .membership_snapshot(&self.owner, cid)
            .unwrap()
            .revision;
        self.s
            .execute(
                &self.owner,
                &Operation::Membership {
                    request: MembershipRequest {
                        operation_id: id.into(),
                        conversation_id: cid.into(),
                        expected_revision: revision,
                        change,
                    },
                },
            )
            .unwrap();
    }
}
#[test]
fn overlapping_verified_routes_hold_once_and_resolution_delivers_one_private_message() {
    let f = F::new(2, false);
    let before = f.count("journal");
    assert!(!f
        .receive("event", "resource", "private ambiguous text")
        .unwrap());
    assert_eq!(f.count("journal"), before);
    assert_eq!(f.count("messages"), 0);
    let held = f.held();
    assert_eq!(held.reason, IncomingContactReason::AmbiguousRoute);
    assert_eq!(held.candidates.len(), 2);
    assert!(!serde_json::to_string(&held)
        .unwrap()
        .contains("private ambiguous text"));
    assert!(!f
        .receive("retry-event", "resource", "private ambiguous text")
        .unwrap());
    assert_eq!(f.count("incoming_contact_holds"), 1);
    let chosen = held
        .candidates
        .iter()
        .find(|p| p.conversation_id == f.rooms[1])
        .unwrap();
    let result =
        f.s.resolve_incoming_sms_contact(&f.root, "resolve", &held.id, 0, &chosen.id)
            .unwrap();
    assert_eq!(result.state, IncomingContactState::Resolved);
    let decision = result.decision.as_ref().unwrap();
    assert_eq!(decision.candidate_id.as_deref(), Some(chosen.id.as_str()));
    assert_eq!(decision.actor_identity_id, f.account);
    assert_eq!(decision.operation_id, "resolve");
    assert_eq!(
        f.s.resolve_incoming_sms_contact(&f.root, "resolve", &held.id, 0, &chosen.id)
            .unwrap(),
        result
    );
    assert_eq!(f.count("messages"), 1);
    let c = f.s.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM messages WHERE conversation_id=?1",
            [&f.rooms[0]],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert!(!crate::audience::visible_to_identity(
        &c,
        &f.observer,
        &f.rooms[1],
        ContentKind::Message,
        decision.message_id.as_ref().unwrap()
    )
    .unwrap());
    assert!(crate::audience::visible_to_identity(
        &c,
        &f.peer,
        &f.rooms[1],
        ContentKind::Message,
        decision.message_id.as_ref().unwrap()
    )
    .unwrap());
    drop(c);
    assert!(!f
        .receive("event", "resource", "private ambiguous text")
        .unwrap());
    assert_eq!(f.count("incoming_contact_audit"), 2);
}
#[test]
fn unique_resource_retries_ignore_new_event_ids_and_conflicting_reuse_never_delivers_again() {
    let f = F::new(1, false);
    assert!(f.receive("event", "same-id", "original").unwrap());
    assert!(!f.receive("another-event", "same-id", "original").unwrap());
    assert_eq!(f.count("messages"), 1);
    assert!(!f
        .receive("reused-event", "same-id", "different content")
        .unwrap());
    let held = f.held();
    assert_eq!(held.reason, IncomingContactReason::ResourceConflict);
    assert!(f
        .s
        .resolve_incoming_sms_contact(
            &f.root,
            "cannot-rebind",
            &held.id,
            0,
            &held.candidates[0].id
        )
        .is_err());
    assert_eq!(f.count("messages"), 1);
    assert_eq!(
        f.receive("event", "same-id", "changed immutable event")
            .unwrap_err()
            .code,
        "conflict"
    );
}
#[test]
fn ineligible_original_resource_cannot_be_revived_by_later_enrollment() {
    let f = F::new(1, false);
    f.enroll(2, false);
    assert!(!f
        .receive("event", "not-enrolled", "never a prompt")
        .unwrap());
    let contact =
        f.s.incoming_contacts(&f.root, None, 32)
            .unwrap()
            .contacts
            .remove(0);
    assert_eq!(contact.reason, IncomingContactReason::Ineligible);
    assert_eq!(contact.state, IncomingContactState::Rejected);
    f.enroll(3, true);
    assert!(!f
        .receive("retry", "not-enrolled", "never a prompt")
        .unwrap());
    assert_eq!(f.count("messages"), 0);
    assert!(f
        .receive("fresh-event", "fresh-resource", "now enrolled")
        .unwrap());
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn disabled_or_unmounted_history_does_not_route_a_late_reply_to_the_remaining_room() {
    let f = F::new(2, false);
    let mut r = route(&f.s.connection().unwrap(), &f.policies[0].route_id).unwrap();
    r.enabled = false;
    f.s.configure_route(&f.root, &r).unwrap();
    let input = SmsObservation {
        event_id: "late-event",
        event_digest: "late-digest",
        provider_id: "late-resource",
        from: "+15555550101",
        to: "+15555550102",
        body: "late reply",
        occurred_at: None,
    };
    assert!(!f
        .s
        .receive_sms_for_routes(&input, &[f.policies[1].route_id.clone()])
        .unwrap());
    let hold = f.held();
    assert_eq!(hold.reason, IncomingContactReason::HistoricalOverlap);
    assert_eq!(hold.candidates.len(), 1);
    assert_eq!(f.count("messages"), 0);
    f.s.resolve_incoming_sms_contact(
        &f.root,
        "choose-current",
        &hold.id,
        0,
        &hold.candidates[0].id,
    )
    .unwrap();
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn original_audience_is_frozen_and_new_members_do_not_receive_a_held_all_message() {
    let f = F::new(2, true);
    f.receive("event", "resource", "old audience only").unwrap();
    let hold = f.held();
    let chosen = hold
        .candidates
        .iter()
        .find(|c| c.conversation_id == f.rooms[0])
        .unwrap();
    let newcomer = f.s.create_identity("newcomer").unwrap().id;
    f.change(
        &f.rooms[0],
        "new-member",
        MembershipAction::Add {
            identity_id: newcomer.clone(),
            kind: IdentityKind::Human,
            role: "member".into(),
        },
    );
    let resolved =
        f.s.resolve_incoming_sms_contact(&f.root, "resolve-original", &hold.id, 0, &chosen.id)
            .unwrap();
    let message = resolved.decision.unwrap().message_id.unwrap();
    assert!(!crate::audience::visible_to_identity(
        &f.s.connection().unwrap(),
        &newcomer,
        &f.rooms[0],
        ContentKind::Message,
        &message
    )
    .unwrap());
}
#[test]
fn removed_and_readmitted_epoch_cannot_inherit_the_old_candidate() {
    let f = F::new(2, false);
    f.receive("event", "resource", "old epoch text").unwrap();
    let hold = f.held();
    let chosen = hold
        .candidates
        .iter()
        .find(|c| c.conversation_id == f.rooms[0])
        .unwrap();
    let member =
        crate::audience::member(&f.s.connection().unwrap(), &f.peer, &f.rooms[0], true).unwrap();
    f.change(
        &f.rooms[0],
        "remove",
        MembershipAction::Remove {
            participant_id: member.participant_id,
            reason: None,
        },
    );
    f.change(
        &f.rooms[0],
        "readmit",
        MembershipAction::Add {
            identity_id: f.peer.clone(),
            kind: IdentityKind::Human,
            role: "member".into(),
        },
    );
    assert!(f
        .s
        .resolve_incoming_sms_contact(&f.root, "refuse-old", &hold.id, 0, &chosen.id)
        .is_err());
    assert_eq!(f.count("messages"), 0);
    assert_eq!(f.held().state, IncomingContactState::Held);
}
#[test]
fn storage_failure_rolls_back_message_fanout_decision_and_replay_commits_once() {
    let f = F::new(2, false);
    f.receive("event", "resource", "one atomic message")
        .unwrap();
    let hold = f.held();
    let chosen = &hold.candidates[0];
    let before = f.count("journal");
    f.s.connection().unwrap().execute_batch("CREATE TRIGGER fail_sms_resolution BEFORE INSERT ON incoming_contact_deliveries BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(f
        .s
        .resolve_incoming_sms_contact(&f.root, "resolve", &hold.id, 0, &chosen.id)
        .is_err());
    assert_eq!(f.count("messages"), 0);
    assert_eq!(f.count("journal"), before);
    assert_eq!(f.count("incoming_contact_audit"), 1);
    f.s.connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_sms_resolution")
        .unwrap();
    f.s.resolve_incoming_sms_contact(&f.root, "resolve", &hold.id, 0, &chosen.id)
        .unwrap();
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn current_account_delegate_endpoint_and_sms_controls_fence_resolution() {
    for mode in ["delegate", "endpoint", "stop", "account"] {
        let f = F::new(2, false);
        f.receive("event", "resource", "current only").unwrap();
        let hold = f.held();
        let chosen = &hold.candidates[0];
        match mode {
            "delegate" => {
                f.s.revoke(&f.s.revalidate(&f.root).unwrap(), &f.delegate)
                    .unwrap()
            }
            "account" => {
                f.s.revoke(&f.s.revalidate(&f.root).unwrap(), &f.root)
                    .unwrap()
            }
            "endpoint" => {
                f.s.execute_provisioning(
                    &f.root,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "withdraw".into(),
                        endpoint_id: f.policies[0].endpoint_id.clone(),
                        expected_revision: 1,
                    },
                )
                .unwrap();
            }
            _ => {
                f.s.receive_sms_control_for_routes(
                    "stop",
                    "stop-digest",
                    "+15555550101",
                    "+15555550102",
                    "STOP",
                    crate::now(),
                    &f.policies
                        .iter()
                        .map(|p| p.route_id.clone())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            }
        }
        assert!(
            f.s.resolve_incoming_sms_contact(&f.root, "cannot-resolve", &hold.id, 0, &chosen.id)
                .is_err(),
            "{mode}"
        );
        assert_eq!(f.count("messages"), 0);
    }
}

#[test]
fn competing_administrative_connections_choose_one_original_candidate_once() {
    let f = F::new(2, false);
    f.receive("event", "resource", "one decision").unwrap();
    let hold = f.held();
    let path = f.s.path().to_path_buf();
    let results = std::thread::scope(|scope| {
        let mut threads = vec![];
        for (i, candidate) in hold.candidates.iter().enumerate() {
            let path = path.clone();
            let key = f.root.clone();
            let id = hold.id.clone();
            let candidate = candidate.id.clone();
            threads.push(scope.spawn(move || {
                let s = Store::open_administration(&path, Limits::default()).unwrap();
                s.resolve_incoming_sms_contact(&key, &format!("compete-{i}"), &id, 0, &candidate)
            }));
        }
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(f.count("messages"), 1);
    assert_eq!(f.count("incoming_contact_deliveries"), 1);
    assert_eq!(f.count("incoming_contact_audit"), 2);
}
#[test]
fn completed_resolution_replay_rechecks_withdrawn_endpoint_without_rewriting_fact() {
    let f = F::new(2, false);
    f.receive("event", "resource", "completed fact").unwrap();
    let hold = f.held();
    let candidate = &hold.candidates[0];
    let resolved =
        f.s.resolve_incoming_sms_contact(&f.root, "resolve", &hold.id, 0, &candidate.id)
            .unwrap();
    f.s.execute_provisioning(
        &f.root,
        ProvisioningCommand::RevokeEndpoint {
            operation_id: "withdraw-after".into(),
            endpoint_id: f.policies[0].endpoint_id.clone(),
            expected_revision: 1,
        },
    )
    .unwrap();
    assert!(f
        .s
        .resolve_incoming_sms_contact(&f.root, "resolve", &hold.id, 0, &candidate.id)
        .is_err());
    assert_eq!(f.s.incoming_contact(&f.root, &hold.id).unwrap(), resolved);
    assert_eq!(f.count("messages"), 1);
    assert_eq!(f.count("incoming_contact_audit"), 2);
}
#[test]
fn real_restore_keeps_original_sms_resolution_and_does_not_deliver_again() {
    let f = F::new(2, false);
    f.receive("event", "resource", "restored private message")
        .unwrap();
    let hold = f.held();
    let candidate = &hold.candidates[0];
    let resolved =
        f.s.resolve_incoming_sms_contact(&f.root, "resolve", &hold.id, 0, &candidate.id)
            .unwrap();
    let backup = f._tmp.path().join("sms-backup.sqlite3");
    f.s.backup(&backup).unwrap();
    let path = f._tmp.path().join("sms-restored.sqlite3");
    std::fs::copy(&backup, &path).unwrap();
    let restored = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(
        restored
            .resolve_incoming_sms_contact(&f.root, "resolve", &hold.id, 0, &candidate.id)
            .unwrap(),
        resolved
    );
    let input = SmsObservation {
        event_id: "new-callback",
        event_digest: "new-digest",
        provider_id: "resource",
        from: "+15555550101",
        to: "+15555550102",
        body: "restored private message",
        occurred_at: None,
    };
    assert!(!restored
        .receive_sms_for_routes(
            &input,
            &f.policies
                .iter()
                .map(|p| p.route_id.clone())
                .collect::<Vec<_>>()
        )
        .unwrap());
    let c = restored.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM incoming_contact_audit", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
}
