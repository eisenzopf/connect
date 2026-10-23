#![allow(clippy::unwrap_used)]
use crate::{Scope, Store};
use connect_protocol::{
    events::*, provisioning::*, session_control::*, Limits, Operation, Outcome,
};
use serde_json::json;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    ids: Vec<String>,
    keys: Vec<String>,
    pids: Vec<String>,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let owner = store.create_identity("host").unwrap().id;
        let admin = store
            .issue(&owner, Scope::operator(), crate::now() + 3600)
            .unwrap();
        let mut ids = vec![owner];
        for (index, kind) in [
            IdentityKind::Ai,
            IdentityKind::External,
            IdentityKind::Human,
        ]
        .into_iter()
        .enumerate()
        {
            let ProvisioningOutcome::Identity { identity_id, .. } = store
                .execute_provisioning(
                    &admin.id,
                    ProvisioningCommand::Identity {
                        operation_id: format!("identity-{index}"),
                        alias: format!("session-party-{index}"),
                        kind,
                        display_name: None,
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            ids.push(identity_id);
        }
        let keys: Vec<_> = ids
            .iter()
            .map(|id| {
                store
                    .issue(
                        id,
                        Scope::collaboration(vec!["*".into()]),
                        crate::now() + 3600,
                    )
                    .unwrap()
                    .id
            })
            .collect();
        let Outcome::Conversation(conv) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let members = store
            .membership_snapshot(&keys[0], &conv.id)
            .unwrap()
            .participants;
        let pids = ids
            .iter()
            .map(|id| {
                members
                    .iter()
                    .find(|p| &p.identity_id == id)
                    .unwrap()
                    .participant_id
                    .clone()
            })
            .collect();
        Self {
            dir,
            store,
            ids,
            keys,
            pids,
            cid: conv.id,
        }
    }
    fn request(&self, id: &str, expected: Option<u64>, action: Action) -> Request {
        Request {
            operation_id: id.into(),
            conversation_id: self.cid.clone(),
            session_id: "main-session".into(),
            by_participant: None,
            expected_revision: expected,
            action,
        }
    }
    fn invite(&self) -> Request {
        self.request(
            "invite",
            None,
            Action::Invite {
                participants: self.pids[1..].to_vec(),
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["opus","g.711-mu"],"video":["vp8"]}),
            },
        )
    }
    fn apply(&self, actor: usize, request: Request) -> Outcome {
        self.store
            .execute(&self.keys[actor], &Operation::SessionControl { request })
            .unwrap()
    }
    fn accept(&self, actor: usize, revision: u64) -> Outcome {
        self.apply(
            actor,
            self.request(
                &format!("accept-{actor}"),
                Some(revision),
                Action::Accept {
                    capabilities: if actor == 2 {
                        json!({"audio":["g.711-mu"]})
                    } else {
                        json!({"audio":["opus"],"video":["vp8"]})
                    },
                },
            ),
        )
    }
}
fn result(outcome: Outcome) -> ControlOutcome {
    let Outcome::SessionControl(value) = outcome else {
        panic!("Session control receipt")
    };
    value
}

#[test]
fn solo_session_keeps_one_original_participant_and_can_grow_without_rewriting_replay() {
    let f = Fixture::new();
    let mut invite = f.invite();
    let Action::Invite { participants, .. } = &mut invite.action else {
        unreachable!()
    };
    participants.clear();
    let created = f.apply(0, invite.clone());
    let original = result(created.clone());
    assert_eq!(original.session.state, "accepted");
    assert_eq!(original.session.topology_revision, 0);
    let appearances = f
        .store
        .session_presence(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    assert_eq!(appearances.len(), 1);
    assert_eq!(appearances[0].participant_id, f.pids[0]);
    assert_eq!(appearances[0].state, "present");
    assert!(f
        .store
        .session_view(&f.keys[1], &f.cid, "main-session")
        .is_err());
    let c = rusqlite::Connection::open(f.store.path()).unwrap();
    let invitations: u64 = c
        .query_row("SELECT COUNT(*) FROM session_invitation_sources", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(invitations, 0, "a solo Session has no outbound invitation");
    drop(c);
    assert_eq!(f.apply(0, invite.clone()), created);

    // Empty recipients mean initial solo admission, never a fresh topology
    // mutation on an existing Session. Replay still recovers the original.
    let mut empty_existing = invite.clone();
    empty_existing.operation_id = "empty-existing".into();
    empty_existing.expected_revision = Some(0);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert!(f
        .store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: empty_existing
            }
        )
        .is_err());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);

    let mut add = f.invite();
    add.operation_id = "grow-solo".into();
    add.expected_revision = Some(0);
    let Action::Invite { participants, .. } = &mut add.action else {
        unreachable!()
    };
    *participants = vec![f.pids[1].clone()];
    assert_eq!(result(f.apply(0, add)).session.topology_revision, 1);
    assert_eq!(result(f.accept(1, 1)).session.topology_revision, 2);
    let appearances = f
        .store
        .session_presence(&f.keys[1], &f.cid, "main-session")
        .unwrap();
    assert_eq!(appearances.len(), 2);
    assert!(appearances.iter().all(|p| p.state == "present"));
    assert_eq!(f.apply(0, invite.clone()), created);

    let path = f.store.path().to_owned();
    let keys = f.keys.clone();
    let cid = f.cid.clone();
    drop(f.store);
    let reopened = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(
        reopened
            .execute(
                &keys[0],
                &Operation::SessionControl {
                    request: invite.clone()
                }
            )
            .unwrap(),
        created
    );
    assert_eq!(
        reopened
            .session_view(&keys[0], &cid, "main-session")
            .unwrap()
            .topology_revision,
        2
    );
    let end = Request {
        operation_id: "end-grown-solo".into(),
        conversation_id: cid.clone(),
        session_id: "main-session".into(),
        by_participant: None,
        expected_revision: Some(2),
        action: Action::End {
            reason_code: 200,
            reason: "finished".into(),
        },
    };
    assert_eq!(
        result(
            reopened
                .execute(&keys[1], &Operation::SessionControl { request: end })
                .unwrap()
        )
        .session
        .state,
        "ended"
    );
    assert_eq!(
        reopened
            .execute(
                &keys[0],
                &Operation::SessionControl {
                    request: invite.clone()
                }
            )
            .unwrap(),
        created
    );
    invite.operation_id = "cannot-reopen-ended-solo".into();
    invite.expected_revision = Some(3);
    assert!(reopened
        .execute(&keys[0], &Operation::SessionControl { request: invite })
        .is_err());
    assert_eq!(
        reopened
            .session_view(&keys[0], &cid, "main-session")
            .unwrap()
            .state,
        "ended"
    );
}
#[test]
fn public_session_invite_accept_leave_role_and_end_keep_general_presence_and_main_id() {
    let f = Fixture::new();
    let invite = f.invite();
    let created = f.apply(0, invite.clone());
    assert_eq!(f.apply(0, invite.clone()), created);
    assert_eq!(result(created).session.topology_revision, 0);
    for actor in 1..4 {
        assert_eq!(
            result(f.accept(actor, (actor - 1) as u64))
                .session
                .topology_revision,
            actor as u64
        );
    }
    let before = f
        .store
        .session_view(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    assert!(before.is_main);
    assert_eq!(before.state, "accepted");
    let appearances = f
        .store
        .session_presence(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    assert_eq!(appearances.len(), 4);
    assert!(appearances.iter().all(|p| p.state == "present"));
    let role = f.request(
        "role",
        Some(3),
        Action::Role {
            participant_id: f.pids[3].clone(),
            role: "observer".into(),
        },
    );
    let changed = result(f.apply(0, role));
    assert_eq!(changed.session.topology_revision, 4);
    let no_right = f.request(
        "role-label-is-not-authority",
        Some(4),
        Action::Role {
            participant_id: f.pids[0].clone(),
            role: "observer".into(),
        },
    );
    assert_eq!(
        f.store
            .execute(&f.keys[3], &Operation::SessionControl { request: no_right })
            .unwrap_err()
            .code,
        "forbidden"
    );
    let leave = f.request(
        "leave-third",
        Some(4),
        Action::Leave {
            participant_id: f.pids[2].clone(),
        },
    );
    let left = f.apply(2, leave.clone());
    assert_eq!(f.apply(2, leave), left);
    assert!(f
        .store
        .authorize_session_binding(&f.keys[2], "main-session")
        .is_err());
    for actor in [0, 1, 3] {
        f.store
            .authorize_session_binding(&f.keys[actor], "main-session")
            .unwrap();
    }
    assert_eq!(
        f.store
            .session_view(&f.keys[1], &f.cid, "main-session")
            .unwrap()
            .state,
        "accepted"
    );
    let ended = result(f.apply(
        0,
        f.request(
            "end",
            Some(5),
            Action::End {
                reason_code: 42,
                reason: "finished".into(),
            },
        ),
    ));
    assert_eq!(ended.session.state, "ended");
    assert!(ended.session.is_main);
    for key in &f.keys {
        assert!(f
            .store
            .authorize_session_binding(key, "main-session")
            .is_err());
    }
    f.store
        .send_message_as(
            &f.keys[0],
            "conversation-survives",
            &f.cid,
            None,
            connect_protocol::messages::MessageDraft::text_all("still here"),
        )
        .unwrap();
}
#[test]
fn existing_session_invitation_adds_only_explicit_participants_and_reject_does_not_end_others() {
    let f = Fixture::new();
    let mut initial = f.invite();
    if let Action::Invite { participants, .. } = &mut initial.action {
        participants.truncate(1);
    }
    f.apply(0, initial);
    f.accept(1, 0);
    assert!(f
        .store
        .session_view(&f.keys[2], &f.cid, "main-session")
        .is_err());
    let mut invitation = f.invite();
    invitation.operation_id = "invite-additional".into();
    invitation.expected_revision = Some(1);
    if let Action::Invite { participants, .. } = &mut invitation.action {
        *participants = f.pids[2..].to_vec();
    }
    let receipt = f.apply(0, invitation.clone());
    assert_eq!(f.apply(0, invitation), receipt);
    assert_eq!(
        f.store
            .session_presence(&f.keys[0], &f.cid, "main-session")
            .unwrap()
            .iter()
            .filter(|p| p.state == "invited")
            .count(),
        2
    );
    let decline = f.request(
        "reject-third",
        Some(2),
        Action::Reject {
            reason_code: 7,
            reason: "unavailable".into(),
        },
    );
    let Outcome::SessionDecision(decision) = f.apply(2, decline.clone()) else {
        panic!()
    };
    assert_eq!(decision.state, "rejected");
    assert_eq!(f.apply(2, decline), Outcome::SessionDecision(decision));
    assert_eq!(
        f.store
            .session_view(&f.keys[1], &f.cid, "main-session")
            .unwrap()
            .state,
        "accepted"
    );
    f.accept(3, 3);
    f.store
        .authorize_session_binding(&f.keys[3], "main-session")
        .unwrap();
}
#[test]
fn camera_mismatch_stale_revision_spoof_and_changed_intent_preserve_existing_session() {
    let f = Fixture::new();
    let invitation = f.invite();
    f.apply(0, invitation.clone());
    f.accept(2, 0);
    let camera = f.request(
        "unsupported-camera",
        Some(1),
        Action::Upgrade {
            media: vec!["video".into()],
        },
    );
    assert_eq!(
        f.store
            .execute(&f.keys[2], &Operation::SessionControl { request: camera })
            .unwrap_err()
            .code,
        "unsupported-feature"
    );
    let current = f
        .store
        .session_view(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    assert_eq!(current.media, ["audio"]);
    assert_eq!(current.topology_revision, 1);
    let stale = f.request(
        "stale-end",
        Some(0),
        Action::End {
            reason_code: 0,
            reason: "stale".into(),
        },
    );
    assert_eq!(
        f.store
            .execute(&f.keys[0], &Operation::SessionControl { request: stale })
            .unwrap_err()
            .code,
        "conflict"
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let mut spoof = f.request(
        "spoof",
        Some(1),
        Action::Accept {
            capabilities: json!({"audio":["opus"]}),
        },
    );
    spoof.by_participant = Some(f.pids[1].clone());
    assert_eq!(
        f.store
            .execute(&f.keys[3], &Operation::SessionControl { request: spoof })
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let mut changed = invitation;
    if let Action::Invite { intent, .. } = &mut changed.action {
        *intent = "different".into();
    }
    assert_eq!(
        f.store
            .execute(&f.keys[0], &Operation::SessionControl { request: changed })
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .session_view(&f.keys[0], &f.cid, "main-session")
            .unwrap(),
        current
    );
}
#[test]
fn original_session_receipt_and_source_facts_survive_backup_and_capability_storage_failure_recovers(
) {
    let f = Fixture::new();
    let request = f.invite();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    f.store.connection().unwrap().execute_batch("ALTER TABLE session_presence_capabilities RENAME TO temporarily_unavailable_capabilities").unwrap();
    // Isolated missing-table failure rolls back resource, audience, presence and receipt.
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &Operation::SessionControl {
                    request: request.clone()
                }
            )
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    f.store.connection().unwrap().execute_batch("ALTER TABLE temporarily_unavailable_capabilities RENAME TO session_presence_capabilities").unwrap();
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let created = f.apply(0, request.clone());
    f.accept(1, 0);
    let snapshot = f
        .store
        .conversation_snapshot(&f.keys[0], &f.cid, SnapshotQuery::default())
        .unwrap();
    assert!(snapshot
        .items
        .iter()
        .any(|item| matches!(item,SnapshotItem::Session(s)if s.is_main&&s.id=="main-session")));
    let baseline = f
        .store
        .conversation_events(&f.keys[0], &f.cid, EventQuery::default())
        .unwrap();
    let role = f.request(
        "role-fact",
        Some(1),
        Action::Role {
            participant_id: f.pids[1].clone(),
            role: "speaker".into(),
        },
    );
    f.apply(0, role);
    let events = f
        .store
        .conversation_events(
            &f.keys[0],
            &f.cid,
            EventQuery {
                after: Some(baseline.cursor),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(events.events.iter().any(|e|matches!(&e.data,JournalData::SessionControl(s)if s.topology_revision==2&&s.id=="main-session")&&e.session_id.as_deref()==Some("main-session")&&e.operation_id.as_deref()==Some("role-fact")));
    let path = f.dir.path().join("backup.sqlite3");
    f.store.backup(&path).unwrap();
    let restored = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(
        restored
            .execute(&f.keys[0], &Operation::SessionControl { request })
            .unwrap(),
        created
    );
    assert_eq!(
        restored
            .session_view(&f.keys[0], &f.cid, "main-session")
            .unwrap()
            .topology_revision,
        2
    );
}

#[test]
fn current_recipient_ceiling_cannot_end_other_participants_and_unknown_codecs_do_not_admit() {
    let f = Fixture::new();
    let mut unknown = f.invite();
    unknown.operation_id = "unknown-codec".into();
    if let Action::Invite { capabilities, .. } = &mut unknown.action {
        *capabilities = json!({"audio":["uninstalled-codec"]});
    }
    assert_eq!(
        f.store
            .execute(&f.keys[0], &Operation::SessionControl { request: unknown })
            .unwrap_err()
            .code,
        "unsupported-feature"
    );
    assert!(f
        .store
        .session_view(&f.keys[0], &f.cid, "main-session")
        .is_err());
    f.apply(0, f.invite());
    f.accept(1, 0);
    let limited = f
        .store
        .issue(
            &f.ids[0],
            Scope {
                operations: vec!["session.end".into()],
                recipients: vec![f.ids[1].clone()],
            },
            crate::now() + 600,
        )
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute(
                &limited.id,
                &Operation::SessionControl {
                    request: f.request(
                        "bounded-end",
                        Some(1),
                        Action::End {
                            reason_code: 0,
                            reason: "not authorized for everyone".into()
                        }
                    )
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(
        f.store
            .session_view(&f.keys[1], &f.cid, "main-session")
            .unwrap()
            .state,
        "accepted"
    );
}

#[test]
fn parallel_sessions_keep_main_identity_and_roles_while_unconnected_capabilities_change() {
    let f = Fixture::new();
    let first = result(f.apply(0, f.invite()));
    assert!(first.session.is_main);
    let original_role = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.participant_id == f.pids[1])
        .unwrap()
        .role;
    let mut other = f.invite();
    other.operation_id = "parallel-invite".into();
    other.session_id = "parallel-session".into();
    let parallel = result(f.apply(0, other));
    assert!(!parallel.session.is_main);
    let mut accept = f.request(
        "parallel-accept",
        Some(0),
        Action::Accept {
            capabilities: json!({"audio":["opus"]}),
        },
    );
    accept.session_id = "parallel-session".into();
    f.apply(1, accept);
    f.apply(
        0,
        f.request(
            "main-role",
            Some(0),
            Action::Role {
                participant_id: f.pids[1].clone(),
                role: "speaker".into(),
            },
        ),
    );
    let upgraded = result(f.apply(
        0,
        f.request(
            "unconnected-upgrade",
            Some(1),
            Action::Upgrade {
                media: vec!["video".into()],
            },
        ),
    ));
    assert_eq!(upgraded.session.media, ["audio", "video"]);
    assert_eq!(upgraded.session.state, "invited");
    let codecs = result(f.apply(
        0,
        f.request(
            "unconnected-codecs",
            Some(2),
            Action::Renegotiate {
                codec_preferences: vec!["opus".into()],
            },
        ),
    ));
    assert_eq!(codecs.session.codec_preferences, ["opus"]);
    assert_eq!(
        codecs.session.capabilities_offer.as_ref().unwrap()["video"],
        json!(["vp8"])
    );
    f.accept(1, 3);
    assert_eq!(
        f.store
            .session_presence(&f.keys[0], &f.cid, "parallel-session")
            .unwrap()
            .into_iter()
            .find(|p| p.participant_id == f.pids[1])
            .unwrap()
            .role,
        "participant"
    );
    assert_eq!(
        f.store
            .membership_snapshot(&f.keys[0], &f.cid)
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.participant_id == f.pids[1])
            .unwrap()
            .role,
        original_role
    );
    f.apply(
        0,
        f.request(
            "end-main-only",
            Some(4),
            Action::End {
                reason_code: 0,
                reason: "main ended".into(),
            },
        ),
    );
    let parallel = f
        .store
        .session_view(&f.keys[1], &f.cid, "parallel-session")
        .unwrap();
    assert_eq!(parallel.state, "accepted");
    assert_eq!(parallel.media, ["audio"]);
    assert!(!parallel.is_main);
    let mut replacement = f.invite();
    replacement.operation_id = "new-main".into();
    replacement.session_id = "new-main-session".into();
    let created = result(f.apply(0, replacement));
    assert!(created.session.is_main);
    assert!(
        !f.store
            .session_view(&f.keys[0], &f.cid, "main-session")
            .unwrap()
            .is_main
    );
}

#[test]
fn screen_upgrade_requires_original_offer_and_own_explicit_acceptance() {
    for (offered, accepted) in [(true, true), (false, true), (true, false)] {
        let f = Fixture::new();
        let mut invite = f.invite();
        let Action::Invite { capabilities, .. } = &mut invite.action else {
            panic!()
        };
        *capabilities = if offered {
            json!({"audio":["opus"],"video":["vp8"],"screen":["vp8"]})
        } else {
            json!({"audio":["opus"],"video":["vp8"]})
        };
        f.apply(0, invite);
        f.apply(
            1,
            f.request(
                "screen-accept",
                Some(0),
                Action::Accept {
                    capabilities: if accepted {
                        json!({"audio":["opus"],"video":["vp8"],"screen":["vp8"]})
                    } else {
                        json!({"audio":["opus"],"video":["vp8"]})
                    },
                },
            ),
        );
        let request = f.request(
            "screen-upgrade",
            Some(1),
            Action::Upgrade {
                media: vec!["screen".into()],
            },
        );
        let actual = f.store.execute(
            &f.keys[1],
            &Operation::SessionControl {
                request: request.clone(),
            },
        );
        if offered && accepted {
            let outcome = actual.unwrap();
            assert_eq!(
                result(outcome.clone()).session.media,
                vec!["audio", "screen"]
            );
            assert_eq!(f.apply(1, request), outcome);
        } else {
            assert_eq!(actual.unwrap_err().code, "unsupported-feature");
            let session = f
                .store
                .session_view(&f.keys[1], &f.cid, "main-session")
                .unwrap();
            assert_eq!(session.media, vec!["audio"]);
            assert_eq!(session.topology_revision, 1);
        }
    }
}

#[test]
fn mixed_session_can_accept_explicit_screen_without_claiming_audio_or_camera_support() {
    let f = Fixture::new();
    let mut invite = f.invite();
    let Action::Invite {
        medium,
        capabilities,
        ..
    } = &mut invite.action
    else {
        panic!()
    };
    *medium = "mixed".into();
    *capabilities = json!({"screen":["vp8"]});
    f.apply(0, invite);
    let accepted = result(f.apply(
        1,
        f.request(
            "screen-only-accept",
            Some(0),
            Action::Accept {
                capabilities: json!({"screen":["vp8"]}),
            },
        ),
    ));
    assert_eq!(accepted.session.state, "accepted");
    assert_eq!(
        accepted.session.media,
        vec!["audio", "data", "screen", "video"]
    );
    assert_eq!(
        accepted.session.capabilities_offer,
        Some(json!({"screen":["vp8"]}))
    );
}

#[test]
fn canonical_session_context_retains_observed_outcomes_and_source_attribution() {
    let f = Fixture::new();
    let invited = result(f.apply(0, f.invite())).session;
    let mut observed = vec![invited.clone()];
    for actor in 1..4 {
        observed.push(result(f.accept(actor, (actor - 1) as u64)).session);
    }
    let ended = result(f.apply(
        0,
        f.request(
            "end-context-call",
            Some(3),
            Action::End {
                reason_code: 200,
                reason: "completed conversation".into(),
            },
        ),
    ))
    .session;
    let Outcome::ContextView(view) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ContextView {
                conversation_id: f.cid.clone(),
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!("context view")
    };
    let outcomes: Vec<_> = view
        .items
        .iter()
        .filter(|v| v.kind == "session_control")
        .collect();
    assert_eq!(outcomes.len(), 5);
    assert!(outcomes.iter().all(|v| v.source_id == "main-session"
        && v.actor_id == f.ids[0]
        && v.sequence <= view.watermark));
    let snapshots: Vec<SessionView> = outcomes
        .iter()
        .map(|v| serde_json::from_str(&v.body).unwrap())
        .collect();
    assert_eq!(snapshots[0], invited);
    assert_eq!(snapshots.last().unwrap(), &ended);
    observed.push(ended.clone());
    assert_eq!(
        snapshots, observed,
        "later state must not rewrite earlier outcomes"
    );
    assert_eq!(snapshots[0].state, "invited");
    assert_eq!(snapshots.last().unwrap().state, "ended");
    assert_eq!(view.omitted, 0);
    let Outcome::ContextView(tail) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ContextView {
                conversation_id: f.cid.clone(),
                limit: 2,
            },
        )
        .unwrap()
    else {
        panic!("context tail")
    };
    assert_eq!(tail.items.len(), 2);
    assert_eq!(tail.omitted, 3);
    assert_eq!(tail.watermark, view.watermark);
    let path = f.dir.path().join("state.sqlite3");
    drop(f.store);
    let reopened = Store::open(&path, Limits::default()).unwrap();
    let Outcome::ContextView(after_restart) = reopened
        .execute(
            &f.keys[0],
            &Operation::ContextView {
                conversation_id: f.cid.clone(),
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!("context after restart")
    };
    assert_eq!(
        after_restart, view,
        "restart must retain attributed prior outcomes"
    );
}

#[test]
fn canonical_session_context_filters_both_principals_before_limits_and_omissions() {
    let f = Fixture::new();
    f.apply(0, f.invite());
    let mut private = f.invite();
    private.operation_id = "private-context-session".into();
    private.session_id = "owner-human-only".into();
    private.action = Action::Invite {
        participants: vec![f.pids[3].clone()],
        medium: "voice".into(),
        intent: "private consultation".into(),
        capabilities: json!({"audio":["opus"]}),
    };
    f.apply(0, private);
    let narrow = f
        .store
        .issue(
            &f.ids[1],
            Scope::collaboration(vec![f.ids[1].clone()]),
            crate::now() + 3600,
        )
        .unwrap();
    let c = f.store.connection().unwrap();
    let owner = crate::identity::actor_from_connection(&c, &f.keys[0]).unwrap();
    let ai = crate::identity::actor_from_connection(&c, &f.keys[1]).unwrap();
    let shared = crate::context::view_for_scope(
        &f.store,
        &c,
        &owner,
        &f.cid,
        1,
        Some(&ai.identity_id),
        Some(&ai),
    )
    .unwrap();
    assert_eq!(shared.items.len(), 1);
    assert_eq!(shared.items[0].source_id, "main-session");
    assert_eq!(
        shared.omitted, 0,
        "a hidden private outcome must not affect counts"
    );
    let limited = crate::identity::actor_from_connection(&c, &narrow.id).unwrap();
    let denied = crate::context::view_for_scope(
        &f.store,
        &c,
        &owner,
        &f.cid,
        1,
        Some(&ai.identity_id),
        Some(&limited),
    )
    .unwrap();
    assert!(denied.items.is_empty());
    assert_eq!(denied.omitted, 0);
}

#[test]
fn canonical_session_context_delivery_validates_the_exact_historical_journal_body() {
    let f = Fixture::new();
    f.apply(0, f.invite());
    f.accept(1, 0);
    let Outcome::ContextView(view) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ContextView {
                conversation_id: f.cid.clone(),
                limit: 100,
            },
        )
        .unwrap()
    else {
        panic!("context view")
    };
    assert_eq!(view.items.len(), 2);
    let mut forged = view.clone();
    let old: serde_json::Value = serde_json::from_str(&forged.items[0].body).unwrap();
    assert_eq!(old["state"], "invited");
    forged.items[0].body = forged.items[1].body.clone();
    assert_eq!(
        f.store
            .begin_context_delivery(&f.keys[0], "main-session", "vapi", &forged)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut wrong_source = view.clone();
    wrong_source.items[0].source_id = "unrelated-session".into();
    assert!(f
        .store
        .begin_context_delivery(&f.keys[0], "main-session", "vapi", &wrong_source)
        .is_err());
    let mut relabeled = view.clone();
    relabeled.items[0].kind = "session_ready".into();
    relabeled.items[0].body = {
        let c = f.store.connection().unwrap();
        serde_json::to_string(&crate::sessions::get(&c, &f.cid, "main-session").unwrap()).unwrap()
    };
    assert_eq!(
        f.store
            .begin_context_delivery(&f.keys[0], "main-session", "vapi", &relabeled)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let receipt = f
        .store
        .begin_context_delivery(&f.keys[0], "main-session", "vapi", &view)
        .unwrap();
    let c = f.store.connection().unwrap();
    let saved: String = c
        .query_row(
            "SELECT selection_json FROM context_deliveries WHERE id=?1",
            [&receipt],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<connect_protocol::ContextView>(&saved).unwrap(),
        view
    );
}

#[test]
fn ordinary_session_context_keeps_invite_accept_history_and_admits_its_exact_delivery() {
    let f = Fixture::new();
    let Outcome::Session(invited) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::SessionInvite {
                request_id: "context-invite-history".into(),
                conversation_id: f.cid.clone(),
                participant_id: f.ids[1].clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!("invited Session")
    };
    let Outcome::Session(accepted) = f
        .store
        .execute(
            &f.keys[1],
            &Operation::SessionAccept {
                request_id: "context-accept-history".into(),
                conversation_id: f.cid.clone(),
                session_id: invited.id.clone(),
            },
        )
        .unwrap()
    else {
        panic!("accepted Session")
    };
    let view = f
        .store
        .context_for_session(&f.keys[0], &invited.id)
        .unwrap();
    let snapshots: Vec<connect_protocol::Session> = view
        .items
        .iter()
        .filter(|item| item.source_id == invited.id)
        .map(|item| serde_json::from_str(&item.body).unwrap())
        .collect();
    assert_eq!(snapshots, vec![invited.clone(), accepted]);
    assert!(f
        .store
        .begin_context_delivery(&f.keys[0], &invited.id, "vapi", &view)
        .is_ok());
    let mut forged = view.clone();
    forged.items[0].body = forged.items[1].body.clone();
    assert_eq!(
        f.store
            .begin_context_delivery(&f.keys[0], &invited.id, "vapi", &forged)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn terminal_record_intent_preserves_each_explicit_numeric_reason_code() {
    for (kind, code) in [("cancel", 487), ("reject", 486), ("end", 200)] {
        let f = Fixture::new();
        let mut invite = f.invite();
        if let Action::Invite { participants, .. } = &mut invite.action {
            *participants = vec![f.pids[1].clone()];
        }
        f.apply(0, invite);
        let (actor, action) = match kind {
            "cancel" => (
                0,
                Action::Cancel {
                    reason_code: code,
                    reason: "original numeric reason".into(),
                },
            ),
            "reject" => (
                1,
                Action::Reject {
                    reason_code: code,
                    reason: "original numeric reason".into(),
                },
            ),
            _ => {
                f.accept(1, 0);
                (
                    0,
                    Action::End {
                        reason_code: code,
                        reason: "original numeric reason".into(),
                    },
                )
            }
        };
        let revision = f
            .store
            .session_view(&f.keys[0], &f.cid, "main-session")
            .unwrap()
            .topology_revision;
        f.apply(actor, f.request("terminal", Some(revision), action));
        let c = f.store.connection().unwrap();
        let value:u64=c.query_row("SELECT json_extract(snapshot_json,'$.reason_code') FROM session_record_intents WHERE session_id='main-session'",[],|r|r.get(0)).unwrap();
        assert_eq!(value, code);
    }
}

#[test]
fn identity_invitation_resolves_current_participants_without_rewriting_original_intent() {
    let f = Fixture::new();
    let mut request = f.invite();
    if let Action::Invite { participants, .. } = &mut request.action {
        *participants = vec![f.ids[1].clone(), f.pids[2].clone()];
    }
    let first = f.apply(0, request.clone());
    assert_eq!(f.apply(0, request.clone()), first);
    let presence = f
        .store
        .session_presence(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    let targets = presence
        .iter()
        .filter(|p| p.state == "invited")
        .map(|p| p.participant_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        targets,
        std::collections::BTreeSet::from([f.pids[1].clone(), f.pids[2].clone()])
    );
    let saved = crate::operations::receipts::receipt(
        &f.store.connection().unwrap(),
        &f.ids[0],
        &request.operation_id,
    )
    .unwrap()
    .unwrap();
    assert!(
        saved.operation
            == Operation::SessionControl {
                request: request.clone()
            }
    );
    let mut alias = request.clone();
    if let Action::Invite { participants, .. } = &mut alias.action {
        participants[0] = f.pids[1].clone();
    }
    assert_eq!(
        f.store
            .execute(&f.keys[0], &Operation::SessionControl { request: alias })
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(result(f.accept(1, 0)).session.state, "accepted");
    assert_eq!(f.apply(0, request), first);
}
#[test]
fn identity_invitation_cannot_join_unadmitted_members_or_bypass_recipient_ceilings() {
    let f = Fixture::new();
    let outside = f.store.create_identity("not-admitted").unwrap().id;
    let limited = f
        .store
        .issue(
            &f.ids[0],
            Scope::collaboration(vec![f.ids[0].clone()]),
            crate::now() + 300,
        )
        .unwrap();
    for (token, target) in [(&f.keys[0], outside), (&limited.id, f.ids[1].clone())] {
        let mut request = f.invite();
        if let Action::Invite { participants, .. } = &mut request.action {
            *participants = vec![target];
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert_eq!(
            f.store
                .execute(token, &Operation::SessionControl { request })
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}
#[test]
fn invitation_aliases_cannot_duplicate_a_participant_or_assert_the_caller() {
    let f = Fixture::new();
    for (targets, code) in [
        (vec![f.ids[1].clone(), f.pids[1].clone()], "invalid-request"),
        (vec![f.ids[0].clone()], "forbidden"),
    ] {
        let mut request = f.invite();
        if let Action::Invite { participants, .. } = &mut request.action {
            *participants = targets;
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert_eq!(
            f.store
                .execute(&f.keys[0], &Operation::SessionControl { request })
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}

#[test]
fn identity_invitation_uses_only_current_membership_epoch_after_readmission() {
    use connect_protocol::membership::{MembershipAction, MembershipRequest};
    let f = Fixture::new();
    let snapshot = f.store.membership_snapshot(&f.keys[0], &f.cid).unwrap();
    let original = snapshot
        .participants
        .iter()
        .find(|p| p.identity_id == f.ids[1])
        .unwrap()
        .clone();
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "remove-invite-target".into(),
                conversation_id: f.cid.clone(),
                expected_revision: snapshot.revision,
                change: MembershipAction::Remove {
                    participant_id: f.pids[1].clone(),
                    reason: Some("removed before invitation".into()),
                },
            },
        )
        .unwrap();
    for target in [&f.ids[1], &f.pids[1]] {
        let mut request = f.invite();
        if let Action::Invite { participants, .. } = &mut request.action {
            *participants = vec![target.clone()];
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert_eq!(
            f.store
                .execute(&f.keys[0], &Operation::SessionControl { request })
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
    let snapshot = f.store.membership_snapshot(&f.keys[0], &f.cid).unwrap();
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "readmit-invite-target".into(),
                conversation_id: f.cid.clone(),
                expected_revision: snapshot.revision,
                change: MembershipAction::Add {
                    identity_id: f.ids[1].clone(),
                    kind: IdentityKind::Ai,
                    role: "participant".into(),
                },
            },
        )
        .unwrap();
    let mut request = f.invite();
    if let Action::Invite { participants, .. } = &mut request.action {
        *participants = vec![f.ids[1].clone()];
    }
    f.apply(0, request);
    let presence = f
        .store
        .session_presence(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    let target = presence
        .iter()
        .find(|p| p.participant_id == original.participant_id)
        .unwrap();
    assert_eq!(target.state, "invited");
    assert_eq!(target.membership_epoch, original.membership_epoch + 1);
}

#[test]
fn terminal_session_journal_keeps_original_metadata_for_each_authorized_recipient() {
    let f = Fixture::new();
    f.apply(0, f.invite());
    f.accept(1, 0);
    let cursors = f
        .keys
        .iter()
        .map(|key| {
            f.store
                .conversation_snapshot(key, &f.cid, SnapshotQuery::default())
                .unwrap()
                .cursor
        })
        .collect::<Vec<_>>();
    let closed = result(f.apply(
        0,
        f.request(
            "end-original-terminal",
            Some(1),
            Action::End {
                reason_code: 488,
                reason: "session-finished".into(),
            },
        ),
    ));
    let mut facts = Vec::new();
    for (key, cursor) in f.keys.iter().zip(&cursors) {
        let page = f
            .store
            .conversation_events(
                key,
                &f.cid,
                EventQuery {
                    after: Some(cursor.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        let values = page
            .events
            .iter()
            .filter_map(|e| {
                if let JournalData::SessionEnded(fact) = &e.data {
                    assert!(fact.ended_at <= e.observed_at);
                    Some(fact.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 1);
        let fact = values[0].clone();
        assert_eq!(fact.session, closed.session);
        assert_eq!(fact.by_participant.as_deref(), Some(f.pids[0].as_str()));
        assert_eq!(fact.by_membership_epoch, Some(1));
        assert_eq!(fact.reason_code, 488);
        assert_eq!(fact.reason, "session-finished");
        facts.push(fact);
    }
    assert!(facts.windows(2).all(|p| p[0] == p[1]));
    let c = f.store.connection().unwrap();
    assert!(c
        .execute(
            "UPDATE session_terminal_facts SET fact_json='{}' WHERE session_id='main-session'",
            []
        )
        .is_err());
    assert!(c
        .execute(
            "DELETE FROM session_terminal_facts WHERE session_id='main-session'",
            []
        )
        .is_err());
    c.execute(
        "UPDATE sessions SET ended_reason='later current metadata' WHERE id='main-session'",
        [],
    )
    .unwrap();
    drop(c);
    let page = f
        .store
        .conversation_events(
            &f.keys[0],
            &f.cid,
            EventQuery {
                after: Some(cursors[0].clone()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(page
        .events
        .iter()
        .any(|e| matches!(&e.data,JournalData::SessionEnded(fact)if fact==&facts[0])));
    let c = f.store.connection().unwrap();
    c.execute("UPDATE credentials SET operations='[\"conversation.get\",\"conversation.subscribe\"]' WHERE id=?1",[&f.keys[1]]).unwrap();
    drop(c);
    match f.store.conversation_events(
        &f.keys[1],
        &f.cid,
        EventQuery {
            after: Some(cursors[1].clone()),
            ..Default::default()
        },
    ) {
        Ok(page) => assert!(!page
            .events
            .iter()
            .any(|e| matches!(&e.data, JournalData::SessionEnded(_)))),
        Err(e) => assert_eq!(e.code, "forbidden"),
    }
}

fn invitation_sources(f: &Fixture, operation: &str) -> Vec<crate::SessionInvitationSource> {
    let c = f.store.connection().unwrap();
    let record: String = c
        .query_row(
            "SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id=?2",
            rusqlite::params![f.ids[0], operation],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    f.store.session_invitation_sources(&record).unwrap()
}
#[test]
fn invitation_sources_capture_original_targets_and_hold_no_idle_authority() {
    let f = Fixture::new();
    let command = f.invite();
    let first = f.apply(0, command.clone());
    let sources = invitation_sources(&f, "invite");
    assert_eq!(sources.len(), 3);
    let write =
        vec![crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap()];
    let writer = f.store.try_acquire_authority(&write).unwrap();
    drop(writer);
    let source = sources
        .iter()
        .find(|s| s.target_identity_id() == f.ids[1])
        .unwrap();
    let read = f
        .store
        .try_session_invitation_read(source, &f.keys[1])
        .unwrap();
    assert!(read.current());
    assert_eq!(read.identity_id(), f.ids[1]);
    let original = read.metadata().clone();
    assert_eq!(original.from_participant_id, f.pids[0]);
    assert_eq!(original.from_membership_epoch, 1);
    assert_eq!(original.target_participant_id, f.pids[1]);
    assert_eq!(original.target_membership_epoch, 1);
    assert_eq!(original.session_id, "main-session");
    assert_eq!(original.medium, "voice");
    assert_eq!(
        original.capabilities_offer,
        json!({"audio":["opus","g.711-mu"],"video":["vp8"]})
    );
    assert_eq!(
        f.store.try_acquire_authority(&write).unwrap_err().code,
        "authority-busy"
    );
    drop(read);
    let writer = f.store.try_acquire_authority(&write).unwrap();
    assert_eq!(
        f.store
            .try_session_invitation_read(source, &f.keys[1])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(writer);
    assert_eq!(f.apply(0, command), first);
    let replay = invitation_sources(&f, "invite");
    assert_eq!(replay.len(), 3);
    let next = replay
        .iter()
        .find(|s| s.target_identity_id() == f.ids[1])
        .unwrap();
    let read = f
        .store
        .try_session_invitation_read(next, &f.keys[1])
        .unwrap();
    assert_eq!(read.metadata(), &original);
}
#[test]
fn invitation_metadata_requires_current_target_permission_and_source_ceiling() {
    let f = Fixture::new();
    f.apply(0, f.invite());
    let sources = invitation_sources(&f, "invite");
    let source = sources
        .iter()
        .find(|s| s.target_identity_id() == f.ids[1])
        .unwrap();
    assert_eq!(
        f.store
            .try_session_invitation_read(source, &f.keys[2])
            .unwrap_err()
            .code,
        "forbidden"
    );
    for scope in [
        Scope {
            operations: vec!["send".into()],
            recipients: vec!["*".into()],
        },
        Scope {
            operations: vec!["session.accept".into()],
            recipients: vec![f.ids[1].clone()],
        },
    ] {
        let credential = f.store.issue(&f.ids[1], scope, crate::now() + 300).unwrap();
        assert_eq!(
            f.store
                .try_session_invitation_read(source, &credential.id)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let credential = f
        .store
        .issue(
            &f.ids[1],
            Scope {
                operations: vec!["session.reject".into()],
                recipients: vec![f.ids[0].clone(), f.ids[1].clone()],
            },
            crate::now() + 300,
        )
        .unwrap();
    let read = f
        .store
        .try_session_invitation_read(source, &credential.id)
        .unwrap();
    assert!(read.current());
    drop(read);
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET revoked=1 WHERE id=?1",
        [&credential.id],
    )
    .unwrap();
    drop(c);
    assert!(f
        .store
        .try_session_invitation_read(source, &credential.id)
        .is_err());
}
#[test]
fn accepted_and_reinvited_slots_cannot_adopt_an_earlier_invitation_source() {
    let f = Fixture::new();
    f.apply(0, f.invite());
    let sources = invitation_sources(&f, "invite");
    let source = sources
        .iter()
        .find(|s| s.target_identity_id() == f.ids[1])
        .unwrap();
    f.accept(1, 0);
    assert_eq!(
        f.store
            .try_session_invitation_read(source, &f.keys[1])
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.apply(
        1,
        f.request(
            "leave-first-invitation",
            Some(1),
            Action::Leave {
                participant_id: f.pids[1].clone(),
            },
        ),
    );
    let mut next = f.request("reinvite-same-epoch", Some(2), f.invite().action);
    if let Action::Invite { participants, .. } = &mut next.action {
        *participants = vec![f.pids[1].clone()];
    }
    f.apply(0, next);
    let newer = invitation_sources(&f, "reinvite-same-epoch");
    assert_eq!(newer.len(), 1);
    assert_eq!(
        f.store
            .try_session_invitation_read(source, &f.keys[1])
            .unwrap_err()
            .code,
        "forbidden"
    );
    let read = f
        .store
        .try_session_invitation_read(&newer[0], &f.keys[1])
        .unwrap();
    assert_eq!(read.metadata().target_membership_epoch, 1);
}
#[test]
fn removed_and_readmitted_targets_require_a_new_original_invitation_epoch() {
    use connect_protocol::membership::{MembershipAction, MembershipRequest};
    let f = Fixture::new();
    f.apply(0, f.invite());
    let old = invitation_sources(&f, "invite");
    let source = old
        .iter()
        .find(|s| s.target_identity_id() == f.ids[1])
        .unwrap();
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "remove-invited-target".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Remove {
                    participant_id: f.pids[1].clone(),
                    reason: None,
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .try_session_invitation_read(source, &f.keys[1])
            .unwrap_err()
            .code,
        "forbidden"
    );
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "readmit-invited-target".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Add {
                    identity_id: f.ids[1].clone(),
                    kind: IdentityKind::Ai,
                    role: "participant".into(),
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .try_session_invitation_read(source, &f.keys[1])
            .unwrap_err()
            .code,
        "forbidden"
    );
    let current = f
        .store
        .session_view(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    let mut next = f.request(
        "invite-new-target-epoch",
        Some(current.topology_revision),
        f.invite().action,
    );
    if let Action::Invite { participants, .. } = &mut next.action {
        *participants = vec![f.ids[1].clone()];
    }
    f.apply(0, next);
    let new = invitation_sources(&f, "invite-new-target-epoch");
    let read = f
        .store
        .try_session_invitation_read(&new[0], &f.keys[1])
        .unwrap();
    assert_eq!(read.metadata().target_membership_epoch, 2);
    assert_eq!(
        f.store
            .try_session_invitation_read(source, &f.keys[1])
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn invitation_sources_reconstruct_after_backup_but_old_handles_do_not_cross_runtimes() {
    let f = Fixture::new();
    f.apply(0, f.invite());
    let sources = invitation_sources(&f, "invite");
    let source = sources
        .iter()
        .find(|s| s.target_identity_id() == f.ids[1])
        .unwrap();
    let backup = f.dir.path().join("invitation-copy.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(
        restored
            .try_session_invitation_read(source, &f.keys[1])
            .unwrap_err()
            .code,
        "forbidden"
    );
    let c = restored.connection().unwrap();
    let record: String = c
        .query_row(
            "SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id='invite'",
            [&f.ids[0]],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    let fresh = restored.session_invitation_sources(&record).unwrap();
    let target = fresh
        .iter()
        .find(|s| s.target_identity_id() == f.ids[1])
        .unwrap();
    let original = f
        .store
        .try_session_invitation_read(source, &f.keys[1])
        .unwrap();
    let new = restored
        .try_session_invitation_read(target, &f.keys[1])
        .unwrap();
    assert_eq!(original.metadata(), new.metadata());
}
#[test]
fn invitation_capture_rolls_back_with_failed_journal_and_retries_one_original_intent() {
    let f = Fixture::new();
    let request = f.invite();
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER fail_invitation_journal BEFORE INSERT ON journal WHEN NEW.kind='session_control' BEGIN SELECT RAISE(ABORT,'fixture journal storage refusal'); END;").unwrap();
    drop(c);
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &Operation::SessionControl {
                    request: request.clone()
                }
            )
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    let c = f.store.connection().unwrap();
    for table in [
        "session_invitation_sources",
        "session_invitation_targets",
        "session_invitation_slots",
    ] {
        let count: u64 = c
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    c.execute_batch("DROP TRIGGER fail_invitation_journal")
        .unwrap();
    drop(c);
    let first = f.apply(0, request.clone());
    assert_eq!(f.apply(0, request), first);
    assert_eq!(invitation_sources(&f, "invite").len(), 3);
    let c = f.store.connection().unwrap();
    assert!(c
        .execute("UPDATE session_invitation_sources SET intent='changed'", [])
        .is_err());
    assert!(c
        .execute("DELETE FROM session_invitation_targets", [])
        .is_err());
}

#[test]
fn current_session_participant_can_end_with_only_named_permission_and_replay_requires_current_rights(
) {
    let f = Fixture::new();
    f.apply(0, f.invite());
    f.accept(1, 0);
    let end = f.request(
        "participant-end",
        Some(1),
        Action::End {
            reason_code: 200,
            reason: "normal-clearing".into(),
        },
    );
    let issue = |operations: Vec<String>, recipients: Vec<String>| {
        f.store
            .issue(
                &f.ids[1],
                Scope {
                    operations,
                    recipients,
                },
                crate::now() + 3600,
            )
            .unwrap()
            .id
    };
    let missing = issue(vec!["session_get".into()], vec!["*".into()]);
    assert_eq!(
        f.store
            .execute(
                &missing,
                &Operation::SessionControl {
                    request: end.clone()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let narrow = issue(vec!["session.end".into()], vec![f.ids[1].clone()]);
    assert_eq!(
        f.store
            .execute(
                &narrow,
                &Operation::SessionControl {
                    request: end.clone()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let key = issue(vec!["session.end".into()], vec!["*".into()]);
    let mut spoofed = end.clone();
    spoofed.by_participant = Some(f.pids[0].clone());
    assert_eq!(
        f.store
            .execute(&key, &Operation::SessionControl { request: spoofed })
            .unwrap_err()
            .code,
        "forbidden"
    );
    let original = f
        .store
        .execute(
            &key,
            &Operation::SessionControl {
                request: end.clone(),
            },
        )
        .unwrap();
    assert_eq!(result(original.clone()).session.state, "ended");
    assert_eq!(
        f.store
            .execute(
                &key,
                &Operation::SessionControl {
                    request: end.clone()
                }
            )
            .unwrap(),
        original
    );
    let c = f.store.connection().unwrap();
    let ends: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM session_terminal_facts WHERE session_id='main-session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ends, 1);
    c.execute(
        "UPDATE credentials SET operations='[\"session_get\"]' WHERE id=?1",
        [&key],
    )
    .unwrap();
    drop(c);
    assert_eq!(
        f.store
            .execute(&key, &Operation::SessionControl { request: end })
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn invitation_or_departed_session_presence_does_not_grant_whole_session_end() {
    let f = Fixture::new();
    let mut invite = f.invite();
    if let Action::Invite { participants, .. } = &mut invite.action {
        *participants = vec![f.pids[1].clone()];
    }
    f.apply(0, invite);
    let end = f.request(
        "presence-required-end",
        Some(0),
        Action::End {
            reason_code: 200,
            reason: "normal-clearing".into(),
        },
    );
    for actor in [1, 2, 3] {
        assert_eq!(
            f.store
                .execute(
                    &f.keys[actor],
                    &Operation::SessionControl {
                        request: end.clone()
                    }
                )
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    f.accept(1, 0);
    f.apply(
        1,
        f.request(
            "leave-before-end",
            Some(1),
            Action::Leave {
                participant_id: f.pids[1].clone(),
            },
        ),
    );
    let mut end = end;
    end.expected_revision = Some(2);
    assert_eq!(
        f.store
            .execute(&f.keys[1], &Operation::SessionControl { request: end })
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .session_view(&f.keys[0], &f.cid, "main-session")
            .unwrap()
            .state,
        "accepted"
    );
}

#[test]
fn cancellation_requires_unaccepted_session_and_preserves_pending_invitees_after_acceptance() {
    let f = Fixture::new();
    let invited = result(f.apply(0, f.invite()));
    let cancel = f.request(
        "cancel-before-accept",
        None,
        Action::Cancel {
            reason_code: 487,
            reason: "request-cancelled".into(),
        },
    );
    let cancelled = f.apply(0, cancel.clone());
    let Outcome::SessionDecision(decision) = &cancelled else {
        panic!("cancellation receipt")
    };
    assert_eq!(decision.state, "cancelled");
    assert_eq!(f.apply(0, cancel), cancelled);
    assert_eq!(decision.revision, invited.session.topology_revision + 1);
    assert_eq!(
        f.store
            .execute(
                &f.keys[1],
                &Operation::SessionControl {
                    request: f.request(
                        "late-accept",
                        Some(1),
                        Action::Accept {
                            capabilities: json!({"audio":["opus"]})
                        }
                    ),
                }
            )
            .unwrap_err()
            .code,
        "conflict"
    );

    let f = Fixture::new();
    f.apply(0, f.invite());
    let accepted = result(f.accept(1, 0));
    let presence = f
        .store
        .session_presence(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    assert!(presence
        .iter()
        .any(|p| p.participant_id == f.pids[2] && p.state == "invited"));
    let cancel = f.request(
        "cancel-after-accept",
        Some(accepted.session.topology_revision),
        Action::Cancel {
            reason_code: 487,
            reason: "request-cancelled".into(),
        },
    );
    assert_eq!(
        f.store
            .execute(&f.keys[0], &Operation::SessionControl { request: cancel })
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .session_view(&f.keys[0], &f.cid, "main-session")
            .unwrap(),
        accepted.session
    );
    assert_eq!(
        f.store
            .session_presence(&f.keys[0], &f.cid, "main-session")
            .unwrap(),
        presence
    );
    let ended = result(f.apply(
        0,
        f.request(
            "end-after-accept",
            Some(accepted.session.topology_revision),
            Action::End {
                reason_code: 200,
                reason: "normal-clear".into(),
            },
        ),
    ));
    assert_eq!(ended.session.state, "ended");
}

#[test]
fn cancellation_freezes_only_pending_original_targets_and_replay_cannot_rewrite_the_hint() {
    let f = Fixture::new();
    f.apply(0, f.invite());
    let sources = invitation_sources(&f, "invite");
    f.apply(
        1,
        f.request(
            "decline-before-cancel",
            Some(0),
            Action::Reject {
                reason_code: 486,
                reason: "busy".into(),
            },
        ),
    );
    let cancel = f.request(
        "freeze-cancel",
        Some(1),
        Action::Cancel {
            reason_code: 487,
            reason: "request-cancelled".into(),
        },
    );
    let original = f.apply(0, cancel.clone());
    assert_eq!(f.apply(0, cancel), original);
    for source in sources {
        let hint = f
            .store
            .session_invitation_cancellation_template(&source)
            .unwrap();
        if source.target_identity_id() == f.ids[1] {
            assert!(hint.is_none());
        } else {
            let hint = hint.unwrap();
            assert_eq!(hint.by_participant_id, f.pids[0]);
            assert_eq!(hint.by_membership_epoch, 1);
            assert_eq!(hint.reason_code, 487);
            assert_eq!(hint.reason, "request-cancelled");
        }
    }
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM session_invitation_cancellations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert!(c
        .execute(
            "UPDATE session_invitation_cancellations SET reason='changed'",
            []
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM session_invitation_cancellations", [])
        .is_err());
}
#[test]
fn cancellation_hint_storage_failure_rolls_back_the_session_and_recovers_the_same_logical_command()
{
    let f = Fixture::new();
    f.apply(0, f.invite());
    let sources = invitation_sources(&f, "invite");
    let before = f
        .store
        .session_view(&f.keys[0], &f.cid, "main-session")
        .unwrap();
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER cancel_hint_failure BEFORE INSERT ON session_invitation_cancellations BEGIN SELECT RAISE(ABORT,'isolated cancellation storage failure'); END;").unwrap();
    drop(c);
    let cancel = f.request(
        "recover-cancel-hint",
        Some(0),
        Action::Cancel {
            reason_code: 487,
            reason: "request-cancelled".into(),
        },
    );
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &Operation::SessionControl {
                    request: cancel.clone()
                }
            )
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(
        f.store
            .session_view(&f.keys[0], &f.cid, "main-session")
            .unwrap(),
        before
    );
    for source in &sources {
        assert!(f
            .store
            .session_invitation_cancellation_template(source)
            .unwrap()
            .is_none());
    }
    let c = f.store.connection().unwrap();
    c.execute_batch("DROP TRIGGER cancel_hint_failure").unwrap();
    drop(c);
    let original = f.apply(0, cancel.clone());
    assert_eq!(f.apply(0, cancel), original);
    for source in &sources {
        assert!(f
            .store
            .session_invitation_cancellation_template(source)
            .unwrap()
            .is_some());
    }
}
