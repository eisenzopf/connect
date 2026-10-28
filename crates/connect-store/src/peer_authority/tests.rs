#![allow(clippy::unwrap_used)]
use super::*;
use crate::Scope;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::{messages::*, provisioning::*, Limits, Operation, Outcome};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::time::{Duration, Instant};

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    admin: String,
    user: String,
    identity: String,
    cid: String,
    request: PeerAuthorityRequest,
}
impl Fixture {
    fn new(key_seconds: i64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let identity = store.create_identity("owner").unwrap().id;
        let admin = store
            .issue(&identity, Scope::operator(), crate::now() + 600)
            .unwrap()
            .id;
        let user = store
            .issue(
                &identity,
                Scope::collaboration(vec!["*".into()]),
                crate::now() + 600,
            )
            .unwrap()
            .id;
        store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Endpoint {
                    operation_id: "device".into(),
                    identity_id: identity.clone(),
                    device: DeviceSpec {
                        id: "device".into(),
                        kind: "desktop".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: "quic://localhost:4444".into(),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap();
        let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
        store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::SigningKey {
                    operation_id: "key".into(),
                    identity_id: identity.clone(),
                    credential_id: user.clone(),
                    device_id: "device".into(),
                    key_id: "key".into(),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                    not_before: crate::now() - 1,
                    expires_at: crate::now() + key_seconds,
                    expected_revision: None,
                },
            )
            .unwrap();
        let Outcome::Conversation(conversation) = store
            .execute(
                &user,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let request = PeerAuthorityRequest {
            credentials: vec![user.clone()],
            device: PeerDeviceClaim {
                id: "device".into(),
                kind: "desktop".into(),
                platform: "fixture".into(),
            },
            key_id: "key".into(),
            algorithm: "EdDSA".into(),
            command: PeerCommand::Data {
                conversation_id: conversation.id.clone(),
                command: PeerDataCommand::MessageSend,
            },
        };
        Self {
            _dir: dir,
            store,
            admin,
            user,
            identity,
            cid: conversation.id,
            request,
        }
    }
    fn admit(&self, request: &PeerAuthorityRequest) -> Result<PeerAuthorityGuard> {
        self.store
            .admit_peer_authority(request, &AuthorityWait::new(Duration::from_secs(1)))
    }
    fn send(&self, proof: &PeerAuthorityGuard, id: &str) -> Result<Outcome> {
        self.store.send_message_as_with_peer(
            &self.user,
            id,
            &self.cid,
            None,
            MessageDraft::text(MessageAudience::all(), "held"),
            Some(proof),
        )
    }
}
#[test]
fn peer_admission_requires_exact_current_device_key_credential_algorithm_and_resource() {
    let f = Fixture::new(300);
    let proof = f.admit(&f.request).unwrap();
    assert!(proof.current());
    assert_eq!(format!("{proof:?}"), "PeerAuthorityGuard(redacted)");
    drop(proof);
    let unbound = f
        .store
        .issue(&f.identity, Scope::operator(), crate::now() + 600)
        .unwrap();
    for n in 0..10 {
        let mut r = f.request.clone();
        match n {
            0 => r.device.id = "other".into(),
            1 => r.device.kind = "phone".into(),
            2 => r.device.platform = "other".into(),
            3 => r.key_id = "other".into(),
            4 => r.algorithm = "ES256".into(),
            5 => r.algorithm = "unknown".into(),
            6 => r.credentials = vec![unbound.id.clone()],
            7 => r.credentials.push(unbound.id.clone()),
            8 => {
                r.command = PeerCommand::Data {
                    conversation_id: "foreign".into(),
                    command: PeerDataCommand::MessageSend,
                }
            }
            _ => r.credentials.clear(),
        }
        assert!(f.admit(&r).is_err(), "claim permutation {n}");
    }
    let admin = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        admin
            .admit_peer_authority(&f.request, &AuthorityWait::new(Duration::from_millis(10)))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn peer_sql_rejects_foreign_root_wrong_credential_command_and_conversation_without_mutation() {
    let f = Fixture::new(300);
    let other = Fixture::new(300);
    let foreign = other.admit(&other.request).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(f.send(&foreign, "foreign").unwrap_err().code, "forbidden");
    let mut r = f.request.clone();
    r.command = PeerCommand::Data {
        conversation_id: f.cid.clone(),
        command: PeerDataCommand::MessageHistory,
    };
    let history = f.admit(&r).unwrap();
    assert_eq!(
        f.send(&history, "wrong-command").unwrap_err().code,
        "forbidden"
    );
    let send = f.admit(&f.request).unwrap();
    assert_eq!(
        f.store
            .message_history_with_peer(&f.user, &f.cid, &HistoryQuery::default(), Some(&send))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .send_message_as_with_peer(
                &f.admin,
                "wrong-credential",
                &f.cid,
                None,
                MessageDraft::text(MessageAudience::all(), "bad"),
                Some(&send)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .send_message_as_with_peer(
                &f.user,
                "wrong-cid",
                "another",
                None,
                MessageDraft::text(MessageAudience::all(), "bad"),
                Some(&send)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn peer_held_sql_completes_while_actual_key_retirement_waits_without_read_reentry() {
    let f = Fixture::new(300);
    let proof = f.admit(&f.request).unwrap();
    let peer = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    let admin = f.admin.clone();
    let writer = std::thread::spawn(move || {
        peer.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "revoke".into(),
                key_id: "key".into(),
                expected_revision: 0,
            },
        )
    });
    crate::authority_fences::test_wait_for_writer(
        &f.store,
        &AuthorityScope::identity(&f.identity, AuthorityMode::Write).unwrap(),
    );
    let start = Instant::now();
    assert!(f.send(&proof, "before-revoke").is_ok());
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(proof.current());
    drop(proof);
    writer.join().unwrap().unwrap();
    assert!(f.admit(&f.request).is_err());
    let Outcome::History { messages, .. } = f
        .store
        .message_history(&f.user, &f.cid, &HistoryQuery::default())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages.len(), 1);
}
#[test]
fn peer_sql_wait_rechecks_real_key_expiry_before_any_durable_write() {
    let f = Fixture::new(10);
    let proof = f.admit(&f.request).unwrap();
    let expiry = proof.expires_at();
    let connection = f.store.connection().unwrap();
    let store = f.store.clone();
    let user = f.user.clone();
    let cid = f.cid.clone();
    let (entered, observed) = std::sync::mpsc::channel();
    let pending = std::thread::spawn(move || {
        entered.send(()).unwrap();
        store.send_message_as_with_peer(
            &user,
            "expired-sql",
            &cid,
            None,
            MessageDraft::text(MessageAudience::all(), "must roll back"),
            Some(&proof),
        )
    });
    observed.recv_timeout(Duration::from_secs(1)).unwrap();
    while crate::now() < expiry {
        std::thread::sleep(Duration::from_millis(5));
    }
    let before: i64 = connection
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    drop(connection);
    assert_eq!(pending.join().unwrap().unwrap_err().code, "forbidden");
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn peer_proof_expiry_can_only_narrow_and_blocks_sql_after_its_validated_source_expires() {
    let f = Fixture::new(300);
    let mut proof = f.admit(&f.request).unwrap();
    let original = proof.expires_at();
    proof.limit_expiry(original + 100).unwrap();
    assert_eq!(proof.expires_at(), original);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert!(proof.limit_expiry(crate::now()).is_err());
    assert!(!proof.current());
    assert_eq!(
        f.send(&proof, "expired-source").unwrap_err().code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn peer_key_lookup_distinguishes_unknown_key_from_storage_failure_without_admission() {
    let f = Fixture::new(300);
    let mut unknown = f.request.clone();
    unknown.key_id = "missing".into();
    assert_eq!(f.admit(&unknown).unwrap_err().code, "forbidden");
    let c = f.store.connection().unwrap();
    let before: i64 = c
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    c.execute_batch("ALTER TABLE credential_signing_keys RENAME TO deliberately_unavailable_links")
        .unwrap();
    drop(c);
    assert_eq!(f.admit(&f.request).unwrap_err().code, "storage-unavailable");
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn conversation_peer_proof_binds_exact_create_close_and_list_without_reentry() {
    use connect_protocol::conversations::*;
    let f = Fixture::new(300);
    let operation = Operation::Create {
        request_id: "canonical-create".into(),
        request: CreateRequest::persistent(vec![]),
    };
    let mut req = f.request.clone();
    req.command = PeerCommand::Conversation(PeerConversationCommand {
        operation: operation.clone(),
        candidate_cid: Some("server_candidate".into()),
    });
    let proof = f.admit(&req).unwrap();
    let outcome = f
        .store
        .execute_conversation_with_peer(&f.user, &operation, &proof)
        .unwrap();
    let Outcome::Conversation(created) = outcome else {
        panic!()
    };
    assert_eq!(created.id, "server_candidate");
    let changed = Operation::Create {
        request_id: "changed-id".into(),
        request: CreateRequest::persistent(vec![]),
    };
    assert_eq!(
        f.store
            .execute_conversation_with_peer(&f.user, &changed, &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(proof);
    req.command = PeerCommand::Conversation(PeerConversationCommand {
        operation: operation.clone(),
        candidate_cid: Some("other_server_candidate".into()),
    });
    let proof = f.admit(&req).unwrap();
    assert_eq!(proof.creation_id.as_deref(), Some(created.id.as_str()));
    let Outcome::Conversation(replayed) = f
        .store
        .execute_conversation_with_peer(&f.user, &operation, &proof)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(replayed.id, created.id);
    drop(proof);
    let close = Operation::Close {
        request_id: "canonical-close".into(),
        conversation_id: created.id,
        options: CloseOptions {
            expected_revision: Some(created.revision),
            reason: Some("complete".into()),
            reason_code: Some(9),
        },
    };
    req.command = PeerCommand::Conversation(PeerConversationCommand {
        operation: close.clone(),
        candidate_cid: None,
    });
    let proof = f.admit(&req).unwrap();
    assert!(
        matches!(f.store.execute_conversation_with_peer(&f.user,&close,&proof).unwrap(),Outcome::Conversation(v) if v.closed&&v.reason_code==Some(9))
    );
    drop(proof);
    let list = Operation::Conversations {
        query: ListQuery {
            filter: ListFilter {
                state: ConversationState::Closed,
                ..Default::default()
            },
            ..Default::default()
        },
    };
    req.command = PeerCommand::Conversation(PeerConversationCommand {
        operation: list.clone(),
        candidate_cid: None,
    });
    let proof = f.admit(&req).unwrap();
    assert!(
        matches!(f.store.execute_conversation_with_peer(&f.user,&list,&proof).unwrap(),Outcome::Conversations(v) if v.conversations.len()==1)
    );
    assert_eq!(
        f.store
            .execute_conversation_with_peer(
                &f.user,
                &Operation::Conversations {
                    query: ListQuery::default()
                },
                &proof
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn conversation_create_source_expiring_behind_sql_wait_rolls_back_without_receipt() {
    use connect_protocol::conversations::CreateRequest;
    let f = Fixture::new(300);
    let operation = Operation::Create {
        request_id: "expired-create".into(),
        request: CreateRequest::persistent(vec![]),
    };
    let mut req = f.request.clone();
    req.command = PeerCommand::Conversation(PeerConversationCommand {
        operation: operation.clone(),
        candidate_cid: Some("server_expiring_candidate".into()),
    });
    let mut proof = f.admit(&req).unwrap();
    proof.limit_expiry(crate::now() + 1).unwrap();
    let c = Connection::open(f.store.path()).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let store = f.store.clone();
    let user = f.user.clone();
    let job =
        std::thread::spawn(move || store.execute_conversation_with_peer(&user, &operation, &proof));
    std::thread::sleep(Duration::from_millis(1200));
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(job.join().unwrap().unwrap_err().code, "forbidden");
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='expired-create'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM conversations WHERE id='server_expiring_candidate'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn conversation_receipt_projection_retains_new_current_member_identity() {
    use connect_protocol::conversations::CreateRequest;
    let f = Fixture::new(300);
    let added = f.store.create_identity("added").unwrap().id;
    let c = Connection::open(f.store.path()).unwrap();
    c.execute("INSERT INTO participants(conversation_id,identity_id,tenant_id,kind) SELECT ?1,id,tenant_id,kind FROM identities WHERE id=?2",params![f.cid,added]).unwrap();
    let operation = Operation::Create {
        request_id: "conversation".into(),
        request: CreateRequest::persistent(vec![]),
    };
    let mut req = f.request.clone();
    req.command = PeerCommand::Conversation(PeerConversationCommand {
        operation: operation.clone(),
        candidate_cid: Some("unused_candidate".into()),
    });
    let proof = f.admit(&req).unwrap();
    assert!(f.store.authority_covers(
        &proof._scopes,
        &[AuthorityScope::identity(&added, AuthorityMode::Read).unwrap()]
    ));
    assert!(
        matches!(f.store.execute_conversation_with_peer(&f.user,&operation,&proof).unwrap(),Outcome::Conversation(v) if v.participants.iter().any(|p|p.identity_id==added))
    );
}

fn membership_add(f: &Fixture, id: &str) -> connect_protocol::membership::MembershipRequest {
    let target = f.store.create_identity("membership-target").unwrap().id;
    connect_protocol::membership::MembershipRequest {
        operation_id: id.into(),
        conversation_id: f.cid.clone(),
        expected_revision: f
            .store
            .membership_snapshot(&f.user, &f.cid)
            .unwrap()
            .revision,
        change: connect_protocol::membership::MembershipAction::Add {
            identity_id: target,
            kind: IdentityKind::Human,
            role: "guest".into(),
        },
    }
}
#[test]
fn membership_peer_proof_binds_every_field_credential_and_installation_without_mutation() {
    let f = Fixture::new(300);
    let other = Fixture::new(300);
    let change = membership_add(&f, "exact-membership");
    let mut req = f.request.clone();
    req.command = PeerCommand::Membership(change.clone());
    let proof = f.admit(&req).unwrap();
    let foreign = other.admit(&other.request).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    for n in 0..8 {
        let mut changed = change.clone();
        match n {
            0 => changed.operation_id = "other".into(),
            1 => changed.conversation_id = "other".into(),
            2 => changed.expected_revision += 1,
            3 => {
                if let connect_protocol::membership::MembershipAction::Add { role, .. } =
                    &mut changed.change
                {
                    *role = "manager".into()
                }
            }
            4 => {
                if let connect_protocol::membership::MembershipAction::Add { identity_id, .. } =
                    &mut changed.change
                {
                    *identity_id = f.identity.clone()
                }
            }
            5 => {
                if let connect_protocol::membership::MembershipAction::Add { kind, .. } =
                    &mut changed.change
                {
                    *kind = IdentityKind::Ai
                }
            }
            _ => (),
        }
        let (credential, held) = match n {
            6 => (&f.admin, &proof),
            7 => (&f.user, &foreign),
            _ => (&f.user, &proof),
        };
        assert_eq!(
            f.store
                .execute_membership_with_peer(credential, changed, held)
                .unwrap_err()
                .code,
            "forbidden",
            "mutation {n}"
        );
    }
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let record = f
        .store
        .execute_membership_with_peer(&f.user, change, &proof)
        .unwrap();
    assert_eq!(record.operation_id, "exact-membership");
    assert_eq!(
        record.result.unwrap()["data"]["receipt"]["completion_boundary"],
        "local-mutation"
    );
}
#[test]
fn membership_peer_held_writer_finishes_before_queued_key_retirement_without_reentry() {
    let f = Fixture::new(300);
    let change = membership_add(&f, "membership-before-retire");
    let mut req = f.request.clone();
    req.command = PeerCommand::Membership(change.clone());
    let proof = f.admit(&req).unwrap();
    let store = f.store.clone();
    let admin = f.admin.clone();
    let writer = std::thread::spawn(move || {
        store.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "membership-key-retire".into(),
                key_id: "key".into(),
                expected_revision: 0,
            },
        )
    });
    crate::authority_fences::test_wait_for_writer(
        &f.store,
        &AuthorityScope::identity(&f.identity, AuthorityMode::Write).unwrap(),
    );
    let start = Instant::now();
    assert!(f
        .store
        .execute_membership_with_peer(&f.user, change, &proof)
        .is_ok());
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(proof);
    writer.join().unwrap().unwrap();
    assert!(f.admit(&req).is_err());
}
#[test]
fn membership_peer_source_expiring_behind_sql_wait_rolls_back_effect_receipt_and_journal() {
    let f = Fixture::new(300);
    let change = membership_add(&f, "expired-membership");
    let mut req = f.request.clone();
    req.command = PeerCommand::Membership(change.clone());
    let mut proof = f.admit(&req).unwrap();
    proof.limit_expiry(crate::now() + 1).unwrap();
    let c = Connection::open(f.store.path()).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let store = f.store.clone();
    let user = f.user.clone();
    let job = std::thread::spawn(move || store.execute_membership_with_peer(&user, change, &proof));
    std::thread::sleep(Duration::from_millis(1200));
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(job.join().unwrap().unwrap_err().code, "forbidden");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

fn subscription(f: &Fixture, seconds: u32) -> PeerSubscriptionCommand {
    PeerSubscriptionCommand {
        conversation_id: f.cid.clone(),
        query: connect_protocol::events::EventQuery {
            limit: Some(10),
            lease_seconds: Some(seconds),
            ..Default::default()
        },
        live: true,
        actor_epoch: None,
        ceiling: None,
    }
}
#[test]
fn subscription_peer_proof_binds_query_live_mode_and_snapshot_page_without_mutation() {
    let f = Fixture::new(300);
    let command = subscription(&f, 10);
    let mut request = f.request.clone();
    request.command = PeerCommand::Subscription(command.clone());
    let proof = f.admit(&request).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    for n in 0..7 {
        let mut changed = command.clone();
        match n {
            0 => changed.query.limit = Some(1),
            1 => changed.live = false,
            2 => changed.query.after = Some("other".into()),
            3 => changed.query.lease_seconds = Some(1),
            4 => changed.query.wait_ms = 1,
            5 => changed.conversation_id = "other".into(),
            _ => changed.actor_epoch = Some(("other".into(), 1)),
        }
        assert_eq!(
            f.store
                .conversation_events_with_peer(&f.user, &changed, &proof)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let (page, _) = f
        .store
        .start_peer_subscription(&f.user, &command, &proof)
        .unwrap();
    assert!(page.snapshot.is_some());
    drop(proof);
    let query = connect_protocol::events::SnapshotQuery {
        page: None,
        limit: Some(1),
    };
    request.command = PeerCommand::Snapshot {
        conversation_id: f.cid.clone(),
        query: query.clone(),
    };
    let proof = f.admit(&request).unwrap();
    let page = f
        .store
        .conversation_snapshot_with_peer(&f.user, &f.cid, query.clone(), Some(&proof))
        .unwrap();
    assert!(page.has_more);
    assert_eq!(
        f.store
            .conversation_snapshot_with_peer(
                &f.user,
                &f.cid,
                connect_protocol::events::SnapshotQuery {
                    page: page.next,
                    limit: Some(1)
                },
                Some(&proof)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn subscription_wait_holds_no_read_lock_but_prepared_page_holds_current_key_until_delivery_drop() {
    use connect_protocol::membership::*;
    let f = Fixture::new(300);
    let command = subscription(&f, 10);
    let mut request = f.request.clone();
    request.command = PeerCommand::Subscription(command.clone());
    let proof = f.admit(&request).unwrap();
    let (_, source) = f
        .store
        .start_peer_subscription(&f.user, &command, &proof)
        .unwrap();
    let source = source.unwrap();
    drop(proof);
    let snapshot = f.store.membership_snapshot(&f.user, &f.cid).unwrap();
    f.store
        .execute_membership(
            &f.user,
            MembershipRequest {
                operation_id: "change-between-pages".into(),
                conversation_id: f.cid.clone(),
                expected_revision: snapshot.revision,
                change: MembershipAction::UpdateRole {
                    participant_id: snapshot.participants[0].participant_id.clone(),
                    role: "reviewer".into(),
                },
            },
        )
        .unwrap();
    let read = f
        .store
        .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert!(read.page.events.iter().any(|event|matches!(&event.data,connect_protocol::events::JournalData::Membership{role:Some(role),..} if role=="reviewer")));
    let store = f.store.clone();
    let admin = f.admin.clone();
    let writer = std::thread::spawn(move || {
        store.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire-after-page".into(),
                key_id: "key".into(),
                expected_revision: 0,
            },
        )
    });
    crate::authority_fences::test_wait_for_writer(
        &f.store,
        &AuthorityScope::identity(&f.identity, AuthorityMode::Write).unwrap(),
    );
    assert!(!writer.is_finished());
    assert!(read.proof.current());
    drop(read);
    writer.join().unwrap().unwrap();
    assert_eq!(
        f.store
            .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_millis(50)))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn subscription_source_is_installation_and_original_epoch_bound_and_expiry_produces_resync() {
    use connect_protocol::membership::*;
    let f = Fixture::new(300);
    let other = Fixture::new(300);
    let command = subscription(&f, 5);
    let mut request = f.request.clone();
    request.command = PeerCommand::Subscription(command.clone());
    let proof = f.admit(&request).unwrap();
    let (_, source) = f
        .store
        .start_peer_subscription(&f.user, &command, &proof)
        .unwrap();
    let source = source.unwrap();
    drop(proof);
    assert_eq!(
        other
            .store
            .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_millis(50)))
            .unwrap_err()
            .code,
        "forbidden"
    );
    while crate::now() < source.expires_at() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let read = f
        .store
        .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert!(read.terminal && read.page.resync_required && read.page.snapshot.is_some());
    drop(read);
    let command = subscription(&f, 10);
    request.command = PeerCommand::Subscription(command.clone());
    let proof = f.admit(&request).unwrap();
    let (_, source) = f
        .store
        .start_peer_subscription(&f.user, &command, &proof)
        .unwrap();
    let source = source.unwrap();
    drop(proof);
    let revision = f
        .store
        .membership_snapshot(&f.user, &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute_membership(
            &f.user,
            MembershipRequest {
                operation_id: "leave-source-epoch".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Leave {},
            },
        )
        .unwrap();
    let c = f.store.connection().unwrap();
    let revision: u64 = c
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [&f.cid],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    f.store
        .execute_membership(
            &f.admin,
            MembershipRequest {
                operation_id: "new-source-epoch".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: MembershipAction::Add {
                    identity_id: f.identity.clone(),
                    kind: IdentityKind::Human,
                    role: "participant".into(),
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn subscription_prepared_message_keeps_departed_source_owner_fenced_without_reader_reentry() {
    use connect_protocol::membership::*;
    let f = Fixture::new(300);
    let bob = f.store.create_identity("departed-source").unwrap().id;
    let bob_key = f
        .store
        .issue(
            &bob,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 300,
        )
        .unwrap()
        .id;
    let state = f.store.membership_snapshot(&f.user, &f.cid).unwrap();
    f.store
        .execute_membership(
            &f.user,
            MembershipRequest {
                operation_id: "add-source-owner".into(),
                conversation_id: f.cid.clone(),
                expected_revision: state.revision,
                change: MembershipAction::Add {
                    identity_id: bob.clone(),
                    kind: IdentityKind::Human,
                    role: "participant".into(),
                },
            },
        )
        .unwrap();
    let mut command = subscription(&f, 30);
    command.query.limit = Some(1);
    let mut request = f.request.clone();
    request.command = PeerCommand::Subscription(command.clone());
    let proof = f.admit(&request).unwrap();
    let (_, source) = f
        .store
        .start_peer_subscription(&f.user, &command, &proof)
        .unwrap();
    let source = source.unwrap();
    drop(proof);
    f.store
        .execute(
            &bob_key,
            &Operation::Send {
                request_id: "departed-source-message".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text_all("source owner custody"),
            },
        )
        .unwrap();
    let state = f.store.membership_snapshot(&bob_key, &f.cid).unwrap();
    f.store
        .execute_membership(
            &bob_key,
            MembershipRequest {
                operation_id: "source-owner-leave".into(),
                conversation_id: f.cid.clone(),
                expected_revision: state.revision,
                change: MembershipAction::Leave {},
            },
        )
        .unwrap();
    let administrator = f
        .store
        .create_identity("source-retirement-admin")
        .unwrap()
        .id;
    let admin = f
        .store
        .issue(&administrator, Scope::operator(), crate::now() + 300)
        .unwrap()
        .id;
    let read = f
        .store
        .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert_eq!(read.page.events.len(), 1);
    assert!(matches!(
        read.page.events[0].data,
        connect_protocol::events::JournalData::Message(_)
    ));
    assert!(f.store.authority_covers(
        &read.proof._scopes,
        &[AuthorityScope::identity(&bob, AuthorityMode::Read).unwrap()]
    ));
    let store = f.store.clone();
    let id = bob.clone();
    let writer = std::thread::spawn(move || {
        store.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeIdentity {
                operation_id: "retire-departed-source".into(),
                identity_id: id,
                expected_revision: 0,
            },
        )
    });
    crate::authority_fences::test_wait_for_writer(
        &f.store,
        &AuthorityScope::identity(&bob, AuthorityMode::Write).unwrap(),
    );
    let PeerCommand::Subscription(command) = &read.proof.request.command else {
        panic!()
    };
    let start = Instant::now();
    let projected = f
        .store
        .conversation_events_with_peer(&f.user, command, &read.proof)
        .unwrap();
    assert_eq!(projected.events[0].event_id, read.page.events[0].event_id);
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(!writer.is_finished());
    drop(read);
    writer.join().unwrap().unwrap();
    let after = f
        .store
        .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    // Sender retirement stops future actions; it does not retract immutable
    // messages from a recipient's original, still-current audience epoch.
    assert!(matches!(
        after.page.events[0].data,
        connect_protocol::events::JournalData::Message(_)
    ));
}

#[test]
fn subscription_source_growth_retries_same_cursor_without_upgrading_old_guard() {
    use connect_protocol::membership::*;
    let f = Fixture::new(300);
    let bob = f.store.create_identity("reply-source").unwrap().id;
    let bob_key = f
        .store
        .issue(
            &bob,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 300,
        )
        .unwrap()
        .id;
    let state = f.store.membership_snapshot(&f.user, &f.cid).unwrap();
    f.store
        .execute_membership(
            &f.user,
            MembershipRequest {
                operation_id: "add-reply-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: state.revision,
                change: MembershipAction::Add {
                    identity_id: bob.clone(),
                    kind: IdentityKind::Human,
                    role: "participant".into(),
                },
            },
        )
        .unwrap();
    let mut parent = MessageDraft::text_all("retained parent");
    parent.msg_id = Some("departed-parent".into());
    f.store
        .execute(
            &bob_key,
            &Operation::Send {
                request_id: "parent-send".into(),
                conversation_id: f.cid.clone(),
                message: parent,
            },
        )
        .unwrap();
    let state = f.store.membership_snapshot(&bob_key, &f.cid).unwrap();
    f.store
        .execute_membership(
            &bob_key,
            MembershipRequest {
                operation_id: "leave-reply-source".into(),
                conversation_id: f.cid.clone(),
                expected_revision: state.revision,
                change: MembershipAction::Leave {},
            },
        )
        .unwrap();
    let mut command = subscription(&f, 30);
    let mut request = f.request.clone();
    request.command = PeerCommand::Subscription(command.clone());
    let initial = f.admit(&request).unwrap();
    let (page, source) = f
        .store
        .start_peer_subscription(&f.user, &command, &initial)
        .unwrap();
    let source = source.unwrap();
    drop(initial);
    command.query.after = Some(page.cursor.clone());
    command.query.lease = page.lease.clone();
    request.command = PeerCommand::Subscription(command.clone());
    let old = f.admit(&request).unwrap();
    let bob_scope = AuthorityScope::identity(&bob, AuthorityMode::Read).unwrap();
    assert!(!f.store.authority_covers(&old._scopes, &[bob_scope.clone()]));
    let mut reply = MessageDraft::text_all("new fact with a departed source");
    reply.in_reply_to_msg = Some("departed-parent".into());
    f.store
        .execute(
            &f.user,
            &Operation::Send {
                request_id: "reply-send".into(),
                conversation_id: f.cid.clone(),
                message: reply,
            },
        )
        .unwrap();
    // Real journal growth after admission; the original cursor has not moved.
    assert_eq!(
        f.store
            .conversation_events_with_peer(&f.user, &command, &old)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(!f.store.authority_covers(&old._scopes, &[bob_scope.clone()]));
    drop(old);
    let read = f
        .store
        .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert!(f.store.authority_covers(&read.proof._scopes, &[bob_scope]));
    assert_eq!(read.next.expires_at(), source.expires_at());
    assert_eq!(read.page.events.len(), 1);
    let connect_protocol::events::JournalData::Message(value) = &read.page.events[0].data else {
        panic!()
    };
    assert_eq!(value.in_reply_to_msg.as_deref(), Some("departed-parent"));
    let native = f
        .store
        .conversation_events_with_authority(
            &f.user,
            &f.cid,
            command.query.clone(),
            &read.proof._scopes,
        )
        .unwrap();
    assert_eq!(native.events[0].event_id, read.page.events[0].event_id);
    drop(read);
    // Actual original-principal retirement is unauthorized, not retryable.
    f.store
        .revoke(&f.store.revalidate(&f.admin).unwrap(), &f.user)
        .unwrap();
    assert_eq!(
        f.store
            .read_peer_subscription(&source, &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap_err()
            .code,
        "unauthorized"
    );
}

mod read_receipts;

mod session_control;
mod session_decisions;

#[test]
fn single_credential_snapshot_checks_current_projection_without_duplicate_source_captures() {
    let f = Fixture::new(300);
    let query = connect_protocol::events::SnapshotQuery {
        page: None,
        limit: Some(100),
    };
    let mut request = f.request.clone();
    request.command = PeerCommand::Snapshot {
        conversation_id: f.cid.clone(),
        query: query.clone(),
    };
    let proof = f.admit(&request).unwrap();
    let before = crate::events::snapshot_capture_count();
    let page = f
        .store
        .conversation_snapshot_with_peer(&f.user, &f.cid, query, Some(&proof))
        .unwrap();
    assert!(!page.items.is_empty());
    // Current scopes before/after plus one frozen result capture. Source
    // binding/key/device checks must not perform two extra complete captures.
    assert_eq!(crate::events::snapshot_capture_count() - before, 3);
}
#[test]
fn multi_credential_snapshot_keeps_each_credentials_projection_revalidation() {
    let f = Fixture::new(300);
    f.store
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO credential_signing_keys(credential_id,key_id) VALUES(?1,'key')",
            [&f.admin],
        )
        .unwrap();
    let query = connect_protocol::events::SnapshotQuery {
        page: None,
        limit: Some(100),
    };
    let mut request = f.request.clone();
    request.credentials = vec![f.admin.clone(), f.user.clone()];
    request.command = PeerCommand::Snapshot {
        conversation_id: f.cid.clone(),
        query: query.clone(),
    };
    let proof = f.admit(&request).unwrap();
    let before = crate::events::snapshot_capture_count();
    let page = f
        .store
        .conversation_snapshot_with_peer(&f.user, &f.cid, query, Some(&proof))
        .unwrap();
    assert!(!page.items.is_empty());
    // Two credentials are checked twice, in addition to the three captures
    // required by the result/projection executor itself.
    assert_eq!(crate::events::snapshot_capture_count() - before, 7);
}
#[test]
fn snapshot_source_shortcut_cannot_replace_projection_coverage_or_current_key_checks() {
    let f = Fixture::new(300);
    let query = connect_protocol::events::SnapshotQuery {
        page: None,
        limit: Some(100),
    };
    let mut binding = f.request.clone();
    binding.command = PeerCommand::Binding;
    let mut proof = f.admit(&binding).unwrap();
    // A source-only proof is not a projection grant, even with an otherwise
    // correctly named snapshot request. Simulate an invalid internal caller.
    proof.request.command = PeerCommand::Snapshot {
        conversation_id: f.cid.clone(),
        query: query.clone(),
    };
    assert_eq!(
        f.store
            .conversation_snapshot_with_peer(&f.user, &f.cid, query.clone(), Some(&proof))
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(proof);
    let mut request = f.request.clone();
    request.command = PeerCommand::Snapshot {
        conversation_id: f.cid.clone(),
        query: query.clone(),
    };
    let proof = f.admit(&request).unwrap();
    // Bypass ordinary write fencing only in this test to prove the reader
    // still revalidates its current key rather than trusting old admission.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE peer_signing_keys SET revoked=1 WHERE key_id='key'",
            [],
        )
        .unwrap();
    assert_eq!(
        f.store
            .conversation_snapshot_with_peer(&f.user, &f.cid, query, Some(&proof))
            .unwrap_err()
            .code,
        "forbidden"
    );
}
