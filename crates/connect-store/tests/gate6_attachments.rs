//! Explicit artifact audiences are independent of labels and future members.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_protocol::{
    membership::*, messages::*, provisioning::IdentityKind, Limits, Operation, Outcome,
};
use connect_store::{Scope, Store};
use sha2::{Digest, Sha256};
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    ids: Vec<String>,
    keys: Vec<String>,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
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
            panic!()
        };
        Self {
            _dir: dir,
            store,
            ids,
            keys,
            cid: c.id,
        }
    }
    fn participant(&self, n: usize) -> String {
        self.store
            .membership_snapshot(&self.keys[0], &self.cid)
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.identity_id == self.ids[n])
            .unwrap()
            .participant_id
    }
    fn to(&self, n: usize) -> MessageAudience {
        MessageAudience::Participants(vec![self.participant(n)])
    }
    fn begin(&self, id: &str, to: MessageAudience) -> Operation {
        Operation::ArtifactBegin {
            to,
            request_id: id.into(),
            conversation_id: self.cid.clone(),
            name: "private.bin".into(),
            media_type: "application/octet-stream".into(),
            size: 6,
            sha256: hex::encode(Sha256::digest(b"secret")),
            expires_at: None,
        }
    }
    fn get(&self, id: &str) -> Operation {
        Operation::ArtifactGet {
            conversation_id: self.cid.clone(),
            artifact_id: id.into(),
            offset: 0,
            limit: 10,
        }
    }
    fn change(&self, who: usize, id: &str, change: MembershipAction) -> MembershipOutcome {
        let revision = self
            .store
            .membership_snapshot(&self.keys[0], &self.cid)
            .unwrap()
            .revision;
        self.store
            .execute_membership(
                &self.keys[who],
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
        let MembershipOutcome::Invitation { invitation, .. } = self.change(
            0,
            "invite",
            MembershipAction::Invite {
                identity_id: self.ids[n].clone(),
                kind: IdentityKind::Human,
                role: "member".into(),
                expires_at: chrono::Utc::now().timestamp() + 600,
            },
        ) else {
            panic!()
        };
        self.change(
            n,
            "join",
            MembershipAction::Join {
                invitation_id: invitation.invitation_id,
            },
        );
    }
    fn finish(&self, id: &str) {
        self.store
            .execute(
                &self.keys[0],
                &Operation::ArtifactAppend {
                    request_id: "append".into(),
                    conversation_id: self.cid.clone(),
                    artifact_id: id.into(),
                    offset: 0,
                    data: STANDARD.encode(b"secret"),
                },
            )
            .unwrap();
        self.store
            .execute(
                &self.keys[0],
                &Operation::ArtifactFinish {
                    request_id: "finish".into(),
                    conversation_id: self.cid.clone(),
                    artifact_id: id.into(),
                },
            )
            .unwrap();
    }
}
#[test]
fn private_upload_metadata_and_bytes_reach_only_the_explicit_audience() {
    let f = Fixture::new();
    let narrow = f
        .store
        .issue(
            &f.ids[0],
            Scope {
                operations: vec!["artifact_begin".into()],
                recipients: vec![f.ids[1].clone()],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let Outcome::Artifact(artifact) = f
        .store
        .execute(&narrow.id, &f.begin("private", f.to(1)))
        .unwrap()
    else {
        panic!()
    };
    f.finish(&artifact.id);
    let Outcome::ArtifactChunk(chunk) = f.store.execute(&f.keys[1], &f.get(&artifact.id)).unwrap()
    else {
        panic!()
    };
    assert_eq!(STANDARD.decode(chunk.data).unwrap(), b"secret");
    assert_eq!(
        f.store
            .execute(&f.keys[2], &f.get(&artifact.id))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let Outcome::Inbox { events, .. } = f
        .store
        .execute(
            &f.keys[2],
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
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains(&artifact.id));
    let mut draft = MessageDraft::text(f.to(1), "attached");
    draft.attachments = vec![AttachmentRef {
        id: artifact.id.clone(),
        content_type: artifact.media_type,
        size_bytes: artifact.size,
    }];
    f.store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "message".into(),
                conversation_id: f.cid.clone(),
                message: draft.clone(),
            },
        )
        .unwrap();
    let Outcome::History { messages, .. } = f
        .store
        .message_history(
            &f.keys[1],
            &f.cid,
            &HistoryQuery {
                include_attachments: Some(false),
                ..Default::default()
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages[0].attachments, draft.attachments);
}
#[test]
fn all_upload_receipt_replay_and_finalization_do_not_add_late_members() {
    let f = Fixture::new();
    let op = f.begin("all", MessageAudience::all());
    let first = f.store.execute(&f.keys[0], &op).unwrap();
    let Outcome::Artifact(artifact) = &first else {
        panic!()
    };
    f.join(3);
    assert_eq!(f.store.execute(&f.keys[0], &op).unwrap(), first);
    f.finish(&artifact.id);
    assert_eq!(
        f.store
            .execute(&f.keys[3], &f.get(&artifact.id))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let Outcome::Inbox { events, .. } = f
        .store
        .execute(
            &f.keys[3],
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
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains(&artifact.id));
    assert_eq!(
        f.store
            .execute(&f.keys[0], &f.begin("all", f.to(1)))
            .unwrap_err()
            .code,
        "conflict"
    );
    let backup = f._dir.path().join("backup.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(restored.execute(&f.keys[0], &op).unwrap(), first);
    assert_eq!(
        restored
            .execute(&f.keys[3], &f.get(&artifact.id))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn revoked_read_share_stops_bytes_and_attachment_reference_visibility() {
    let f = Fixture::new();
    let Outcome::Artifact(artifact) = f
        .store
        .execute(&f.keys[0], &f.begin("private", f.to(1)))
        .unwrap()
    else {
        panic!()
    };
    f.finish(&artifact.id);
    let mut draft = MessageDraft::text(f.to(1), "shared message with private file");
    draft.attachments = vec![AttachmentRef {
        id: artifact.id.clone(),
        content_type: artifact.media_type.clone(),
        size_bytes: artifact.size,
    }];
    let Outcome::Accepted { message_id, .. } = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "attached".into(),
                conversation_id: f.cid.clone(),
                message: draft,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.change(
        0,
        "share-message",
        MembershipAction::ShareHistory {
            resources: vec![ContentRef {
                kind: ContentKind::Message,
                id: message_id.clone(),
            }],
            participants: vec![f.participant(2)],
            provenance: "explicit message disclosure".into(),
        },
    );
    let MembershipOutcome::Sharing { sharing_id, .. } = f.change(
        0,
        "share",
        MembershipAction::ShareHistory {
            resources: vec![ContentRef {
                kind: ContentKind::Artifact,
                id: artifact.id.clone(),
            }],
            participants: vec![f.participant(2)],
            provenance: "explicit file sharing".into(),
        },
    ) else {
        panic!()
    };
    let Outcome::History { messages, .. } = f
        .store
        .message_history(&f.keys[2], &f.cid, &HistoryQuery::default())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages[0].attachments[0].id, artifact.id);
    assert!(f.store.execute(&f.keys[2], &f.get(&artifact.id)).is_ok());
    f.change(
        0,
        "revoke",
        MembershipAction::RevokeHistoryShare { sharing_id },
    );
    assert_eq!(
        f.store
            .execute(&f.keys[2], &f.get(&artifact.id))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let Outcome::History { messages, .. } = f
        .store
        .message_history(&f.keys[2], &f.cid, &HistoryQuery::default())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, message_id);
    assert!(messages[0].attachments.is_empty());
}

impl Fixture {
    fn link(
        &self,
        who: usize,
        id: &str,
        message: Option<&str>,
        ttl: u32,
    ) -> connect_protocol::attachments::ArtifactLink {
        let operation = match message {
            Some(msg) => Operation::ArtifactDownloadLink {
                conversation_id: self.cid.clone(),
                artifact_id: id.into(),
                message_id: msg.into(),
                expires_in: ttl,
            },
            None => Operation::ArtifactUploadLink {
                conversation_id: self.cid.clone(),
                artifact_id: id.into(),
                expires_in: ttl,
            },
        };
        let Outcome::ArtifactLink(link) = self.store.execute(&self.keys[who], &operation).unwrap()
        else {
            panic!("link")
        };
        link
    }
    fn via(&self, id: &str, link: &str) -> Operation {
        Operation::ArtifactLinkGet {
            conversation_id: self.cid.clone(),
            artifact_id: id.into(),
            link: link.into(),
            offset: 0,
            limit: 10,
        }
    }
    fn attached(&self, id: &str) -> String {
        let mut draft = MessageDraft::text(self.to(1), "private attachment");
        draft.attachments = vec![AttachmentRef {
            id: id.into(),
            content_type: "application/octet-stream".into(),
            size_bytes: 6,
        }];
        let Outcome::Accepted { message_id, .. } = self
            .store
            .execute(
                &self.keys[0],
                &Operation::Send {
                    request_id: "attached".into(),
                    conversation_id: self.cid.clone(),
                    message: draft,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        message_id
    }
}
#[test]
fn upload_link_is_current_owner_bound_and_refresh_preserves_chunk_identity() {
    let f = Fixture::new();
    let Outcome::Artifact(artifact) = f
        .store
        .execute(&f.keys[0], &f.begin("private", f.to(1)))
        .unwrap()
    else {
        panic!()
    };
    let upload = f.link(0, &artifact.id, None, 300);
    let append = |link: &str, data: &[u8]| Operation::ArtifactLinkAppend {
        request_id: "link-chunk".into(),
        conversation_id: f.cid.clone(),
        artifact_id: artifact.id.clone(),
        link: link.into(),
        offset: 0,
        data: STANDARD.encode(data),
    };
    assert_eq!(
        f.store
            .execute(&f.keys[1], &append(&upload.path, b"secret"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let accepted = f
        .store
        .execute(&f.keys[0], &append(&upload.path, b"secret"))
        .unwrap();
    let refreshed = f.link(0, &artifact.id, None, 100);
    assert_ne!(upload.path, refreshed.path);
    assert_eq!(
        f.store
            .execute(&f.keys[0], &append(&refreshed.path, b"secret"))
            .unwrap(),
        accepted
    );
    assert_eq!(
        f.store
            .execute(
                &f.keys[0],
                &Operation::ArtifactAppend {
                    request_id: "link-chunk".into(),
                    conversation_id: f.cid.clone(),
                    artifact_id: artifact.id.clone(),
                    offset: 0,
                    data: STANDARD.encode(b"secret")
                }
            )
            .unwrap(),
        accepted
    );
    assert_eq!(
        f.store
            .execute(&f.keys[0], &append(&refreshed.path, b"change"))
            .unwrap_err()
            .code,
        "conflict"
    );
    let Outcome::Artifact(progress) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ArtifactInspect {
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(progress.received, 6);
    let mut altered = upload.path.clone();
    altered.push('x');
    assert_eq!(
        f.store
            .execute(&f.keys[0], &append(&altered, b"secret"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(!format!("{upload:?}").contains(&upload.path));
}
#[test]
fn download_link_requires_both_message_and_file_and_is_not_a_bearer_capability() {
    let f = Fixture::new();
    let Outcome::Artifact(artifact) = f
        .store
        .execute(&f.keys[0], &f.begin("private", f.to(1)))
        .unwrap()
    else {
        panic!()
    };
    f.finish(&artifact.id);
    let message = f.attached(&artifact.id);
    let link = f.link(1, &artifact.id, Some(&message), 300);
    let Outcome::ArtifactChunk(chunk) = f
        .store
        .execute(&f.keys[1], &f.via(&artifact.id, &link.path))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(STANDARD.decode(chunk.data).unwrap(), b"secret");
    for who in [0, 2] {
        assert_eq!(
            f.store
                .execute(&f.keys[who], &f.via(&artifact.id, &link.path))
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    assert_eq!(
        f.store
            .execute("unknown-credential", &f.via(&artifact.id, &link.path))
            .unwrap_err()
            .code,
        "unauthorized"
    );
    let plain = Operation::ArtifactDownloadLink {
        conversation_id: f.cid.clone(),
        artifact_id: artifact.id.clone(),
        message_id: "unrelated-message".into(),
        expires_in: 300,
    };
    assert_eq!(
        f.store.execute(&f.keys[1], &plain).unwrap_err().code,
        "forbidden"
    );
    let append = Operation::ArtifactLinkAppend {
        request_id: "wrong-purpose".into(),
        conversation_id: f.cid.clone(),
        artifact_id: artifact.id.clone(),
        link: link.path,
        offset: 0,
        data: STANDARD.encode(b"secret"),
    };
    assert_eq!(
        f.store.execute(&f.keys[1], &append).unwrap_err().code,
        "forbidden"
    );
}
#[test]
fn stale_links_fail_after_expiry_restart_backup_and_credential_rotation() {
    let f = Fixture::new();
    let Outcome::Artifact(artifact) = f
        .store
        .execute(&f.keys[0], &f.begin("private", f.to(1)))
        .unwrap()
    else {
        panic!()
    };
    f.finish(&artifact.id);
    let message = f.attached(&artifact.id);
    let short = f.link(1, &artifact.id, Some(&message), 1);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(
        f.store
            .execute(&f.keys[1], &f.via(&artifact.id, &short.path))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let link = f.link(1, &artifact.id, Some(&message), 300);
    let replacement = f
        .store
        .issue(
            &f.ids[1],
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute(&replacement.id, &f.via(&artifact.id, &link.path))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let restarted = Store::open(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        restarted
            .execute(&f.keys[1], &f.via(&artifact.id, &link.path))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let backup = f._dir.path().join("links-backup.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(
        restored
            .execute(&f.keys[1], &f.via(&artifact.id, &link.path))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let Outcome::ArtifactLink(fresh) = restarted
        .execute(
            &f.keys[1],
            &Operation::ArtifactDownloadLink {
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id.clone(),
                message_id: message,
                expires_in: 300,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(restarted
        .execute(&f.keys[1], &f.via(&artifact.id, &fresh.path))
        .is_ok());
}
#[test]
fn link_redemption_rechecks_revoked_file_share_even_while_message_remains_visible() {
    let f = Fixture::new();
    let Outcome::Artifact(artifact) = f
        .store
        .execute(&f.keys[0], &f.begin("private", f.to(1)))
        .unwrap()
    else {
        panic!()
    };
    f.finish(&artifact.id);
    let message = f.attached(&artifact.id);
    f.change(
        0,
        "share-message",
        MembershipAction::ShareHistory {
            resources: vec![ContentRef {
                kind: ContentKind::Message,
                id: message.clone(),
            }],
            participants: vec![f.participant(2)],
            provenance: "message sharing".into(),
        },
    );
    let MembershipOutcome::Sharing { sharing_id, .. } = f.change(
        0,
        "share-file",
        MembershipAction::ShareHistory {
            resources: vec![ContentRef {
                kind: ContentKind::Artifact,
                id: artifact.id.clone(),
            }],
            participants: vec![f.participant(2)],
            provenance: "file sharing".into(),
        },
    ) else {
        panic!()
    };
    let link = f.link(2, &artifact.id, Some(&message), 300);
    assert!(f
        .store
        .execute(&f.keys[2], &f.via(&artifact.id, &link.path))
        .is_ok());
    f.change(
        0,
        "revoke-file",
        MembershipAction::RevokeHistoryShare { sharing_id },
    );
    assert_eq!(
        f.store
            .execute(&f.keys[2], &f.via(&artifact.id, &link.path))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let Outcome::History { messages, .. } = f
        .store
        .message_history(&f.keys[2], &f.cid, &HistoryQuery::default())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages[0].id, message);
    assert!(messages[0].attachments.is_empty());
}

#[test]
fn task_links_keep_task_grant_ceiling_and_renewal_never_repeats_the_chunk() {
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
    let permissions = vec![
        TaskPermission::ArtifactBegin,
        TaskPermission::ArtifactAppend,
        TaskPermission::ArtifactFinish,
        TaskPermission::ArtifactGet,
    ];
    let create = |id: &str| {
        let TaskOutcome::Record(task) = f
            .store
            .execute_task(
                &f.keys[0],
                &TaskRequest::new(TaskCommand::Create {
                    request_id: id.into(),
                    definition: TaskDefinition {
                        conversation_id: f.cid.clone(),
                        scope: TaskScope {
                            operations: permissions.clone(),
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
        task
    };
    let task = create("task-one");
    let other = create("task-two");
    let invoke = |task: &TaskRecord, id: &str, mut op: Operation| {
        if let Operation::ArtifactLinkAppend { request_id, .. } = &mut op {
            *request_id = id.into();
        }
        TaskRequest::new(TaskCommand::Invoke {
            request_id: id.into(),
            task_id: task.id.clone(),
            step_id: format!("step-{id}"),
            operation: Box::new(op),
            reply_to: None,
            wait_for_reply: false,
        })
    };
    let complete = |result: TaskOutcome| {
        let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = result else {
            panic!("completed local effect")
        };
        *outcome
    };
    let Outcome::Artifact(artifact) = complete(
        f.store
            .execute_task(
                &f.keys[0],
                &invoke(&task, "begin", f.begin("begin", f.to(1))),
            )
            .unwrap(),
    ) else {
        panic!()
    };
    let origin = InvocationBinding {
        kind: InvocationKind::Native,
        adapter: "fixture".into(),
        external_id: "links-one".into(),
    };
    f.store
        .register_task_binding(&f.keys[0], &task.id, &origin)
        .unwrap();
    let definition = GrantDefinition {
        task_id: task.id.clone(),
        binding: origin.clone(),
        operations: permissions.clone(),
        expires_at: chrono::Utc::now().timestamp() + 120,
    };
    let grant = f
        .store
        .issue_task_grant(&f.keys[0], "grant-one", &definition)
        .unwrap();
    let handle = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &origin)
        .unwrap();
    // The backing owner can read Carol's file; the Bob-only task/grant cannot.
    let Outcome::Artifact(carol_file) = f
        .store
        .execute(&f.keys[2], &f.begin("carol-file", MessageAudience::all()))
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute(
            &f.keys[2],
            &Operation::ArtifactAppend {
                request_id: "carol-chunk".into(),
                conversation_id: f.cid.clone(),
                artifact_id: carol_file.id.clone(),
                offset: 0,
                data: STANDARD.encode(b"secret"),
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[2],
            &Operation::ArtifactFinish {
                request_id: "carol-finish".into(),
                conversation_id: f.cid.clone(),
                artifact_id: carol_file.id.clone(),
            },
        )
        .unwrap();
    let mut message = MessageDraft::text(f.to(0), "Carol file");
    message.attachments.push(AttachmentRef {
        id: carol_file.id.clone(),
        content_type: carol_file.media_type,
        size_bytes: 6,
    });
    let Outcome::Accepted { message_id, .. } = f
        .store
        .execute(
            &f.keys[2],
            &Operation::Send {
                request_id: "carol-message".into(),
                conversation_id: f.cid.clone(),
                message,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    for (id, op) in [
        (
            "inspect-carol",
            Operation::ArtifactInspect {
                conversation_id: f.cid.clone(),
                artifact_id: carol_file.id.clone(),
            },
        ),
        ("read-carol", f.get(&carol_file.id)),
        (
            "link-carol",
            Operation::ArtifactDownloadLink {
                conversation_id: f.cid.clone(),
                artifact_id: carol_file.id.clone(),
                message_id,
                expires_in: 60,
            },
        ),
    ] {
        assert!(f.store.execute(&f.keys[0], &op).is_ok());
        assert_eq!(
            f.store
                .execute_task(&f.keys[0], &invoke(&task, id, op.clone()))
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(
            f.store
                .execute_task_invocation(&handle, &invoke(&task, id, op))
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let issue = |ttl: u32| Operation::ArtifactUploadLink {
        conversation_id: f.cid.clone(),
        artifact_id: artifact.id.clone(),
        expires_in: ttl,
    };
    let Outcome::ArtifactLink(link) = complete(
        f.store
            .execute_task_invocation(&handle, &invoke(&task, "issue", issue(300)))
            .unwrap(),
    ) else {
        panic!()
    };
    assert!(link.expires_at <= definition.expires_at);
    let append = |path: &str| Operation::ArtifactLinkAppend {
        request_id: "chunk".into(),
        conversation_id: f.cid.clone(),
        artifact_id: artifact.id.clone(),
        link: path.into(),
        offset: 0,
        data: STANDARD.encode(b"secret"),
    };
    assert_eq!(
        f.store
            .execute(&f.keys[0], &append(&link.path))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_task(
                &f.keys[0],
                &invoke(&other, "wrong-task", append(&link.path))
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let first = f
        .store
        .execute_task_invocation(&handle, &invoke(&task, "chunk", append(&link.path)))
        .unwrap();
    let Outcome::ArtifactLink(fresh) = complete(
        f.store
            .execute_task_invocation(&handle, &invoke(&task, "refresh", issue(60)))
            .unwrap(),
    ) else {
        panic!()
    };
    assert_ne!(fresh.path, link.path);
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &invoke(&task, "chunk", append(&fresh.path)))
            .unwrap(),
        first
    );
    let mut second = definition.clone();
    second.binding.external_id = "links-two".into();
    f.store
        .register_task_binding(&f.keys[0], &task.id, &second.binding)
        .unwrap();
    let second_grant = f
        .store
        .issue_task_grant(&f.keys[0], "grant-two", &second)
        .unwrap();
    let second_handle = f
        .store
        .authenticate_task_grant(second_grant.expose_secret(), &second.binding)
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation(
                &second_handle,
                &invoke(&task, "wrong-grant", append(&link.path))
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .revoke_task_grant(&f.keys[0], grant.grant.id.as_str())
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &invoke(&task, "chunk", append(&link.path)))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let Outcome::ArtifactLink(before_cancel) = complete(
        f.store
            .execute_task_invocation(&second_handle, &invoke(&task, "before-cancel", issue(30)))
            .unwrap(),
    ) else {
        panic!()
    };
    f.store
        .execute_task(
            &f.keys[0],
            &TaskRequest::new(TaskCommand::Cancel {
                request_id: "cancel-links".into(),
                task_id: task.id.clone(),
                reason: None,
            }),
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation(
                &second_handle,
                &invoke(&task, "after-cancel", append(&before_cancel.path))
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let Outcome::Artifact(progress) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ArtifactInspect {
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(progress.received, 6);
}
