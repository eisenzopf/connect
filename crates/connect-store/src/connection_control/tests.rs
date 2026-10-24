#![allow(clippy::unwrap_used)]
use super::*;
mod assistant_context;
mod consultation_attempts;
mod consultation_audiences;
mod consultation_groups;
mod consultation_joins;
mod consultation_presence;
mod consultation_reads;
mod consultation_returns;
mod handoff_custody;
mod handoff_lifetimes;
mod handoff_preparation;
mod handoff_reads;
mod handoff_roles;
mod handoff_sources;
mod private_consultation;
mod private_provider;
mod replacement_sources;
mod sip_consultation;
use crate::Scope;
use connect_protocol::{
    provisioning::*,
    session_control::{Action, Request},
    Limits, Operation,
};
struct Fixture {
    _dir: tempfile::TempDir,
    admin: String,
    store: Store,
    keys: Vec<String>,
    pids: Vec<String>,
    endpoints: Vec<String>,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        Self::with_endpoint_capabilities(vec!["audio".into(), "video".into()])
    }
    fn with_endpoint_capabilities(capabilities: Vec<String>) -> Self {
        Self::with_identity_kinds(
            capabilities,
            [
                IdentityKind::Ai,
                IdentityKind::External,
                IdentityKind::System,
            ],
        )
    }
    fn with_identity_kinds(capabilities: Vec<String>, kinds: [IdentityKind; 3]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let host = store.create_identity("host").unwrap().id;
        let admin = store
            .issue(&host, Scope::operator(), crate::now() + 3600)
            .unwrap()
            .id;
        let mut identities = vec![host];
        for (index, kind) in kinds.into_iter().enumerate() {
            let ProvisioningOutcome::Identity { identity_id, .. } = store
                .execute_provisioning(
                    &admin,
                    ProvisioningCommand::Identity {
                        operation_id: format!("identity-{index}"),
                        alias: format!("party-{index}"),
                        kind,
                        display_name: None,
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            identities.push(identity_id);
        }
        let mut keys = vec![];
        let mut endpoints = vec![];
        for (index, identity) in identities.iter().enumerate() {
            keys.push(
                store
                    .issue(
                        identity,
                        Scope::collaboration(vec!["*".into()]),
                        crate::now() + 3600,
                    )
                    .unwrap()
                    .id,
            );
            let ProvisioningOutcome::Endpoint { endpoint_id, .. } = store
                .execute_provisioning(
                    &admin,
                    ProvisioningCommand::Endpoint {
                        operation_id: format!("endpoint-{index}"),
                        identity_id: identity.clone(),
                        device: DeviceSpec {
                            id: format!("device-{index}"),
                            kind: "desktop".into(),
                            platform: "isolated-store-fixture".into(),
                        },
                        transport: EndpointTransport::Quic,
                        address: format!("quic://localhost:{}", 4444 + index),
                        capabilities: capabilities.clone(),
                        secret_ref: None,
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            // Controlled provisioning evidence only; this fixture deliberately
            // exercises durable intent, not a physical native/media proof.
            store.connection().unwrap().execute("UPDATE endpoints SET verification_state='verified',verification_evidence='controlled-fixture',verified_at=?2 WHERE id=?1",params![endpoint_id,crate::now()]).unwrap();
            endpoints.push(endpoint_id);
        }
        let Outcome::Conversation(room) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        identities[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let members = store
            .membership_snapshot(&keys[0], &room.id)
            .unwrap()
            .participants;
        let pids = identities
            .iter()
            .map(|id| {
                members
                    .iter()
                    .find(|m| &m.identity_id == id)
                    .unwrap()
                    .participant_id
                    .clone()
            })
            .collect::<Vec<_>>();
        store
            .execute(
                &keys[0],
                &Operation::SessionControl {
                    request: Request {
                        operation_id: "invite".into(),
                        conversation_id: room.id.clone(),
                        session_id: "session".into(),
                        by_participant: None,
                        expected_revision: None,
                        action: Action::Invite {
                            participants: pids[1..].to_vec(),
                            medium: "voice".into(),
                            intent: "conference".into(),
                            capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                        },
                    },
                },
            )
            .unwrap();
        for index in 1..4 {
            store
                .execute(
                    &keys[index],
                    &Operation::SessionControl {
                        request: Request {
                            operation_id: format!("accept-{index}"),
                            conversation_id: room.id.clone(),
                            session_id: "session".into(),
                            by_participant: None,
                            expected_revision: Some(index as u64 - 1),
                            action: Action::Accept {
                                capabilities: json!({"audio":["opus"]}),
                            },
                        },
                    },
                )
                .unwrap();
        }
        Self {
            _dir: dir,
            admin,
            store,
            keys,
            pids,
            endpoints,
            cid: room.id,
        }
    }
    fn request(&self, actor: usize, id: &str) -> PrepareRequest {
        PrepareRequest::new(
            OfferIntent {
                operation_id: format!("prepare-{id}"),
                conversation_id: self.cid.clone(),
                session_id: "session".into(),
                connection_id: format!("connection-{id}"),
                by_participant: Some(self.pids[actor].clone()),
                expected_revision: Some(0),
                offer: json!({"substrate":"quic","capabilities":{},"streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]}]}),
            },
            self.endpoints[actor].clone(),
        )
    }
    fn apply(&self, actor: usize, request: PrepareRequest) -> Result<Outcome> {
        self.store
            .execute(&self.keys[actor], &Operation::ConnectionPrepare { request })
    }
    fn count(&self, table: &str) -> u64 {
        self.store
            .connection()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
#[test]
fn six_durable_slots_for_four_members_replay_one_offer_and_stable_core_mapping_without_readiness() {
    let f = Fixture::new();
    let anchor = f
        .store
        .conversation_events(&f.keys[0], &f.cid, Default::default())
        .unwrap()
        .cursor;
    let requests = [0, 0, 1, 1, 2, 3]
        .into_iter()
        .enumerate()
        .map(|(slot, actor)| (actor, f.request(actor, &slot.to_string())))
        .collect::<Vec<_>>();
    let mut outcomes = vec![];
    for (actor, request) in &requests {
        let outcome = f.apply(*actor, request.clone()).unwrap();
        let Outcome::ConnectionPrepared(value) = &outcome else {
            panic!()
        };
        assert_eq!(value.state, "preparing");
        assert_eq!(value.generation, 1);
        assert_eq!(value.membership_epoch, 1);
        assert_eq!(value.revision, 0);
        assert_eq!(value.participant_id, f.pids[*actor]);
        outcomes.push(outcome);
    }
    assert_eq!(f.count("connections"), 6);
    assert_eq!(f.count("connection_offers"), 6);
    let c = f.store.connection().unwrap();
    let unique: u64 = c
        .query_row(
            "SELECT COUNT(DISTINCT runtime_connection_id) FROM connection_offers",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unique, 6);
    let active:u64=c.query_row("SELECT COUNT(*) FROM connections WHERE state!='preparing' OR observed_runtime_id IS NOT NULL OR readiness_json IS NOT NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(active, 0);
    drop(c);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    for ((actor, request), outcome) in requests.iter().zip(&outcomes) {
        assert_eq!(&f.apply(*actor, request.clone()).unwrap(), outcome);
    }
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let backup = f._dir.path().join("backup.sqlite3");
    f.store.backup(&backup).unwrap();
    assert_eq!(
        Store::inspect_snapshot(&backup).unwrap().digest,
        before.digest
    );
    let events = f
        .store
        .conversation_events(
            &f.keys[0],
            &f.cid,
            connect_protocol::events::EventQuery {
                after: Some(anchor),
                limit: Some(100),
                ..Default::default()
            },
        )
        .unwrap();
    let prepared = events
        .events
        .iter()
        .filter(|e| {
            matches!(
                e.data,
                connect_protocol::events::JournalData::ConnectionPrepared(_)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(prepared.len(), 6);
    assert!(prepared
        .iter()
        .all(|e| e.session_id.as_deref() == Some("session") && e.connection_id.is_some()));
    if let Ok(directory) = std::env::var("CONNECT_GATE6_CONNECTION_PREPARATION_SAMPLES") {
        let snapshot = f
            .store
            .conversation_snapshot(&f.keys[0], &f.cid, Default::default())
            .unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("connection-preparation-facts.json"),
            serde_json::to_vec_pretty(
                &json!({"snapshot":snapshot,"events":events,"prepared_results":outcomes}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn camera_spoof_endpoint_and_stale_revision_refusals_do_not_create_connections() {
    let f = Fixture::new();
    let mut camera = f.request(0, "camera");
    camera.intent.offer["streams_offered"][0]["kind"] = json!("video");
    camera.intent.offer["streams_offered"][0]["codec_preferences"] = json!(["vp8"]);
    assert_eq!(
        f.apply(0, camera).unwrap_err().code,
        "unsupported-media-profile"
    );
    let mut spoof = f.request(0, "spoof");
    spoof.intent.by_participant = Some(f.pids[1].clone());
    assert_eq!(f.apply(0, spoof).unwrap_err().code, "forbidden");
    let mut endpoint = f.request(0, "endpoint");
    endpoint.endpoint_id = f.endpoints[1].clone();
    assert_eq!(f.apply(0, endpoint).unwrap_err().code, "forbidden");
    let mut stale = f.request(0, "stale");
    stale.intent.expected_revision = Some(1);
    let stale_id = stale.preparation_id.clone();
    assert_eq!(f.apply(0, stale).unwrap_err().code, "conflict");
    assert_eq!(f.count("connections"), 0);
    assert_eq!(f.count("connection_offers"), 0);
    let c = f.store.connection().unwrap();
    let metadata:(Option<String>,Option<String>,Option<u64>)=c.query_row("SELECT session_id,connection_id,expected_revision FROM operations WHERE operation_id=?1",[stale_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(metadata, (Some("session".into()), None, Some(1)));
}
#[test]
fn failed_offer_storage_rolls_back_connection_receipt_and_journal_then_original_id_recovers() {
    let f = Fixture::new();
    let request = f.request(0, "recover");
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch("ALTER TABLE connection_offers RENAME TO unavailable_connection_offers")
        .unwrap();
    assert_eq!(
        f.apply(0, request.clone()).unwrap_err().code,
        "storage-unavailable"
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("ALTER TABLE unavailable_connection_offers RENAME TO connection_offers")
        .unwrap();
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let original = f.apply(0, request.clone()).unwrap();
    assert_eq!(f.apply(0, request).unwrap(), original);
    assert_eq!(f.count("connections"), 1);
}
#[test]
fn leaving_one_participant_fences_only_their_preparing_slots_and_preserves_original_receipts() {
    let f = Fixture::new();
    let one = f.request(1, "one");
    let two = f.request(1, "two");
    let other = f.request(2, "other");
    let original = f.apply(1, one.clone()).unwrap();
    f.apply(1, two).unwrap();
    f.apply(2, other).unwrap();
    f.store
        .execute(
            &f.keys[1],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "leave".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(3),
                    action: Action::Leave {
                        participant_id: f.pids[1].clone(),
                    },
                },
            },
        )
        .unwrap();
    let c = f.store.connection().unwrap();
    let fenced:u64=c.query_row("SELECT COUNT(*) FROM connections WHERE participant_id=?1 AND state='retiring' AND generation=2",[&f.pids[1]],|r|r.get(0)).unwrap();
    assert_eq!(fenced, 2);
    let unaffected: String = c
        .query_row(
            "SELECT state FROM connections WHERE id='connection-other'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unaffected, "preparing");
    drop(c);
    assert_eq!(f.apply(1, one).unwrap(), original);
    assert_eq!(
        f.apply(1, f.request(1, "after-leave")).unwrap_err().code,
        "forbidden"
    );
}

impl Fixture {
    fn native_request(&self, seconds: i64) -> crate::PeerAuthorityRequest {
        self.native_request_for(0, seconds, "native")
    }
    fn native_request_for(
        &self,
        actor: usize,
        seconds: i64,
        slot: &str,
    ) -> crate::PeerAuthorityRequest {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let key = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        let identity = self
            .store
            .revalidate(&self.keys[actor])
            .unwrap()
            .identity_id;
        self.store
            .execute_provisioning(
                &self.admin,
                ProvisioningCommand::SigningKey {
                    operation_id: format!("signing-key-{actor}"),
                    identity_id: identity,
                    credential_id: self.keys[actor].clone(),
                    device_id: format!("device-{actor}"),
                    key_id: format!("native-key-{actor}"),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(key.public_key().as_ref()),
                    not_before: crate::now() - 1,
                    expires_at: crate::now() + seconds,
                    expected_revision: None,
                },
            )
            .unwrap();
        crate::PeerAuthorityRequest {
            credentials: vec![self.keys[actor].clone()],
            device: crate::PeerDeviceClaim {
                id: format!("device-{actor}"),
                kind: "desktop".into(),
                platform: "isolated-store-fixture".into(),
            },
            key_id: format!("native-key-{actor}"),
            algorithm: "EdDSA".into(),
            command: crate::PeerCommand::NativeConnectionOffer(self.request(actor, slot).intent),
        }
    }
    fn admit(&self, request: &crate::PeerAuthorityRequest) -> crate::PeerAuthorityGuard {
        self.store
            .admit_peer_authority(
                request,
                &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
            )
            .unwrap()
    }
}
#[test]
fn native_connection_preparation_requires_exact_current_device_key_and_store_proof() {
    let f = Fixture::new();
    let mut request = f.native_request(600);
    let intent = f.request(0, "native").intent;
    request.command = crate::PeerCommand::Binding;
    let binding = f.admit(&request);
    assert_eq!(
        f.store
            .prepare_connection_with_peer(
                &f.keys[0],
                intent.clone(),
                f.endpoints[0].clone(),
                &binding
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(binding);
    request.command = crate::PeerCommand::NativeConnectionOffer(intent.clone());
    let proof = f.admit(&request);
    let original = f
        .store
        .prepare_connection_with_peer(&f.keys[0], intent.clone(), f.endpoints[0].clone(), &proof)
        .unwrap();
    assert_eq!(original.command_type, "connection.prepare");
    assert_ne!(original.operation_id, intent.operation_id);
    let c = f.store.connection().unwrap();
    let public_completed: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id=?1",
            [&intent.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(public_completed, 0);
    let wire_id: String = c
        .query_row(
            "SELECT operation_id FROM connection_offers WHERE connection_id=?1",
            [&intent.connection_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(wire_id, intent.operation_id);
    drop(c);

    assert_eq!(
        original.state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(original.connection_id.as_deref(), Some("connection-native"));
    assert_eq!(original.session_id.as_deref(), Some("session"));
    assert_eq!(
        f.store
            .prepare_connection_with_peer(
                &f.keys[0],
                intent.clone(),
                f.endpoints[0].clone(),
                &proof
            )
            .unwrap(),
        original
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .prepare_connection_with_peer(
                &f.keys[1],
                intent.clone(),
                f.endpoints[0].clone(),
                &proof
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .prepare_connection_with_peer(
                &f.keys[0],
                intent.clone(),
                f.endpoints[1].clone(),
                &proof
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut changed = intent.clone();
    changed.offer["streams_offered"][0]["codec_preferences"] = json!(["g.711-mu"]);
    assert_eq!(
        f.store
            .prepare_connection_with_peer(
                &f.keys[0],
                changed.clone(),
                f.endpoints[0].clone(),
                &proof
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let foreign = Fixture::new();
    assert_eq!(
        foreign
            .store
            .prepare_connection_with_peer(&foreign.keys[0], intent, f.endpoints[0].clone(), &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(proof);
    request.command = crate::PeerCommand::NativeConnectionOffer(changed.clone());
    let changed_proof = f.admit(&request);
    assert_eq!(
        f.store
            .prepare_connection_with_peer(
                &f.keys[0],
                changed,
                f.endpoints[0].clone(),
                &changed_proof
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn native_connection_key_expiry_behind_sql_leaves_no_slot_offer_receipt_or_journal() {
    let f = std::sync::Arc::new(Fixture::new());
    let request = f.native_request(2);
    let proof = f.admit(&request);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let held = f.store.connection().unwrap();
    let worker = f.clone();
    let intent = f.request(0, "native").intent;
    let join = std::thread::spawn(move || {
        worker.store.prepare_connection_with_peer(
            &worker.keys[0],
            intent,
            worker.endpoints[0].clone(),
            &proof,
        )
    });
    std::thread::sleep(std::time::Duration::from_secs(3));
    drop(held);
    assert_eq!(join.join().unwrap().unwrap_err().code, "forbidden");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn retiring_preparations_keep_their_capacity_charge_until_cleanup_is_verified() {
    let mut f = Fixture::new();
    f.store.limits.connections = 2;
    f.apply(0, f.request(0, "capacity-one")).unwrap();
    f.apply(1, f.request(1, "capacity-two")).unwrap();
    f.store
        .execute(
            &f.keys[1],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "capacity-leave".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(3),
                    action: Action::Leave {
                        participant_id: f.pids[1].clone(),
                    },
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.apply(2, f.request(2, "over-capacity")).unwrap_err().code,
        "capacity-exceeded"
    );
    assert_eq!(f.count("connection_offers"), 2);
    assert_eq!(f.count("connections"), 2);
}

#[test]
fn native_connection_projection_requires_completed_exact_preparation_and_preserves_mapping() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .project_connection_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let intent = f.request(0, "native").intent;
    f.store
        .prepare_connection_with_peer(&f.keys[0], intent.clone(), f.endpoints[0].clone(), &proof)
        .unwrap();
    let first = f
        .store
        .project_connection_with_peer(&f.keys[0], &proof)
        .unwrap();
    let repeated = f
        .store
        .project_connection_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(first.runtime_connection_id, repeated.runtime_connection_id);
    assert_ne!(first.runtime_connection_id, intent.connection_id);
    assert_eq!(first.offer, intent.offer);
    assert_eq!(first.connection.connection_id, intent.connection_id);
    assert_eq!(first.connection.endpoint_id, f.endpoints[0]);
    assert_eq!(first.connection.participant_id, f.pids[0]);
    assert_eq!(
        first.session.actor.participant_id,
        first.connection.participant_id
    );
    assert!(first.session.present);
    assert_eq!(first.device_id, "device-0");
    assert!(first.endpoint_capabilities.contains(&"audio".into()));
    assert!(first.accepted_capabilities["audio"]
        .as_array()
        .unwrap()
        .contains(&json!("opus")));
    assert_eq!(first.connection.state, "preparing");
    let c = f.store.connection().unwrap();
    let unobserved: bool = c.query_row("SELECT observed_runtime_id IS NULL AND readiness_json IS NULL FROM connections WHERE id=?1", [&intent.connection_id], |r|r.get(0)).unwrap();
    assert!(unobserved);
}

#[test]
fn native_connection_projection_refuses_binding_other_credential_foreign_store_and_expiry() {
    let f = Fixture::new();
    let mut request = f.native_request(600);
    request.command = crate::PeerCommand::Binding;
    let binding = f.admit(&request);
    assert_eq!(
        f.store
            .project_connection_with_peer(&f.keys[0], &binding)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    drop(binding);
    let intent = f.request(0, "native").intent;
    request.command = crate::PeerCommand::NativeConnectionOffer(intent.clone());
    let mut proof = f.admit(&request);
    f.store
        .prepare_connection_with_peer(&f.keys[0], intent, f.endpoints[0].clone(), &proof)
        .unwrap();
    assert_eq!(
        f.store
            .project_connection_with_peer(&f.keys[1], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let foreign = Fixture::new();
    assert_eq!(
        foreign
            .store
            .project_connection_with_peer(&foreign.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert!(proof.limit_expiry(crate::now()).is_err());
    assert_eq!(
        f.store
            .project_connection_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}

#[test]
fn native_connection_projection_rechecks_slot_generation_instead_of_replaying_receipt_facts() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let proof = f.admit(&request);
    let intent = f.request(0, "native").intent;
    f.store
        .prepare_connection_with_peer(&f.keys[0], intent.clone(), f.endpoints[0].clone(), &proof)
        .unwrap();
    assert!(f
        .store
        .project_connection_with_peer(&f.keys[0], &proof)
        .is_ok());
    // Controlled backend state change; this checks projection fencing only,
    // not actual physical retirement or negotiated media readiness.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET state='retiring',generation=generation+1 WHERE id=?1",
            [&intent.connection_id],
        )
        .unwrap();
    assert_eq!(
        f.store
            .project_connection_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}

mod media_authority;

mod native_wire;

mod native_rejection;
mod native_stream_authority;
