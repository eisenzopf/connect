//! Public store membership, current audience, retry and recovery behavior.
#![allow(clippy::unwrap_used)]
use connect_protocol::{membership::*, provisioning::IdentityKind, Limits, Operation, Outcome};
use connect_store::{Scope, Store};
use rusqlite::{params, Connection};
use sha2::Digest;
use std::sync::{Arc, Barrier};

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    ids: Vec<String>,
    keys: Vec<String>,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let mut ids = vec![];
        let mut keys = vec![];
        for alias in ["alice", "bob", "carol", "agent"] {
            let id = store.create_identity(alias).unwrap().id;
            let key = store
                .issue(
                    &id,
                    Scope::collaboration(vec!["*".into()]),
                    chrono::Utc::now().timestamp() + 3600,
                )
                .unwrap()
                .id;
            ids.push(id);
            keys.push(key);
        }
        let Outcome::Conversation(c) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![ids
                        [1]
                    .clone()]),
                },
            )
            .unwrap()
        else {
            panic!("conversation")
        };
        Self {
            _dir: dir,
            store,
            ids,
            keys,
            cid: c.id,
        }
    }
    fn c(&self) -> Connection {
        let c = Connection::open(self.store.path()).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        c
    }
    fn revision(&self) -> u64 {
        self.c()
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                [&self.cid],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn request(&self, op: &str, change: MembershipAction) -> MembershipRequest {
        MembershipRequest {
            operation_id: op.into(),
            conversation_id: self.cid.clone(),
            expected_revision: self.revision(),
            change,
        }
    }
    fn apply(&self, who: usize, op: &str, change: MembershipAction) -> MembershipOutcome {
        self.store
            .execute_membership(&self.keys[who], self.request(op, change))
            .unwrap()
    }
    fn invite(&self, who: usize, op: &str) -> String {
        let MembershipOutcome::Invitation { invitation, .. } = self.apply(
            0,
            op,
            MembershipAction::Invite {
                identity_id: self.ids[who].clone(),
                kind: IdentityKind::Human,
                role: "member".into(),
                expires_at: chrono::Utc::now().timestamp() + 600,
            },
        ) else {
            panic!("invitation")
        };
        invitation.invitation_id
    }
    fn join(&self, who: usize, op: &str) -> MemberView {
        let invitation_id = self.invite(who, &format!("invite-{op}"));
        let MembershipOutcome::Participant { participant, .. } =
            self.apply(who, op, MembershipAction::Join { invitation_id })
        else {
            panic!("participant")
        };
        participant
    }
    fn member(&self, who: usize) -> MemberView {
        self.store
            .membership_snapshot(&self.keys[0], &self.cid)
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.identity_id == self.ids[who])
            .unwrap()
    }
    fn send(&self, who: usize, op: &str, body: &str) -> String {
        let Outcome::Accepted { message_id, .. } = self
            .store
            .execute(
                &self.keys[who],
                &Operation::Send {
                    request_id: op.into(),
                    conversation_id: self.cid.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all(body),
                },
            )
            .unwrap()
        else {
            panic!("message")
        };
        message_id
    }
    fn history(&self, who: usize) -> Vec<connect_protocol::Message> {
        let Outcome::History { messages, .. } = self
            .store
            .execute(
                &self.keys[who],
                &Operation::History {
                    conversation_id: self.cid.clone(),
                    after: None,
                    limit: 100,
                },
            )
            .unwrap()
        else {
            panic!("history")
        };
        messages
    }
    fn context(&self, who: usize) -> connect_protocol::ContextView {
        let Outcome::ContextView(v) = self
            .store
            .execute(
                &self.keys[who],
                &Operation::ContextView {
                    conversation_id: self.cid.clone(),
                    limit: 100,
                },
            )
            .unwrap()
        else {
            panic!("context")
        };
        v
    }
    fn share(&self, op: &str, resources: Vec<ContentRef>, target: &str) -> String {
        let MembershipOutcome::Sharing { sharing_id, .. } = self.apply(
            0,
            op,
            MembershipAction::ShareHistory {
                resources,
                participants: vec![target.into()],
                provenance: "Explicit local fixture consent".into(),
            },
        ) else {
            panic!("share")
        };
        sharing_id
    }
    fn source(&self, kind: ContentKind, id: &str) -> ContentRef {
        ContentRef {
            kind,
            id: id.into(),
        }
    }
    fn session(&self, op: &str, who: usize) -> connect_protocol::Session {
        let Outcome::Session(s) = self
            .store
            .execute(
                &self.keys[0],
                &Operation::SessionInvite {
                    request_id: op.into(),
                    conversation_id: self.cid.clone(),
                    participant_id: self.ids[who].clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!("session")
        };
        self.store
            .execute(
                &self.keys[who],
                &Operation::SessionAccept {
                    request_id: format!("accept-{op}"),
                    conversation_id: self.cid.clone(),
                    session_id: s.id.clone(),
                },
            )
            .unwrap();
        s
    }
    fn private_message(&self, id: &str, body: &str) {
        let p = self.member(0);
        let c = self.c();
        c.execute("INSERT INTO messages(id,conversation_id,sender,body,audience_mode,sender_participant_id) VALUES(?1,?2,?3,?4,'explicit',?5)",params![id,self.cid,self.ids[0],body,p.participant_id]).unwrap();
        c.execute(
            "INSERT INTO message_audiences VALUES(?1,?2,?3,?4,'sender')",
            params![id, self.cid, p.participant_id, p.membership_epoch],
        )
        .unwrap();
        // Even an erroneously broad hint cannot expand the accepted audience.
        for recipient in &self.ids[..2] {
            c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,message_id) VALUES(?1,?2,?3,'message',?4)",params![uuid::Uuid::new_v4().to_string(),recipient,self.cid,id]).unwrap();
        }
    }
}
#[test]
fn late_join_requires_explicit_share_and_revocation_removes_every_projection() {
    let f = Fixture::new();
    let old = f.send(0, "old", "BEFORE-JOIN");
    let carol = f.join(2, "join-carol");
    assert!(f.history(2).is_empty());
    assert!(f.context(2).items.is_empty());
    assert_eq!(f.context(2).omitted, 0);
    assert_eq!(
        f.store
            .authorize_content(&f.keys[2], &f.cid, &f.source(ContentKind::Message, &old))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let original: u64 = f
        .c()
        .query_row(
            "SELECT COUNT(*) FROM message_audiences WHERE message_id=?1",
            [&old],
            |r| r.get(0),
        )
        .unwrap();
    let sharing = f.share(
        "share",
        vec![f.source(ContentKind::Message, &old)],
        &carol.participant_id,
    );
    assert_eq!(f.history(2)[0].id, old);
    assert_eq!(f.context(2).items[0].body, "BEFORE-JOIN");
    assert_eq!(
        f.c()
            .query_row(
                "SELECT COUNT(*) FROM message_audiences WHERE message_id=?1",
                [&old],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        original
    );
    f.apply(
        0,
        "revoke",
        MembershipAction::RevokeHistoryShare {
            sharing_id: sharing,
        },
    );
    assert!(f.history(2).is_empty());
    let view = f.context(2);
    assert!(view.items.is_empty());
    assert_eq!(view.omitted, 0);
    f.send(0, "new", "AFTER-JOIN");
    assert_eq!(
        f.history(2)
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        vec!["AFTER-JOIN"]
    );
}

#[test]
fn removal_and_readmission_fence_old_resources_receipts_and_journal_epochs() {
    let f = Fixture::new();
    let old = f.send(1, "bob-old", "OLD-EPOCH");
    let bob = f.member(1);
    f.apply(
        0,
        "remove",
        MembershipAction::Remove {
            participant_id: bob.participant_id.clone(),
            reason: None,
        },
    );
    for op in [
        Operation::History {
            conversation_id: f.cid.clone(),
            after: None,
            limit: 100,
        },
        Operation::ContextView {
            conversation_id: f.cid.clone(),
            limit: 100,
        },
        Operation::RequestStatus {
            request_id: "bob-old".into(),
        },
        Operation::Send {
            request_id: "bob-old".into(),
            conversation_id: f.cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all("OLD-EPOCH"),
        },
    ] {
        assert_eq!(
            f.store.execute(&f.keys[1], &op).unwrap_err().code,
            "forbidden"
        );
    }
    let next = f.join(1, "rejoin");
    assert_eq!(next.participant_id, bob.participant_id);
    assert_eq!(next.membership_epoch, 2);
    assert!(f.history(1).is_empty());
    assert!(f.context(1).items.is_empty());
    assert!(f
        .store
        .authorize_content(&f.keys[1], &f.cid, &f.source(ContentKind::Message, &old))
        .is_err());
    assert!(f
        .store
        .execute(
            &f.keys[1],
            &Operation::RequestStatus {
                request_id: "bob-old".into()
            }
        )
        .is_err());
    f.send(1, "fresh", "NEW-EPOCH");
    assert_eq!(f.history(1).len(), 1);
}

#[test]
fn roles_do_not_grant_scope_or_rewrite_session_snapshots() {
    let f = Fixture::new();
    let session = f.session("voice", 1);
    let bob = f.member(1);
    assert_eq!(f.c().execute("UPDATE session_participants SET role='listener' WHERE session_id=?1 AND conversation_id=?2 AND participant_id=?3 AND membership_epoch=1 AND state='present'",params![session.id,f.cid,bob.participant_id]).unwrap(), 1);
    f.apply(
        0,
        "promote-label",
        MembershipAction::UpdateRole {
            participant_id: bob.participant_id.clone(),
            role: "admin".into(),
        },
    );
    assert_eq!(f.member(1).role, "admin");
    assert_eq!(
        f.c()
            .query_row(
                "SELECT role FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=1",
                params![session.id,bob.participant_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "listener"
    );
    let key = f
        .store
        .issue(
            &f.ids[1],
            Scope {
                operations: vec!["history".into()],
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let denied = f.request(
        "role-isnt-scope",
        MembershipAction::Remove {
            participant_id: f.member(0).participant_id,
            reason: None,
        },
    );
    assert_eq!(
        f.store
            .execute_membership(&key.id, denied)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn invitation_target_cancellation_expiry_and_removing_inviter_are_authoritative() {
    let f = Fixture::new();
    let invitation = f.invite(2, "invite");
    let request = f.request(
        "join",
        MembershipAction::Join {
            invitation_id: invitation.clone(),
        },
    );
    assert_eq!(
        f.store
            .execute_membership(&f.keys[3], request.clone())
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .membership_invitations(&f.keys[2], None, 10)
            .unwrap()
            .invitations
            .len(),
        1
    );
    assert!(f
        .store
        .membership_invitations(&f.keys[3], None, 10)
        .unwrap()
        .invitations
        .is_empty());
    f.apply(
        2,
        "decline",
        MembershipAction::Decline {
            invitation_id: invitation.clone(),
        },
    );
    assert!(f.store.execute_membership(&f.keys[2], request).is_err());
    let pending = f.invite(2, "second");
    f.c()
        .execute(
            "UPDATE membership_invitations SET expires_at=1 WHERE id=?1",
            [&pending],
        )
        .unwrap();
    assert!(f
        .store
        .membership_invitations(&f.keys[2], None, 10)
        .unwrap()
        .invitations
        .is_empty());
    assert!(f
        .store
        .execute_membership(
            &f.keys[2],
            f.request(
                "expired",
                MembershipAction::Join {
                    invitation_id: pending
                }
            )
        )
        .is_err());
    let fresh = f.invite(2, "third");
    f.apply(
        1,
        "remove-inviter",
        MembershipAction::Remove {
            participant_id: f.member(0).participant_id,
            reason: None,
        },
    );
    assert!(f
        .store
        .execute_membership(
            &f.keys[2],
            f.request(
                "stale-invite",
                MembershipAction::Join {
                    invitation_id: fresh
                }
            )
        )
        .is_err());
}

#[test]
fn membership_receipts_replay_current_state_conflict_on_intent_and_rollback_atomically() {
    let f = Fixture::new();
    let request = f.request(
        "intent",
        MembershipAction::Invite {
            identity_id: f.ids[2].clone(),
            kind: IdentityKind::Human,
            role: "guest".into(),
            expires_at: chrono::Utc::now().timestamp() + 600,
        },
    );
    f.c().execute_batch("CREATE TRIGGER deny_membership_receipt BEFORE INSERT ON operations BEGIN SELECT RAISE(ABORT,'fixture disk rejection'); END").unwrap();
    let before = f.revision();
    assert_eq!(
        f.store
            .execute_membership(&f.keys[0], request.clone())
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(f.revision(), before);
    assert_eq!(
        f.c()
            .query_row("SELECT COUNT(*) FROM membership_invitations", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    f.c()
        .execute_batch("DROP TRIGGER deny_membership_receipt")
        .unwrap();
    let first = f
        .store
        .execute_membership(&f.keys[0], request.clone())
        .unwrap();
    assert_eq!(
        f.store
            .execute_membership(&f.keys[0], request.clone())
            .unwrap(),
        first
    );
    let mut changed = request.clone();
    if let MembershipAction::Invite { role, .. } = &mut changed.change {
        *role = "changed".into();
    }
    assert_eq!(
        f.store
            .execute_membership(&f.keys[0], changed)
            .unwrap_err()
            .code,
        "conflict"
    );
    let MembershipOutcome::Invitation { invitation, .. } = first else {
        panic!("invitation")
    };
    f.apply(
        0,
        "cancel",
        MembershipAction::CancelInvitation {
            invitation_id: invitation.invitation_id,
        },
    );
    let MembershipOutcome::Invitation { invitation, .. } =
        f.store.execute_membership(&f.keys[0], request).unwrap()
    else {
        panic!("receipt")
    };
    assert_eq!(invitation.state, "cancelled");
}

#[test]
fn concurrent_handles_serialize_expected_membership_revision() {
    let f = Fixture::new();
    let second = Arc::new(Store::open(f.store.path(), Limits::default()).unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let requests = [
        f.request(
            "race-1",
            MembershipAction::UpdateRole {
                participant_id: f.member(1).participant_id,
                role: "first".into(),
            },
        ),
        f.request(
            "race-2",
            MembershipAction::UpdateRole {
                participant_id: f.member(1).participant_id,
                role: "second".into(),
            },
        ),
    ];
    let handles = [f.store.clone(), second]
        .into_iter()
        .zip(requests)
        .map(|(store, request)| {
            let b = barrier.clone();
            let key = f.keys[0].clone();
            std::thread::spawn(move || {
                b.wait();
                store.execute_membership(&key, request)
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter_map(|r| r.as_ref().err())
            .next()
            .unwrap()
            .code,
        "conflict"
    );
}

#[test]
fn private_sources_do_not_enter_history_inbox_counts_or_model_exports() {
    let f = Fixture::new();
    let session = f.session("voice", 1);
    f.private_message("msg_secret", "PRIVATE-CANARY");
    f.send(0, "public", "PUBLIC-FACT");
    assert!(f
        .context(0)
        .items
        .iter()
        .any(|i| i.body == "PRIVATE-CANARY"));
    assert!(!f.history(1).iter().any(|m| m.body == "PRIVATE-CANARY"));
    assert!(!f
        .context(1)
        .items
        .iter()
        .any(|i| i.body == "PRIVATE-CANARY"));
    let Outcome::Inbox { events, .. } = f
        .store
        .execute(
            &f.keys[1],
            &Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 0,
            },
        )
        .unwrap()
    else {
        panic!("inbox")
    };
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("PRIVATE-CANARY"));
    let selected = f
        .store
        .context_for_session(&f.keys[0], &session.id)
        .unwrap();
    assert!(!serde_json::to_string(&selected)
        .unwrap()
        .contains("PRIVATE-CANARY"));
    assert_eq!(selected.omitted, 0);
    let prepared = f
        .store
        .prepare_vapi_session_context(&f.keys[0], &session.id)
        .unwrap();
    assert!(!prepared.json.contains("PRIVATE-CANARY"));
    let mut forged = selected.clone();
    forged.items[0].body = "FORGED".into();
    assert_eq!(
        f.store
            .begin_context_delivery(&f.keys[0], &session.id, "vapi", &forged)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .begin_context_delivery(&f.keys[0], &session.id, "vapi", &selected)
        .unwrap();
}

#[test]
fn current_authority_matrix_covers_all_source_kinds_and_preserves_other_members() {
    let mut f = Fixture::new();
    for who in 0..4 {
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("record_get".into());
        f.keys[who] = f
            .store
            .issue(&f.ids[who], scope, chrono::Utc::now().timestamp() + 600)
            .unwrap()
            .id;
    }
    let message = f.send(0, "fact", "ORIGINAL");
    let session = f.session("session", 1);
    let Outcome::Artifact(artifact) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ArtifactBegin {
                to: connect_protocol::messages::MessageAudience::all(),
                request_id: "file".into(),
                conversation_id: f.cid.clone(),
                name: "fixture.txt".into(),
                media_type: "text/plain".into(),
                size: 5,
                sha256: hex::encode(sha2::Sha256::digest(b"hello")),
                expires_at: None,
            },
        )
        .unwrap()
    else {
        panic!("artifact")
    };
    f.store
        .execute(
            &f.keys[0],
            &Operation::ArtifactAppend {
                request_id: "append".into(),
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                data: "aGVsbG8=".into(),
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[0],
            &Operation::ArtifactFinish {
                request_id: "finish".into(),
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .unwrap();
    let mut sources = vec![
        f.source(ContentKind::Message, &message),
        f.source(ContentKind::Artifact, &artifact.id),
        f.source(ContentKind::Session, &session.id),
    ];
    // The policy is also used by later record/call writers. These six source
    // rows qualify only the shared admission hook, not those writers or media.
    let alice = f.member(0);
    let bob = f.member(1);
    for kind in [
        ContentKind::Call,
        ContentKind::SipLeg,
        ContentKind::ContextDelivery,
        ContentKind::Record,
        ContentKind::Transcript,
        ContentKind::Observation,
    ] {
        let id = format!("fixture:{}", kind.as_str());
        let c = f.c();
        // SIP and context references carry their original stored revision in
        // journal facts. Seed those rows as well as the admission hook; a bare
        // audience reference is not an existing versioned resource.
        match kind {
            ContentKind::SipLeg => {
                c.execute(
                    "INSERT INTO sip_legs(id,conversation_id,session_id,credential_id,actor_id,destination,created_at,updated_at) VALUES(?1,?2,?3,(SELECT id FROM credentials WHERE identity_id=?4 LIMIT 1),?4,'sip:fixture@invalid',0,0)",
                    params![id, f.cid, session.id, f.ids[0]],
                ).unwrap();
                c.execute("UPDATE sip_legs SET state='active' WHERE id=?1", [&id])
                    .unwrap();
            }
            ContentKind::ContextDelivery => {
                c.execute(
                    "INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at) VALUES(?1,?2,?3,'fixture',0,'{}','pending',0,0)",
                    params![id, f.cid, session.id],
                ).unwrap();
                c.execute(
                    "UPDATE context_deliveries SET state='accepted' WHERE id=?1",
                    [&id],
                )
                .unwrap();
            }
            _ => {}
        }
        c.execute(
            "INSERT OR IGNORE INTO audience_resources VALUES(?1,?2,?3,?4)",
            params![kind.as_str(), id, f.cid, f.ids[0]],
        )
        .unwrap();
        for member in [&alice, &bob] {
            c.execute(
                "INSERT OR IGNORE INTO resource_audiences VALUES(?1,?2,?3,?4,?5)",
                params![
                    kind.as_str(),
                    id,
                    f.cid,
                    member.participant_id,
                    member.membership_epoch
                ],
            )
            .unwrap();
        }
        sources.push(f.source(kind, &id));
    }
    let carol = f.join(2, "late-join");
    for source in &sources {
        assert!(
            f.store
                .authorize_content(&f.keys[0], &f.cid, source)
                .is_ok(),
            "owner {:?}",
            source.kind
        );
        assert!(
            f.store
                .authorize_content(&f.keys[1], &f.cid, source)
                .is_ok(),
            "original {:?}",
            source.kind
        );
        assert_eq!(
            f.store
                .authorize_content(&f.keys[2], &f.cid, source)
                .unwrap_err()
                .code,
            "forbidden",
            "late {:?}",
            source.kind
        );
        assert_eq!(
            f.store
                .authorize_content(&f.keys[2], "unknown", source)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let sharing = f.share(
        "explicit-nine-kinds",
        sources.clone(),
        &carol.participant_id,
    );
    for kind in [ContentKind::SipLeg, ContentKind::ContextDelivery] {
        let (revision, provenance): (u64, String) = f.c().query_row(
            "SELECT resource_revision,json_extract(source_json,'$.kind') FROM journal WHERE recipient=?1 AND json_extract(data_json,'$.source_kind')=?2 ORDER BY sequence DESC LIMIT 1",
            params![f.ids[2], kind.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(revision, 1, "original {kind:?} resource version");
        assert_eq!(provenance, "history-share");
    }
    for source in &sources {
        assert!(
            f.store
                .authorize_content(&f.keys[2], &f.cid, source)
                .is_ok(),
            "shared {:?}",
            source.kind
        );
    }
    let file_read = Operation::ArtifactGet {
        conversation_id: f.cid.clone(),
        artifact_id: artifact.id.clone(),
        offset: 0,
        limit: 5,
    };
    let Outcome::ArtifactChunk(chunk) = f.store.execute(&f.keys[2], &file_read).unwrap() else {
        panic!("chunk")
    };
    assert_eq!(chunk.data, "aGVsbG8=");
    assert!(f
        .store
        .execute(
            &f.keys[2],
            &Operation::SessionGet {
                conversation_id: f.cid.clone(),
                session_id: session.id
            }
        )
        .is_ok());
    f.apply(
        0,
        "remove-bob",
        MembershipAction::Remove {
            participant_id: bob.participant_id,
            reason: None,
        },
    );
    for source in &sources {
        assert!(
            f.store
                .authorize_content(&f.keys[1], &f.cid, source)
                .is_err(),
            "removed {:?}",
            source.kind
        );
        assert!(
            f.store
                .authorize_content(&f.keys[0], &f.cid, source)
                .is_ok(),
            "unrelated {:?}",
            source.kind
        );
    }
    f.join(1, "readmit-bob");
    for source in &sources {
        assert!(
            f.store
                .authorize_content(&f.keys[1], &f.cid, source)
                .is_err(),
            "new epoch {:?}",
            source.kind
        );
    }
    f.apply(
        0,
        "revoke-nine-kinds",
        MembershipAction::RevokeHistoryShare {
            sharing_id: sharing,
        },
    );
    for source in &sources {
        assert!(
            f.store
                .authorize_content(&f.keys[2], &f.cid, source)
                .is_err(),
            "revoked share {:?}",
            source.kind
        );
    }
    assert!(f.store.execute(&f.keys[2], &file_read).is_err());
    let sharing = f.share("share-again", sources.clone(), &carol.participant_id);
    let operator = f
        .store
        .issue(
            &f.ids[0],
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store
        .revoke(&f.store.authenticate(&operator.token).unwrap(), &f.keys[2])
        .unwrap();
    for source in &sources {
        assert!(
            f.store
                .authorize_content(&f.keys[2], &f.cid, source)
                .is_err(),
            "revoked credential {:?}",
            source.kind
        );
    }
    assert!(!sharing.is_empty());
}

#[test]
fn sharing_requires_source_read_scope_current_recipient_ceiling_and_same_conversation() {
    let f = Fixture::new();
    let source = f.send(0, "fact", "PRIVATE");
    let carol = f.join(2, "join");
    let request = f.request(
        "share",
        MembershipAction::ShareHistory {
            resources: vec![f.source(ContentKind::Message, &source)],
            participants: vec![carol.participant_id],
            provenance: "Consent".into(),
        },
    );
    for scope in [
        Scope {
            operations: vec!["conversation.history.share".into()],
            recipients: vec!["*".into()],
        },
        Scope {
            operations: vec!["conversation.history.share".into(), "history".into()],
            recipients: vec![],
        },
    ] {
        let key = f
            .store
            .issue(&f.ids[0], scope, chrono::Utc::now().timestamp() + 600)
            .unwrap();
        assert_eq!(
            f.store
                .execute_membership(&key.id, request.clone())
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let Outcome::Conversation(other) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Create {
                request_id: "other".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    f.ids[2].clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    assert_eq!(
        f.store
            .authorize_content(
                &f.keys[2],
                &other.id,
                &f.source(ContentKind::Message, &source)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    // A membership label does not make a cross-tenant row a valid participant.
    f.c()
        .execute_batch("INSERT INTO tenants(id) VALUES('tnt_other');")
        .unwrap();
    f.c()
        .execute(
            "UPDATE identities SET tenant_id='tnt_other' WHERE id=?1",
            [&f.ids[3]],
        )
        .unwrap();
    let denied = f.request(
        "cross-tenant",
        MembershipAction::Invite {
            identity_id: f.ids[3].clone(),
            kind: IdentityKind::Human,
            role: "admin".into(),
            expires_at: chrono::Utc::now().timestamp() + 600,
        },
    );
    assert_eq!(
        f.store
            .execute_membership(&f.keys[0], denied)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let admin = f
        .store
        .issue(
            &f.ids[3],
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    assert!(f.store.membership_snapshot(&admin.id, &f.cid).is_err());
    assert!(f
        .store
        .authorize_content(&admin.id, &f.cid, &f.source(ContentKind::Message, &source))
        .is_err());
}

#[test]
fn queued_provider_dispatch_rechecks_original_epoch_after_removal_and_readmission() {
    for who in [0, 1] {
        let f = Fixture::new();
        let operator = f
            .store
            .issue(
                &f.ids[0],
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        f.store
            .configure_route(
                &operator.id,
                &connect_store::Route {
                    id: "vapi".into(),
                    identity_id: f.ids[1].clone(),
                    credential_id: f.keys[1].clone(),
                    conversation_id: f.cid.clone(),
                    connector: "vapi".into(),
                    target: "12345678-1234-1234-1234-123456789012".into(),
                    source: None,
                    enabled: true,
                    incoming_to: connect_protocol::messages::MessageAudience::all(),
                },
            )
            .unwrap();
        f.send(0, "queued", "DO-NOT-DISPATCH");
        let removed = f.member(who);
        f.apply(
            1 - who,
            "remove",
            MembershipAction::Remove {
                participant_id: removed.participant_id,
                reason: None,
            },
        );
        let inviter = 1 - who;
        let MembershipOutcome::Invitation { invitation, .. } = f.apply(
            inviter,
            "invite-back",
            MembershipAction::Invite {
                identity_id: f.ids[who].clone(),
                kind: IdentityKind::Human,
                role: "agent".into(),
                expires_at: chrono::Utc::now().timestamp() + 600,
            },
        ) else {
            panic!("invite")
        };
        f.apply(
            who,
            "join-back",
            MembershipAction::Join {
                invitation_id: invitation.invitation_id,
            },
        );
        assert!(f.store.claim_provider_job().unwrap().is_none());
        assert_eq!(
            f.c()
                .query_row("SELECT state FROM provider_jobs", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "cancelled"
        );
        assert_eq!(
            f.c()
                .query_row("SELECT COUNT(*) FROM context_deliveries", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn opaque_provider_memory_cannot_outlive_audience_changes_or_narrower_credentials() {
    let mut f = Fixture::new();
    let admin = f
        .store
        .issue(
            &f.ids[0],
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store
        .configure_route(
            &admin.id,
            &connect_store::Route {
                id: "vapi".into(),
                identity_id: f.ids[1].clone(),
                credential_id: f.keys[1].clone(),
                conversation_id: f.cid.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    f.private_message("private-source", "OWNER-ONLY-CANARY");
    let old = f.send(0, "first", "PUBLIC-QUESTION");
    let first = f.store.claim_provider_job().unwrap().unwrap();
    assert!(!first
        .context
        .as_ref()
        .unwrap()
        .contains("OWNER-ONLY-CANARY"));
    f.store
        .finish_provider_job(&first, "completed", Some("chat-first"), None)
        .unwrap();
    f.send(0, "same-policy", "Next question");
    let second = f.store.claim_provider_job().unwrap().unwrap();
    assert_eq!(second.previous_chat_id.as_deref(), Some("chat-first"));
    f.store
        .finish_provider_job(&second, "completed", Some("chat-second"), None)
        .unwrap();
    let carol = f.join(2, "join");
    f.send(0, "changed-policy", "New audience");
    let changed = f.store.claim_provider_job().unwrap().unwrap();
    assert!(changed.previous_chat_id.is_none());
    // A completion after policy changed retains the policy at dispatch, not a
    // newly blessed reference to remote memory from before the change.
    let sharing = f.share(
        "share",
        vec![f.source(ContentKind::Message, &old)],
        &carol.participant_id,
    );
    f.store
        .finish_provider_job(&changed, "completed", Some("stale-chat"), None)
        .unwrap();
    f.send(0, "after-callback", "Question");
    let after = f.store.claim_provider_job().unwrap().unwrap();
    assert!(after.previous_chat_id.is_none());
    f.store
        .finish_provider_job(&after, "completed", Some("current-chat"), None)
        .unwrap();
    f.apply(
        0,
        "revoke",
        MembershipAction::RevokeHistoryShare {
            sharing_id: sharing,
        },
    );
    f.send(0, "after-revoke", "Question");
    let revoked = f.store.claim_provider_job().unwrap().unwrap();
    assert!(revoked.previous_chat_id.is_none());
    f.store
        .finish_provider_job(&revoked, "completed", Some("post-revoke-chat"), None)
        .unwrap();
    let mut narrow = Scope::collaboration(vec!["*".into()]);
    narrow.operations.retain(|op| op != "context_view");
    f.keys[0] = f
        .store
        .issue(&f.ids[0], narrow, chrono::Utc::now().timestamp() + 600)
        .unwrap()
        .id;
    f.send(0, "narrow", "No history authority");
    let narrow = f.store.claim_provider_job().unwrap().unwrap();
    assert!(narrow.context.is_none());
    assert!(narrow.previous_chat_id.is_none());
}

#[test]
fn compound_source_events_respect_each_peer_audience_and_each_credential_ceiling() {
    for owner in [0, 1] {
        let f = Fixture::new();
        let session = f.session("voice", 1);
        let message = f.send(0, "public", "PUBLIC");
        let Outcome::Artifact(artifact) = f
            .store
            .execute(
                &f.keys[owner],
                &Operation::ArtifactBegin {
                    to: connect_protocol::messages::MessageAudience::all(),
                    request_id: "artifact".into(),
                    conversation_id: f.cid.clone(),
                    name: "ARTIFACT-CANARY".into(),
                    media_type: "text/plain".into(),
                    size: 1,
                    sha256: "0".repeat(64),
                    expires_at: None,
                },
            )
            .unwrap()
        else {
            panic!("artifact")
        };
        if owner == 0 {
            f.c()
                .execute(
                    "DELETE FROM artifact_audiences WHERE artifact_id=?1 AND participant_id=?2",
                    params![artifact.id, f.member(1).participant_id],
                )
                .unwrap();
        }
        let c = f.c();
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,message_id,artifact_json) VALUES('compound',?1,?2,'message',?3,?4)",params![f.ids[0],f.cid,message,serde_json::to_string(&artifact).unwrap()]).unwrap();
        let sequence = c.last_insert_rowid() as u64;
        if owner == 0 {
            let selected = f
                .store
                .context_for_session(&f.keys[0], &session.id)
                .unwrap();
            assert!(!selected.items.iter().any(|i| i.sequence == sequence));
            assert_eq!(selected.omitted, 0);
        } else {
            let narrow = f
                .store
                .issue(
                    &f.ids[0],
                    Scope::communications(vec![]),
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap();
            let Outcome::ContextView(v) = f
                .store
                .execute(
                    &narrow.id,
                    &Operation::ContextView {
                        conversation_id: f.cid.clone(),
                        limit: 100,
                    },
                )
                .unwrap()
            else {
                panic!("context")
            };
            assert!(!v.items.iter().any(|i| i.sequence == sequence));
            assert_eq!(v.omitted, 0);
            let Outcome::Inbox { events, .. } = f
                .store
                .execute(
                    &narrow.id,
                    &Operation::Inbox {
                        cursor: None,
                        limit: 100,
                        wait_ms: 0,
                    },
                )
                .unwrap()
            else {
                panic!("inbox")
            };
            assert!(!events.iter().any(|e| e.id == "compound"));
        }
    }
}

#[test]
fn removal_fences_only_the_targets_current_presence_and_preserves_other_sessions() {
    let f = Fixture::new();
    f.join(2, "join");
    let bob_session = f.session("bob-session", 1);
    let carol_session = f.session("carol-session", 2);
    for (who, session) in [(1, &bob_session), (2, &carol_session)] {
        let p = f.member(who);
        let c = f.c();
        let device = format!("dev_{who}");
        let endpoint = format!("ep_{who}");
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,'tnt_default','native','fixture')",params![device,f.ids[who]]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES(?1,?2,'tnt_default',?3,'quic',?4,'{}','verified','proof',100)",params![endpoint,f.ids[who],device,format!("fixture-{who}")]).unwrap();
        let present: u64 = c.query_row(
            "SELECT count(*) FROM session_participants WHERE session_id=?1 AND conversation_id=?2 AND participant_id=?3 AND membership_epoch=1 AND state='present'",
            params![session.id, f.cid, p.participant_id], |r|r.get(0),
        ).unwrap();
        assert_eq!(
            present, 1,
            "the public Session accept already established presence"
        );
        c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,readiness_json) VALUES(?1,?2,?3,1,?4,'tnt_default',?5,1,'active','{}')",params![format!("conn_{who}"),session.id,p.participant_id,f.ids[who],endpoint]).unwrap();
    }
    f.apply(
        0,
        "remove",
        MembershipAction::Remove {
            participant_id: f.member(1).participant_id,
            reason: None,
        },
    );
    let c = f.c();
    let removed: (String, u64, Option<String>) = c
        .query_row(
            "SELECT state,generation,readiness_json FROM connections WHERE id='conn_1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(removed, ("retired".into(), 2, None));
    let other: (String, u64) = c
        .query_row(
            "SELECT state,generation FROM connections WHERE id='conn_2'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(other, ("active".into(), 1));
    assert!(f
        .store
        .authorize_session_binding(&f.keys[1], &bob_session.id)
        .is_err());
    assert!(f
        .store
        .authorize_session_binding(&f.keys[2], &carol_session.id)
        .is_ok());
}

#[test]
fn membership_sharing_backup_preserves_policy_and_schema20_is_refused_unchanged() {
    let f = Fixture::new();
    let source = f.send(0, "fact", "RESTORED-FACT");
    let carol = f.join(2, "join");
    f.share(
        "share",
        vec![f.source(ContentKind::Message, &source)],
        &carol.participant_id,
    );
    let backup = f._dir.path().join("backup.sqlite3");
    let before = f.store.backup(&backup).unwrap();
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), before);
    let restored_path = f._dir.path().join("restored.sqlite3");
    std::fs::copy(&backup, &restored_path).unwrap();
    let restored = Store::open(&restored_path, Limits::default()).unwrap();
    assert!(restored
        .authorize_content(&f.keys[2], &f.cid, &f.source(ContentKind::Message, &source))
        .is_ok());
    let c = Connection::open(&restored_path).unwrap();
    c.execute("UPDATE history_shares SET active=0", []).unwrap();
    assert_ne!(
        Store::inspect_snapshot(&restored_path).unwrap().digest,
        before.digest
    );
    assert!(restored
        .authorize_content(&f.keys[2], &f.cid, &f.source(ContentKind::Message, &source))
        .is_err());
    let previous = f._dir.path().join("schema20.sqlite3");
    std::fs::copy(&backup, &previous).unwrap();
    let old = Connection::open(&previous).unwrap();
    old.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA user_version=20")
        .unwrap();
    drop(old);
    let bytes = std::fs::read(&previous).unwrap();
    assert_eq!(
        Store::open(&previous, Limits::default())
            .err()
            .unwrap()
            .code,
        "unsupported-schema"
    );
    assert_eq!(std::fs::read(&previous).unwrap(), bytes);
}

#[test]
fn expiry_during_membership_lock_wait_creates_no_invitation_receipt_or_event() {
    let f = Fixture::new();
    let expires = chrono::Utc::now().timestamp() + 2;
    let key = f
        .store
        .issue(&f.ids[0], Scope::collaboration(vec!["*".into()]), expires)
        .unwrap();
    let request = f.request(
        "expiry",
        MembershipAction::Invite {
            identity_id: f.ids[2].clone(),
            kind: IdentityKind::Human,
            role: "member".into(),
            expires_at: expires + 600,
        },
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let c = f.c();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let store = f.store.clone();
    let running = std::thread::spawn(move || store.execute_membership(&key.id, request));
    while chrono::Utc::now().timestamp() < expires {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    c.execute_batch("COMMIT").unwrap();
    assert_eq!(running.join().unwrap().unwrap_err().code, "unauthorized");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn voice_selection_rechecks_revoked_shares_after_preparation_and_history_cannot_resume_presence() {
    let f = Fixture::new();
    let carol = f.join(2, "join-carol");
    // A private source belongs to Alice's epoch; Bob gets an explicit history grant.
    f.private_message("selected-private", "REVOCABLE-SELECTION");
    let bob = f.member(1);
    let sharing = f.share(
        "share-private",
        vec![f.source(ContentKind::Message, "selected-private")],
        &bob.participant_id,
    );
    let session = f.session("voice", 1);
    let selected = f
        .store
        .prepare_vapi_session_context(&f.keys[0], &session.id)
        .unwrap();
    assert!(selected.json.contains("REVOCABLE-SELECTION"));
    f.store
        .revalidate_vapi_session_context(&f.keys[0], &session.id, &selected.delivery_id)
        .unwrap();
    f.apply(
        0,
        "revoke",
        MembershipAction::RevokeHistoryShare {
            sharing_id: sharing,
        },
    );
    assert_eq!(
        f.store
            .revalidate_vapi_session_context(&f.keys[0], &session.id, &selected.delivery_id)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let fresh = f
        .store
        .prepare_vapi_session_context(&f.keys[0], &session.id)
        .unwrap();
    assert!(!fresh.json.contains("REVOCABLE-SELECTION"));
    f.store
        .revalidate_vapi_session_context(&f.keys[0], &session.id, &fresh.delivery_id)
        .unwrap();
    f.apply(
        0,
        "remove",
        MembershipAction::Remove {
            participant_id: bob.participant_id,
            reason: None,
        },
    );
    let current = f.join(1, "readmit");
    f.share(
        "session-history",
        vec![f.source(ContentKind::Session, &session.id)],
        &current.participant_id,
    );
    assert!(f
        .store
        .execute(
            &f.keys[1],
            &Operation::SessionGet {
                conversation_id: f.cid.clone(),
                session_id: session.id.clone()
            }
        )
        .is_ok());
    assert!(f
        .store
        .authorize_session_binding(&f.keys[1], &session.id)
        .is_err());
    assert!(f
        .store
        .prepare_vapi_session_context(&f.keys[0], &session.id)
        .is_err());
    assert!(f
        .store
        .revalidate_vapi_session_context(&f.keys[0], &session.id, &fresh.delivery_id)
        .is_err());
    assert!(f
        .store
        .execute(
            &f.keys[1],
            &Operation::SessionEnd {
                request_id: "cannot-end-old-presence".into(),
                conversation_id: f.cid.clone(),
                session_id: session.id,
                reason: None
            }
        )
        .is_err());
    assert!(carol.active);
}

#[test]
fn voice_context_honors_the_configured_delegate_credential_ceiling() {
    let f = Fixture::new();
    let session = f.session("voice", 1);
    f.send(0, "owner-fact", "OWNER-FACT");
    f.send(1, "peer-fact", "PEER-FACT");
    let admin = f
        .store
        .issue(
            &f.ids[0],
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let delegate = f
        .store
        .issue(
            &f.ids[1],
            Scope::communications(vec![]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store
        .configure_route(
            &admin.id,
            &connect_store::Route {
                id: "vapi".into(),
                identity_id: f.ids[1].clone(),
                credential_id: delegate.id.clone(),
                conversation_id: f.cid.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let selected = f
        .store
        .prepare_vapi_session_context(&f.keys[0], &session.id)
        .unwrap();
    assert!(!selected.json.contains("OWNER-FACT"));
    assert!(selected.json.contains("PEER-FACT"));
    let view = f
        .store
        .context_for_session(&f.keys[0], &session.id)
        .unwrap();
    assert_eq!(view.omitted, 0);
    assert_eq!(view.items.len(), 1);
    f.store
        .revalidate_vapi_session_context(&f.keys[0], &session.id, &selected.delivery_id)
        .unwrap();
    f.store
        .revoke(&f.store.authenticate(&admin.token).unwrap(), &delegate.id)
        .unwrap();
    assert!(f
        .store
        .revalidate_vapi_session_context(&f.keys[0], &session.id, &selected.delivery_id)
        .is_err());
}

#[test]
fn queued_voice_and_sip_effects_are_not_restored_by_readmission_and_history_sharing() {
    for (sip, claimed, who) in [
        (false, false, 0),
        (false, true, 0),
        (false, false, 1),
        (false, true, 1),
        (true, false, 0),
        (true, true, 0),
        (true, false, 1),
        (true, true, 1),
    ] {
        let f = Fixture::new();
        let admin = f
            .store
            .issue(
                &f.ids[0],
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut voice_job = None;
        let mut sip_job = None;
        let source = if sip {
            let s = f.session("voice", 1);
            let Outcome::SipLeg(leg) = f
                .store
                .execute(
                    &f.keys[0],
                    &Operation::SipDial {
                        request_id: "dial".into(),
                        conversation_id: f.cid.clone(),
                        session_id: s.id,
                        destination: "sip:fixture@127.0.0.1:5099".into(),
                    },
                )
                .unwrap()
            else {
                panic!("leg")
            };
            if claimed {
                sip_job = f.store.claim_sip_leg_job().unwrap();
                assert!(sip_job.is_some());
            }
            f.source(ContentKind::SipLeg, &leg.id)
        } else {
            f.store
                .configure_voice_route(
                    &admin.id,
                    &connect_store::VoiceRoute {
                        id: "voice".into(),
                        conversation_id: f.cid.clone(),
                        identity_id: f.ids[1].clone(),
                        assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                        phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                        destinations: vec!["+15555550101".into()],
                        enabled: true,
                    },
                )
                .unwrap();
            let Outcome::Call(call) = f
                .store
                .execute(
                    &f.keys[0],
                    &Operation::CallStart {
                        request_id: "dial".into(),
                        conversation_id: f.cid.clone(),
                        route_id: "voice".into(),
                        destination: "+15555550101".into(),
                    },
                )
                .unwrap()
            else {
                panic!("call")
            };
            if claimed {
                voice_job = f.store.claim_voice_job().unwrap();
                assert!(voice_job.is_some());
            }
            f.source(ContentKind::Call, &call.id)
        };
        let inviter = 1 - who;
        f.apply(
            inviter,
            "remove",
            MembershipAction::Remove {
                participant_id: f.member(who).participant_id,
                reason: None,
            },
        );
        let MembershipOutcome::Invitation { invitation, .. } = f.apply(
            inviter,
            "invite-back",
            MembershipAction::Invite {
                identity_id: f.ids[who].clone(),
                kind: IdentityKind::Human,
                role: "member".into(),
                expires_at: chrono::Utc::now().timestamp() + 600,
            },
        ) else {
            panic!("invite")
        };
        let MembershipOutcome::Participant { participant, .. } = f.apply(
            who,
            "readmit",
            MembershipAction::Join {
                invitation_id: invitation.invitation_id,
            },
        ) else {
            panic!("participant")
        };
        f.apply(
            inviter,
            "history",
            MembershipAction::ShareHistory {
                resources: vec![source.clone()],
                participants: vec![participant.participant_id],
                provenance: "Historical metadata only".into(),
            },
        );
        assert!(f
            .store
            .authorize_content(&f.keys[who], &f.cid, &source)
            .is_ok());
        if let Some(job) = voice_job {
            assert!(!f.store.begin_voice_dispatch(&job).unwrap().is_some());
        } else if let Some(job) = sip_job {
            assert!(!f.store.sip_dispatch_allowed(&job).unwrap());
        } else if sip {
            assert!(f.store.claim_sip_leg_job().unwrap().is_none());
        } else {
            assert!(f.store.claim_voice_job().unwrap().is_none());
        }
    }
}
