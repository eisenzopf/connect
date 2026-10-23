//! Addressed fanout, replay, references and provider selection through public APIs.
#![allow(clippy::unwrap_used)]
use connect_protocol::{
    membership::*, messages::*, provisioning::IdentityKind, Limits, Operation, Outcome,
};
use connect_store::{Route, Scope, Store};
use rusqlite::Connection;
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
        for alias in ["alice", "bob", "carol", "dave"] {
            let id = store.create_identity(alias).unwrap().id;
            keys.push(
                store
                    .issue(
                        &id,
                        Scope::collaboration(vec!["*".into()]),
                        chrono::Utc::now().timestamp() + 3600,
                    )
                    .unwrap()
                    .id,
            );
            ids.push(id);
        }
        let Outcome::Conversation(c) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..3].to_vec(),
                    ),
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
    fn participant(&self, n: usize) -> String {
        self.store
            .membership_snapshot(&self.keys[0], &self.cid)
            .unwrap()
            .participants
            .into_iter()
            .find(|m| m.identity_id == self.ids[n])
            .unwrap()
            .participant_id
    }
    fn to(&self, ns: &[usize]) -> MessageAudience {
        MessageAudience::Participants(ns.iter().map(|n| self.participant(*n)).collect())
    }
    fn op(&self, id: &str, message: MessageDraft) -> Operation {
        Operation::Send {
            request_id: id.into(),
            conversation_id: self.cid.clone(),
            message,
        }
    }
    fn send(&self, n: usize, id: &str, message: MessageDraft) -> String {
        let Outcome::Accepted { message_id, .. } = self
            .store
            .execute(&self.keys[n], &self.op(id, message))
            .unwrap()
        else {
            panic!("accepted")
        };
        message_id
    }
    fn history(&self, n: usize) -> Vec<connect_protocol::Message> {
        let Outcome::History { messages, .. } = self
            .store
            .execute(
                &self.keys[n],
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
    fn membership(&self, n: usize, id: &str, change: MembershipAction) -> MembershipOutcome {
        let revision = self
            .store
            .membership_snapshot(&self.keys[0], &self.cid)
            .unwrap()
            .revision;
        self.store
            .execute_membership(
                &self.keys[n],
                MembershipRequest {
                    operation_id: id.into(),
                    conversation_id: self.cid.clone(),
                    expected_revision: revision,
                    change,
                },
            )
            .unwrap()
    }
    fn join(&self, n: usize) {
        let MembershipOutcome::Invitation { invitation, .. } = self.membership(
            0,
            "invite",
            MembershipAction::Invite {
                identity_id: self.ids[n].clone(),
                kind: IdentityKind::Human,
                role: "member".into(),
                expires_at: chrono::Utc::now().timestamp() + 600,
            },
        ) else {
            panic!("invite")
        };
        self.membership(
            n,
            "join",
            MembershipAction::Join {
                invitation_id: invitation.invitation_id,
            },
        );
    }
    fn bind(&self, n: usize, to: MessageAudience) {
        let operator = self
            .store
            .issue(
                &self.ids[0],
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        self.store
            .configure_route(
                &operator.id,
                &Route {
                    id: format!("route{n}"),
                    identity_id: self.ids[n].clone(),
                    credential_id: self.keys[n].clone(),
                    conversation_id: self.cid.clone(),
                    connector: "vapi".into(),
                    target: "12345678-1234-1234-1234-123456789012".into(),
                    source: None,
                    enabled: true,
                    incoming_to: to,
                },
            )
            .unwrap();
    }
    fn count(&self, table: &str) -> u64 {
        assert!([
            "messages",
            "journal",
            "operation_control_receipts",
            "provider_jobs"
        ]
        .contains(&table));
        self.c()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
#[test]
fn one_many_all_are_frozen_and_offline_replay_never_refans_out() {
    let f = Fixture::new();
    let private = f.send(0, "one", MessageDraft::text(f.to(&[1]), "private"));
    assert_eq!(f.history(0).len(), 1);
    assert_eq!(f.history(1)[0].id, private);
    assert!(f.history(2).is_empty());
    let many = f.send(0, "many", MessageDraft::text(f.to(&[1, 2]), "selected"));
    let all = f.op("all", MessageDraft::text_all("everyone now"));
    let receipt = f.store.execute(&f.keys[0], &all).unwrap();
    let before = f.count("journal");
    f.join(3);
    let after_join = f.count("journal");
    assert!(after_join >= before);
    assert_eq!(f.store.execute(&f.keys[0], &all).unwrap(), receipt);
    assert_eq!(f.count("journal"), after_join);
    assert!(f.history(3).is_empty());
    assert_eq!(
        f.history(2)
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            many.as_str(),
            match &receipt {
                Outcome::Accepted { message_id, .. } => message_id.as_str(),
                _ => panic!(),
            }
        ]
    );
    let backup = f._dir.path().join("backup.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(restored.execute(&f.keys[0], &all).unwrap(), receipt);
    let Outcome::Inbox { events, .. } = restored
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
        panic!()
    };
    assert!(events
        .iter()
        .any(|e| e.message.as_ref().is_some_and(|m| m.id == private)));
}
#[test]
fn narrowed_sender_can_address_one_peer_and_changed_intent_cannot_reuse_receipt() {
    let f = Fixture::new();
    let key = f
        .store
        .issue(
            &f.ids[0],
            Scope::messaging(vec![f.ids[1].clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let op = f.op("limited", MessageDraft::text(f.to(&[1]), "only bob"));
    let result = f.store.execute(&key.id, &op).unwrap();
    assert_eq!(f.store.execute(&key.id, &op).unwrap(), result);
    for message in [
        MessageDraft::text(f.to(&[1]), "changed"),
        MessageDraft::text(f.to(&[2]), "only bob"),
    ] {
        assert_eq!(
            f.store
                .execute(&f.keys[0], &f.op("limited", message))
                .unwrap_err()
                .code,
            "conflict"
        );
    }
    assert_eq!(
        f.store
            .execute(
                &key.id,
                &f.op("all-denied", MessageDraft::text_all("too broad"))
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn binary_metadata_and_order_independent_recipient_sets_round_trip() {
    let f = Fixture::new();
    let mut draft = MessageDraft::text(f.to(&[2, 1]), "AP9hAA==");
    draft.body_encoding = BodyEncoding::Base64;
    draft.content_type = "application/octet-stream".into();
    draft.label = "binary".into();
    draft.msg_id = Some("chosen-message".into());
    let op = f.op("binary", draft.clone());
    let receipt = f.store.execute(&f.keys[0], &op).unwrap();
    if let MessageAudience::Participants(ids) = &mut draft.to {
        ids.reverse();
    }
    assert_eq!(
        f.store.execute(&f.keys[0], &f.op("binary", draft)).unwrap(),
        receipt
    );
    let message = &f.history(1)[0];
    assert_eq!(message.body, "AP9hAA==");
    assert_eq!(message.content_type, "application/octet-stream");
    assert_eq!(message.body_encoding, BodyEncoding::Base64);
    assert_eq!(message.label, "binary");
    assert!(message.accepted_at.is_some());
    assert_eq!(message.audience.len(), 3);
    assert_eq!(
        f.store
            .execute(
                &f.keys[1],
                &f.op("collision", {
                    let mut v = MessageDraft::text(f.to(&[0]), "different");
                    v.msg_id = Some("chosen-message".into());
                    v
                })
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let mut invalid = MessageDraft::text(f.to(&[1]), "AP9hA");
    invalid.body_encoding = BodyEncoding::Base64;
    assert_eq!(
        f.store
            .execute(&f.keys[0], &f.op("malformed", invalid))
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn private_reply_cannot_widen_and_revoked_parent_reference_is_hidden() {
    let f = Fixture::new();
    let question = f.send(
        0,
        "question",
        MessageDraft::text(f.to(&[1]), "private question"),
    );
    let mut reply = MessageDraft::text(f.to(&[0, 2]), "leak");
    reply.in_reply_to_msg = Some(question.clone());
    assert_eq!(
        f.store
            .execute(&f.keys[1], &f.op("widen", reply))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut reply = MessageDraft::text(f.to(&[0]), "private answer");
    reply.in_reply_to_msg = Some(question.clone());
    let answer = f.send(1, "answer", reply);
    assert_eq!(
        f.history(0)[1].in_reply_to_msg.as_deref(),
        Some(question.as_str())
    );
    let MembershipOutcome::Sharing { sharing_id, .. } = f.membership(
        1,
        "share-answer",
        MembershipAction::ShareHistory {
            participants: vec![f.participant(2)],
            resources: vec![ContentRef {
                kind: ContentKind::Message,
                id: answer.clone(),
            }],
            provenance: "explicit reply disclosure".into(),
        },
    ) else {
        panic!("share")
    };
    let carol = f.history(2);
    assert_eq!(carol.len(), 1);
    assert_eq!(carol[0].id, answer);
    assert_eq!(carol[0].in_reply_to_msg, None);
    f.membership(
        1,
        "revoke-share",
        MembershipAction::RevokeHistoryShare { sharing_id },
    );
    assert!(f.history(2).is_empty());
    assert_eq!(f.count("messages"), 2);
}
#[test]
fn failure_of_one_journal_recipient_rolls_back_every_message_write_and_retry_is_safe() {
    let f = Fixture::new();
    let before = (
        f.count("messages"),
        f.count("journal"),
        f.count("operation_control_receipts"),
    );
    let c = f.c();
    c.execute_batch(&format!("CREATE TRIGGER reject_test_message BEFORE INSERT ON journal WHEN NEW.kind='message' AND NEW.recipient='{}' BEGIN SELECT RAISE(FAIL,'fixture'); END",f.ids[1])).unwrap();
    let op = f.op("atomic", MessageDraft::text_all("one transaction"));
    assert_eq!(
        f.store.execute(&f.keys[0], &op).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(
        (
            f.count("messages"),
            f.count("journal"),
            f.count("operation_control_receipts")
        ),
        before
    );
    c.execute_batch("DROP TRIGGER reject_test_message").unwrap();
    f.store.execute(&f.keys[0], &op).unwrap();
    assert_eq!(f.count("messages"), 1);
    assert_eq!(f.history(2).len(), 1);
}
#[test]
fn simultaneous_matching_sends_create_one_message_and_one_fanout() {
    let f = Fixture::new();
    let barrier = Arc::new(Barrier::new(3));
    let op = f.op("concurrent", MessageDraft::text(f.to(&[1]), "once"));
    let handles = (0..2)
        .map(|_| {
            let store = f.store.clone();
            let key = f.keys[0].clone();
            let barrier = barrier.clone();
            let op = op.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.execute(&key, &op).unwrap()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results[0], results[1]);
    assert_eq!(f.count("messages"), 1);
    assert_eq!(f.history(1).len(), 1);
    assert!(f.history(2).is_empty());
}
#[test]
fn selected_provider_only_receives_private_send_and_answer_goes_only_to_question_owner() {
    let f = Fixture::new();
    f.bind(1, f.to(&[0]));
    f.bind(2, f.to(&[0]));
    let question = f.send(
        0,
        "private",
        MessageDraft::text(f.to(&[1]), "for one assistant"),
    );
    assert_eq!(f.count("provider_jobs"), 1);
    let job = f.store.claim_provider_job().unwrap().unwrap();
    assert_eq!(job.route.identity_id, f.ids[1]);
    f.store
        .finish_provider_job(
            &job,
            "completed",
            Some("fixture-chat"),
            Some("private answer"),
        )
        .unwrap();
    assert_eq!(f.history(0).len(), 2);
    assert_eq!(
        f.history(1)[1].in_reply_to_msg.as_deref(),
        Some(question.as_str())
    );
    assert!(f.history(2).is_empty());
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(f.count("provider_jobs"), 1);
}
#[test]
fn unsupported_binary_delivery_is_explicit_without_dispatch_or_blocking_later_text() {
    let f = Fixture::new();
    f.bind(1, f.to(&[0]));
    let mut binary = MessageDraft::text(f.to(&[1]), "AA==");
    binary.body_encoding = BodyEncoding::Base64;
    binary.content_type = "application/octet-stream".into();
    f.send(0, "binary", binary);
    assert!(f.store.claim_provider_job().unwrap().is_none());
    let Outcome::ProviderStatus { deliveries } = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ProviderStatus {
                conversation_id: f.cid.clone(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(deliveries[0].state, "unsupported");
    assert_eq!(
        deliveries[0].refusal_code.as_deref(),
        Some("unsupported-message-content")
    );
    f.send(0, "text", MessageDraft::text(f.to(&[1]), "now supported"));
    assert!(f.store.claim_provider_job().unwrap().is_some());
}

#[test]
fn independent_inbound_policy_is_explicit_and_new_all_messages_resolve_current_members() {
    use connect_connector_api::lifecycle::{IncomingMessage, Verification};
    let f = Fixture::new();
    f.bind(1, f.to(&[0]));
    f.bind(2, MessageAudience::all());
    let incoming = |id: &str, body: &str| IncomingMessage {
        mutation_id: id.into(),
        external_id: format!("external-{id}"),
        body: body.into(),
        verification: Verification::VerifiedCallback,
    };
    let first = incoming("first", "private independent");
    let id = f
        .store
        .receive_connector_message("vapi", "route1", &first)
        .unwrap();
    assert_eq!(
        f.store
            .receive_connector_message("vapi", "route1", &first)
            .unwrap(),
        id
    );
    assert!(f.history(2).is_empty());
    assert_eq!(f.history(0).len(), 1);
    assert_eq!(
        f.store
            .receive_connector_message("vapi", "route1", &incoming("first", "changed"))
            .unwrap_err()
            .code,
        "conflict"
    );
    let all = incoming("all", "old all");
    let old = f
        .store
        .receive_connector_message("vapi", "route2", &all)
        .unwrap();
    f.join(3);
    assert!(f.history(3).is_empty());
    assert_eq!(
        f.store
            .receive_connector_message("vapi", "route2", &all)
            .unwrap(),
        old
    );
    assert!(f.history(3).is_empty());
    f.store
        .receive_connector_message("vapi", "route2", &incoming("new", "new all"))
        .unwrap();
    assert_eq!(f.history(3).len(), 1);
    assert_eq!(f.history(3)[0].body, "new all");
}
#[test]
fn leave_and_rejoin_do_not_restore_original_message_receipts_or_queue_authority() {
    let f = Fixture::new();
    f.bind(1, f.to(&[0]));
    let operation = f.op("old", MessageDraft::text(f.to(&[1]), "old epoch"));
    f.store.execute(&f.keys[0], &operation).unwrap();
    f.membership(0, "leave", MembershipAction::Leave {});
    let old_epoch: f64 = f
        .c()
        .query_row(
            "SELECT membership_epoch FROM participants WHERE identity_id=?1 AND conversation_id=?2",
            rusqlite::params![f.ids[0], f.cid],
            |r| r.get(0),
        )
        .unwrap();
    let revision = f
        .c()
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [&f.cid],
            |r| r.get(0),
        )
        .unwrap();
    let MembershipOutcome::Invitation { invitation, .. } = f
        .store
        .execute_membership(
            &f.keys[1],
            MembershipRequest {
                operation_id: "invite-back".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Invite {
                    identity_id: f.ids[0].clone(),
                    kind: IdentityKind::Human,
                    role: "member".into(),
                    expires_at: chrono::Utc::now().timestamp() + 600,
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let revision = f
        .c()
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [&f.cid],
            |r| r.get(0),
        )
        .unwrap();
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "rejoin".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Join {
                    invitation_id: invitation.invitation_id,
                },
            },
        )
        .unwrap();
    let epoch: u64 = f
        .c()
        .query_row(
            "SELECT membership_epoch FROM participants WHERE identity_id=?1 AND conversation_id=?2",
            rusqlite::params![f.ids[0], f.cid],
            |r| r.get(0),
        )
        .unwrap();
    assert!(epoch as f64 > old_epoch);
    assert_eq!(
        f.store.execute(&f.keys[0], &operation).unwrap_err().code,
        "forbidden"
    );
    assert_eq!(
        recovered(&f, &f.keys[0], "old").unwrap_err().code,
        "forbidden"
    );
    assert!(f.history(0).is_empty());
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn attachment_references_require_ready_matching_metadata_and_each_recipient_access() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let bytes = b"binary\0fixture";
    let Outcome::Artifact(artifact) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ArtifactBegin {
                to: connect_protocol::messages::MessageAudience::all(),
                request_id: "begin".into(),
                conversation_id: f.cid.clone(),
                name: "sample.bin".into(),
                media_type: "application/octet-stream".into(),
                size: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
                expires_at: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let mut draft = MessageDraft::text(f.to(&[1]), "");
    draft.attachments = vec![AttachmentRef {
        id: artifact.id.clone(),
        content_type: artifact.media_type.clone(),
        size_bytes: artifact.size,
    }];
    assert_eq!(
        f.store
            .execute(&f.keys[0], &f.op("not-ready", draft.clone()))
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .execute(
            &f.keys[0],
            &Operation::ArtifactAppend {
                request_id: "append".into(),
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                data: STANDARD.encode(bytes),
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
    let id = f.send(0, "attached", draft.clone());
    assert_eq!(f.history(1)[0].attachments, draft.attachments);
    assert_eq!(f.history(1)[0].id, id);
    let mut wrong = draft.clone();
    wrong.attachments[0].size_bytes += 1;
    assert_eq!(
        f.store
            .execute(&f.keys[0], &f.op("wrong-size", wrong))
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.join(3);
    let mut late = draft.clone();
    late.to = f.to(&[3]);
    assert_eq!(
        f.store
            .execute(&f.keys[0], &f.op("late-artifact", late))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut external = serde_json::to_value(&draft).unwrap();
    external["attachments"][0]["url"] = serde_json::json!("https://private.invalid/file");
    assert!(serde_json::from_value::<MessageDraft>(external).is_err());
    let backup = f._dir.path().join("typed-backup.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    let Outcome::History { messages, .. } = restored
        .message_history(&f.keys[1], &f.cid, &HistoryQuery::default())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages[0].attachments, draft.attachments);
}

#[test]
fn explicit_parent_share_allows_a_new_reply_and_foreign_participant_ids_are_denied() {
    let f = Fixture::new();
    let parent = f.send(0, "question", MessageDraft::text(f.to(&[1]), "private"));
    let mut reply = MessageDraft::text(f.to(&[0]), "guess private source");
    reply.in_reply_to_msg = Some(parent.clone());
    assert_eq!(
        f.store
            .execute(&f.keys[2], &f.op("guess", reply.clone()))
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.membership(
        0,
        "share-parent",
        MembershipAction::ShareHistory {
            resources: vec![ContentRef {
                kind: ContentKind::Message,
                id: parent.clone(),
            }],
            participants: vec![f.participant(2)],
            provenance: "explicit disclosure".into(),
        },
    );
    reply.to = f.to(&[0, 1]);
    let id = f.send(2, "reply-after-share", reply);
    assert_eq!(f.history(0)[1].id, id);
    assert_eq!(
        f.history(2)[1].in_reply_to_msg.as_deref(),
        Some(parent.as_str())
    );
    let Outcome::Conversation(other) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Create {
                request_id: "other-room".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    f.ids[1].clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let foreign = f
        .store
        .membership_snapshot(&f.keys[0], &other.id)
        .unwrap()
        .participants[0]
        .participant_id
        .clone();
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &f.op(
                    "cross-cid",
                    MessageDraft::text(MessageAudience::Participants(vec![foreign]), "denied")
                )
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn unsupported_typed_task_effect_is_terminal_without_external_dispatch() {
    use connect_protocol::tasks::*;
    let mut f = Fixture::new();
    f.keys[0] = f
        .store
        .issue(
            &f.ids[0],
            Scope::operator(),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap()
        .id;
    f.bind(1, f.to(&[0]));
    let TaskOutcome::Record(task) = f
        .store
        .execute_task(
            &f.keys[0],
            &TaskRequest::new(TaskCommand::Create {
                request_id: "task".into(),
                definition: TaskDefinition {
                    conversation_id: f.cid.clone(),
                    scope: TaskScope {
                        operations: vec![TaskPermission::Send],
                        recipients: vec![f.ids[1].clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: chrono::Utc::now().timestamp() + 600,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap()
    else {
        panic!()
    };
    let mut draft = MessageDraft::text(f.to(&[1]), "AA==");
    draft.body_encoding = BodyEncoding::Base64;
    draft.content_type = "application/octet-stream".into();
    let request = TaskRequest::new(TaskCommand::Invoke {
        request_id: "binary-task".into(),
        task_id: task.id,
        step_id: "step".into(),
        operation: Box::new(f.op("binary-task", draft)),
        reply_to: None,
        wait_for_reply: false,
    });
    assert_eq!(
        f.store.execute_task(&f.keys[0], &request).unwrap_err().code,
        "provider-failed"
    );
    assert_eq!(
        f.store.execute_task(&f.keys[0], &request).unwrap_err().code,
        "provider-failed"
    );
    assert!(f.store.claim_provider_job().unwrap().is_none());
    assert_eq!(f.count("messages"), 1);
}

#[test]
fn in_flight_legacy_reply_cannot_borrow_readmission_or_a_read_share_for_old_authority() {
    let f = Fixture::new();
    f.bind(1, f.to(&[0]));
    let question = f.send(0, "question", MessageDraft::text(f.to(&[1]), "private"));
    let job = f.store.claim_provider_job().unwrap().unwrap();
    f.membership(1, "leave-bot", MembershipAction::Leave {});
    let revision = f
        .c()
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [&f.cid],
            |r| r.get(0),
        )
        .unwrap();
    let MembershipOutcome::Invitation { invitation, .. } = f
        .store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "invite-bot-back".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Invite {
                    identity_id: f.ids[1].clone(),
                    kind: IdentityKind::Human,
                    role: "member".into(),
                    expires_at: chrono::Utc::now().timestamp() + 500,
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.membership(
        1,
        "rejoin-bot",
        MembershipAction::Join {
            invitation_id: invitation.invitation_id,
        },
    );
    f.membership(
        0,
        "reshare",
        MembershipAction::ShareHistory {
            resources: vec![ContentRef {
                kind: ContentKind::Message,
                id: question,
            }],
            participants: vec![f.participant(1)],
            provenance: "read-only disclosure".into(),
        },
    );
    assert_eq!(f.history(1).len(), 1);
    let before = f.count("messages");
    assert_eq!(
        f.store
            .finish_provider_job(
                &job,
                "completed",
                Some("old-chat"),
                Some("must not revive job")
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("messages"), before);
}

fn recovered(
    f: &Fixture,
    key: &str,
    id: &str,
) -> connect_protocol::Result<connect_protocol::transitions::OperationRecord> {
    f.store.execute_operation_action(
        key,
        connect_protocol::transitions::OperationAction::Get {
            operation_id: id.into(),
        },
    )
}
#[test]
fn ordinary_message_recovery_uses_one_shared_receipt_and_same_id_never_refans_out() {
    use connect_protocol::transitions::OperationState;
    let f = Fixture::new();
    let request = f.op(
        "recover-message",
        MessageDraft::text(f.to(&[1]), "private durable message"),
    );
    let accepted = f.store.execute(&f.keys[0], &request).unwrap();
    let record = recovered(&f, &f.keys[0], "recover-message").unwrap();
    assert_eq!(record.state, OperationState::Completed);
    assert_eq!(record.conversation_id.as_deref(), Some(f.cid.as_str()));
    assert_eq!(
        record.result,
        Some(serde_json::to_value(&accepted).unwrap())
    );
    assert!(!record.causal_event_ids.is_empty());
    assert_eq!(
        f.c()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='requests'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        f.c()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='recover-message'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(f.store.execute(&f.keys[0], &request).unwrap(), accepted);
    assert_eq!(
        recovered(&f, &f.keys[0], "recover-message").unwrap(),
        record
    );
    let Outcome::RequestStatus { state, outcome } = f
        .store
        .execute(
            &f.keys[0],
            &Operation::RequestStatus {
                request_id: "recover-message".into(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(state, "accepted");
    assert_eq!(outcome.as_deref(), Some(&accepted));
    let changed = f.op(
        "recover-message",
        MessageDraft::text(f.to(&[1]), "different intent"),
    );
    assert_eq!(
        f.store.execute(&f.keys[0], &changed).unwrap_err().code,
        "conflict"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let backup = f._dir.path().join("receipt-restart.sqlite3");
    f.store.backup(&backup).unwrap();
    let reopened = Store::open(&backup, connect_protocol::Limits::default()).unwrap();
    assert_eq!(
        reopened
            .execute_operation_action(
                &f.keys[0],
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "recover-message".into()
                }
            )
            .unwrap(),
        record
    );
    assert_eq!(f.count("messages"), 1);
    assert_eq!(
        recovered(&f, &f.keys[1], "recover-message")
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn ordinary_rejection_and_receipt_storage_failure_have_distinct_atomic_recovery() {
    use connect_protocol::transitions::OperationState;
    let f = Fixture::new();
    let empty = f.op("rejected-message", MessageDraft::text(f.to(&[1]), ""));
    assert_eq!(
        f.store.execute(&f.keys[0], &empty).unwrap_err().code,
        "invalid-request"
    );
    let rejected = recovered(&f, &f.keys[0], "rejected-message").unwrap();
    assert_eq!(rejected.state, OperationState::Failed);
    assert!(rejected.result.is_none());
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store.execute(&f.keys[0], &empty).unwrap_err().code,
        "invalid-request"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let c = f.c();
    c.execute_batch("CREATE TRIGGER reject_control_receipt BEFORE INSERT ON operation_control_receipts BEGIN SELECT RAISE(ABORT,'isolated receipt write failure'); END;").unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let request = f.op(
        "storage-retry",
        MessageDraft::text(f.to(&[1]), "atomic message"),
    );
    assert_eq!(
        f.store.execute(&f.keys[0], &request).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(
        recovered(&f, &f.keys[0], "storage-retry").unwrap_err().code,
        "forbidden"
    );
    c.execute_batch("DROP TRIGGER reject_control_receipt")
        .unwrap();
    f.store.execute(&f.keys[0], &request).unwrap();
    assert_eq!(
        recovered(&f, &f.keys[0], "storage-retry").unwrap().state,
        OperationState::Completed
    );
    assert_eq!(f.count("messages"), 1);
}
#[test]
fn ordinary_receipt_rotation_requires_current_original_permission_and_immutable_origin() {
    let f = Fixture::new();
    f.send(
        0,
        "rotate-receipt",
        MessageDraft::text(f.to(&[1]), "retained"),
    );
    let receipt = recovered(&f, &f.keys[0], "rotate-receipt").unwrap();
    let replacement = f
        .store
        .issue(
            &f.ids[0],
            Scope::messaging(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let admin = f
        .store
        .issue(
            &f.ids[0],
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store
        .revoke(&f.store.authenticate(&admin.token).unwrap(), &f.keys[0])
        .unwrap();
    assert_eq!(
        recovered(&f, &replacement.id, "rotate-receipt").unwrap(),
        receipt
    );
    assert!(recovered(&f, &f.keys[0], "rotate-receipt").is_err());
    for permissions in [vec!["operation.get".into()], vec!["send".into()]] {
        let limited = f
            .store
            .issue(
                &f.ids[0],
                Scope {
                    operations: permissions,
                    recipients: vec!["*".into()],
                },
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        assert_eq!(
            recovered(&f, &limited.id, "rotate-receipt")
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let c = f.c();
    for sql in [
        "UPDATE operation_control_receipts SET resources_json='[]'",
        "DELETE FROM operation_control_receipts",
        "UPDATE operations SET result_json='{}' WHERE operation_id='rotate-receipt'",
        "UPDATE operations SET state='cancelled' WHERE operation_id='rotate-receipt'",
    ] {
        assert!(c.execute_batch(sql).is_err());
    }
    assert_eq!(
        recovered(&f, &replacement.id, "rotate-receipt").unwrap(),
        receipt
    );
}
