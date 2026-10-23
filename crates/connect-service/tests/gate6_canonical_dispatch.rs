//! Real signed canonical dispatch into an isolated durable Connect Store.
//! Uses administrator-enrolled Device keys and the owned Store peer policy.
#![allow(clippy::unwrap_used)]
use connect_protocol::{conversations::ListQuery, messages::*, Limits, Operation, Outcome};
use connect_service::{CanonicalHandler, Service, StoreValidator};
use connect_store::{Scope as StoreScope, Store};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rvoip_auth_core::{sig9421::*, BearerValidator};
use rvoip_uctp::v2::{auth_dispatch::*, Envelope};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

struct Peer {
    dispatch: AuthenticatedDispatch,
    signer: Sig9421Signer,
}
impl Peer {
    async fn new(store: &Arc<Store>, credential: rvoip_auth_core::bearer::ValidatedBearer) -> Self {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use connect_protocol::provisioning::*;
        let id = credential.principal.subject.clone();
        let now = chrono::Utc::now().timestamp();
        let admin = store.issue(&id, StoreScope::operator(), now + 600).unwrap();
        let device_id = format!("device_{}", uuid::Uuid::new_v4().simple());
        store
            .execute_provisioning(
                &admin.id,
                ProvisioningCommand::Endpoint {
                    operation_id: format!("enroll-{device_id}"),
                    identity_id: id.clone(),
                    device: DeviceSpec {
                        id: device_id.clone(),
                        kind: "desktop".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: format!("quic://localhost:4444/{device_id}"),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap();
        let key = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let public = Ed25519KeyPair::from_pkcs8(key.as_ref())
            .unwrap()
            .public_key()
            .as_ref()
            .to_vec();
        let key_id = format!("key_{}", uuid::Uuid::new_v4().simple());
        store
            .execute_provisioning(
                &admin.id,
                ProvisioningCommand::SigningKey {
                    operation_id: format!("enroll-{key_id}"),
                    identity_id: id,
                    credential_id: credential.token_id.clone().unwrap(),
                    device_id: device_id.clone(),
                    key_id: key_id.clone(),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(public),
                    not_before: now - 1,
                    expires_at: now + 300,
                    expected_revision: None,
                },
            )
            .unwrap();
        let signer = Sig9421Signer::ed25519(key_id, key.as_ref()).unwrap();
        let device = serde_json::from_value(
            json!({"id":device_id,"kind":"desktop","platform":"fixture","sdk_version":"fixture"}),
        )
        .unwrap();
        let policy = connect_service::StorePeerAuthority::new(store.clone(), 8, None)
            .await
            .unwrap();
        let bound = policy
            .invocation_authority(credential.clone(), device)
            .unwrap();
        let verifier = Arc::new(Sig9421Verifier::new(
            connect_service::StoreKeyResolver::new(store.clone()),
        ));
        let dispatch = AuthenticatedDispatch::new(
            credential.principal,
            verifier,
            bound,
            CancellationToken::new(),
            8,
        )
        .unwrap();
        Self { dispatch, signer }
    }
    async fn try_run(
        &self,
        handler: &CanonicalHandler,
        request: Value,
    ) -> Result<Vec<Envelope>, DispatchError> {
        let bytes = serde_json::to_vec(&self.signer.sign(&request).unwrap()).unwrap();
        match self
            .dispatch
            .dispatch(&bytes, handler, Instant::now() + Duration::from_secs(3))
            .await?
        {
            DispatchOutcome::Handled(reply) => Ok(vec![reply]),
            DispatchOutcome::HandledBatch(batch) => Ok(batch.into_envelopes()),
            DispatchOutcome::ReplyDelivered => panic!("unexpected reply"),
        }
    }
    async fn run(&self, handler: &CanonicalHandler, request: Value) -> Vec<Envelope> {
        self.try_run(handler, request).await.unwrap()
    }
}
fn request(kind: &str, cid: &str, operation: Option<&str>, payload: Value) -> Value {
    let mut v = json!({"v":2,"id":format!("request_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"type":kind,"cid":cid,"payload":payload});
    if let Some(operation) = operation {
        v["operation_id"] = json!(operation);
    }
    v
}
async fn credential(store: &Arc<Store>, token: &str) -> rvoip_auth_core::bearer::ValidatedBearer {
    StoreValidator::canonical(store.clone())
        .validate_credential(token)
        .await
        .unwrap()
}
#[tokio::test]
async fn canonical_signed_send_shares_receipt_with_control_and_history_is_ordered_and_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let mut ids = vec![];
    let mut keys = vec![];
    for alias in ["alice", "bob", "carol"] {
        let id = store.create_identity(alias).unwrap().id;
        keys.push(
            store
                .issue(
                    &id,
                    StoreScope::collaboration(vec!["*".into()]),
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap(),
        );
        ids.push(id);
    }
    let service = Service::new(store.clone());
    let handler = CanonicalHandler::new(service.clone());
    let Outcome::Conversation(c) = service
        .execute(
            keys[0].id.clone(),
            Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(
                    ids[1..].to_vec(),
                ),
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let members = store.membership_snapshot(&keys[0].id, &c.id).unwrap();
    let participant = |i: usize| {
        members
            .participants
            .iter()
            .find(|p| p.identity_id == ids[i])
            .unwrap()
            .participant_id
            .clone()
    };
    let owner = Peer::new(&store, credential(&store, &keys[0].token).await).await;
    let bob = Peer::new(&store, credential(&store, &keys[1].token).await).await;
    let carol = Peer::new(&store, credential(&store, &keys[2].token).await).await;
    for n in 1..=2 {
        let payload = json!({"msg_id":format!("message-{n}"),"from":participant(0),"to":[participant(1)],"content_type":"application/octet-stream","body":"AP8=","body_encoding":"base64","label":"binary"});
        let r = request("message.send", &c.id, Some(&format!("send-{n}")), payload);
        let reply = owner.run(&handler, r.clone()).await;
        assert_eq!(reply[0].kind(), "ack");
        assert_eq!(reply[0].original()["in_reply_to"], r["id"]);
        let mut draft =
            MessageDraft::text(MessageAudience::Participants(vec![participant(1)]), "AP8=");
        draft.body_encoding = BodyEncoding::Base64;
        draft.content_type = "application/octet-stream".into();
        draft.label = "binary".into();
        draft.msg_id = Some(format!("message-{n}"));
        let receipt = service
            .execute(
                keys[0].id.clone(),
                Operation::Send {
                    request_id: format!("send-{n}"),
                    conversation_id: c.id.clone(),
                    message: draft,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            reply[0].original()["payload"]["details"],
            serde_json::to_value(receipt).unwrap()
        );
        let mut replay = r;
        replay["id"] = json!(format!("fresh-{n}"));
        assert_eq!(
            owner.run(&handler, replay).await[0].original()["payload"],
            reply[0].original()["payload"]
        );
    }
    let spoof = owner.run(&handler, request("message.send", &c.id, Some("spoof"), json!({"msg_id":"spoof","from":participant(1),"to":"all","content_type":"text/plain","body":"must not appear"}))).await;
    assert_eq!(spoof[0].original()["payload"]["code"], 403);
    let conflict=owner.run(&handler,request("message.send",&c.id,Some("send-1"),json!({"msg_id":"message-1","to":"all","content_type":"text/plain","body":"changed replay"}))).await;
    assert_eq!(conflict[0].original()["payload"]["code"], 409);
    let mut wrong_connection = request("message.history", &c.id, None, json!({}));
    wrong_connection["connid"] = json!("unvalidated");
    let denied = owner.run(&handler, wrong_connection).await;
    assert_eq!(denied[0].original()["payload"]["code"], 501);
    let history = request("message.history", &c.id, None, json!({"limit":1}));
    let page = bob.run(&handler, history.clone()).await;
    assert_eq!(
        page.iter().map(Envelope::kind).collect::<Vec<_>>(),
        ["message.send", "ack"]
    );
    assert_eq!(page[0].original()["payload"]["msg_id"], "message-1");
    assert_eq!(page[0].original()["payload"]["body"], "AP8=");
    assert!(page[0].original().get("operation_id").is_none());
    for frame in &page {
        assert_eq!(frame.original()["in_reply_to"], history["id"]);
    }
    let cursor = page[1].original()["payload"]["next_cursor"]
        .as_str()
        .unwrap();
    assert!(cursor.len() > 128);
    let second = bob
        .run(
            &handler,
            request(
                "message.history",
                &c.id,
                None,
                json!({"cursor":cursor,"limit":1}),
            ),
        )
        .await;
    assert_eq!(second[0].original()["payload"]["msg_id"], "message-2");
    let denied = carol
        .run(
            &handler,
            request(
                "message.history",
                &c.id,
                None,
                json!({"cursor":cursor,"limit":1}),
            ),
        )
        .await;
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].kind(), "error");
    let empty = carol
        .run(&handler, request("message.history", &c.id, None, json!({})))
        .await;
    assert_eq!(empty.len(), 1);
    assert_eq!(empty[0].kind(), "ack");
    let snapshot = bob
        .run(
            &handler,
            request("conversation.get", &c.id, None, json!({"limit":1})),
        )
        .await;
    assert_eq!(snapshot[0].kind(), "conversation.snapshot");
    assert!(
        snapshot[0].original()["payload"]["cursor"]
            .as_str()
            .unwrap()
            .len()
            > 128
    );
    assert_eq!(
        snapshot[0].original()["payload"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    if let Some(next) = snapshot[0].original()["payload"]["next_page"].as_str() {
        let page = bob
            .run(
                &handler,
                request(
                    "conversation.get",
                    &c.id,
                    None,
                    json!({"page":next,"limit":1}),
                ),
            )
            .await;
        assert_eq!(page[0].kind(), "conversation.snapshot");
    }
    let current = store.membership_snapshot(&keys[0].id, &c.id).unwrap();
    service
        .execute_membership(
            keys[0].id.clone(),
            connect_protocol::membership::MembershipRequest {
                operation_id: "remove-bob".into(),
                conversation_id: c.id.clone(),
                expected_revision: current.revision,
                change: connect_protocol::membership::MembershipAction::Remove {
                    participant_id: participant(1),
                    reason: None,
                },
            },
        )
        .await
        .unwrap();
    let removed = bob
        .try_run(
            &handler,
            request(
                "message.history",
                &c.id,
                None,
                json!({"cursor":cursor,"limit":1}),
            ),
        )
        .await;
    assert_eq!(removed.unwrap_err(), DispatchError::Authority);
    // Revocation denies a fresh envelope even though the backing receipt exists.
    let admin = store
        .issue(
            &ids[0],
            StoreScope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    store
        .revoke(&store.revalidate(&admin.id).unwrap(), &keys[0].id)
        .unwrap();
    let denied = owner
        .try_run(&handler, request("message.history", &c.id, None, json!({})))
        .await;
    assert_eq!(denied.unwrap_err(), DispatchError::Authentication);
}

#[tokio::test]
async fn ordinary_authenticated_composition_rejects_forged_namespace_method_expiry_and_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let id = store.create_identity("owner").unwrap().id;
    let key = store
        .issue(
            &id,
            StoreScope::messaging(vec![]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let good = StoreValidator::new(store.clone())
        .validate_principal(&key.token)
        .await
        .unwrap();
    let service = Service::new(store.clone());
    assert!(service
        .execute_authenticated(good.clone(), Operation::Health {})
        .await
        .is_ok());
    for i in 0..7 {
        let mut p = good.clone();
        match i {
            0 => p.issuer = Some("foreign".into()),
            1 => p.tenant = Some("foreign".into()),
            2 => p.expires_at = None,
            3 => p.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
            4 => p.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            5 => p.method = rvoip_core::AuthenticationMethod::ApiKey,
            _ => p.scopes.push("*".into()),
        }
        assert_eq!(
            service
                .execute_authenticated(p, Operation::Health {})
                .await
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}

#[tokio::test]
async fn canonical_operation_recovery_and_decisions_share_store_receipts_without_silent_scope_loss()
{
    use connect_protocol::{membership::*, transitions::*};
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let identity = store.create_identity("owner").unwrap().id;
    let key = store
        .issue(
            &identity,
            StoreScope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let Outcome::Conversation(conversation) = store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let member = store
        .membership_snapshot(&key.id, &conversation.id)
        .unwrap()
        .participants
        .remove(0);
    let outcome = store
        .execute_membership(
            &key.id,
            MembershipRequest {
                operation_id: "role".into(),
                conversation_id: conversation.id.clone(),
                expected_revision: 0,
                change: MembershipAction::UpdateRole {
                    participant_id: member.participant_id,
                    role: "reviewer".into(),
                },
            },
        )
        .unwrap();
    let peer = Peer::new(&store, credential(&store, &key.token).await).await;
    let handler = CanonicalHandler::new(Service::new(store.clone()));
    let get = request(
        "operation.get",
        &conversation.id,
        None,
        json!({"operation_id":"role"}),
    );
    let reply = peer.run(&handler, get.clone()).await.remove(0);
    assert_eq!(reply.kind(), "operation.updated");
    assert_eq!(reply.original()["in_reply_to"], get["id"]);
    assert_eq!(reply.original()["payload"]["operation_id"], "role");
    assert_eq!(reply.original()["payload"]["state"], "completed");
    assert_eq!(
        reply.original()["payload"]["result"],
        serde_json::to_value(Outcome::Membership(outcome)).unwrap()
    );
    let cancel = request(
        "operation.cancel",
        &conversation.id,
        Some("cancel-role"),
        json!({"operation_id":"role","reason":"same decision"}),
    );
    let receipt = peer.run(&handler, cancel).await.remove(0);
    assert_eq!(receipt.kind(), "operation.updated");
    let control = store
        .execute_operation_action(
            &key.id,
            OperationAction::Cancel {
                operation_id: "cancel-role".into(),
                target_operation_id: "role".into(),
                reason: Some("same decision".into()),
            },
        )
        .unwrap();
    assert_eq!(
        receipt.original()["payload"]["operation_id"],
        control.operation_id
    );
    assert_eq!(
        receipt.original()["payload"]["result"],
        control.result.unwrap()
    );
    let retry = request(
        "operation.cancel",
        &conversation.id,
        Some("cancel-role"),
        json!({"operation_id":"role","reason":"same decision"}),
    );
    assert_eq!(
        peer.run(&handler, retry).await[0].original()["payload"],
        receipt.original()["payload"]
    );
    let changed = request(
        "operation.cancel",
        &conversation.id,
        Some("cancel-role"),
        json!({"operation_id":"role","reason":"changed"}),
    );
    assert_eq!(
        peer.run(&handler, changed).await[0].original()["payload"]["code"],
        409
    );
    let before = Store::inspect_snapshot(store.path()).unwrap();
    let mut revision = request(
        "operation.get",
        &conversation.id,
        None,
        json!({"operation_id":"role"}),
    );
    revision["expected_revision"] = json!(1);
    assert_eq!(
        peer.run(&handler, revision).await[0].original()["payload"]["code"],
        501
    );
    let missing = request(
        "operation.cancel",
        &conversation.id,
        None,
        json!({"operation_id":"role"}),
    );
    // The envelope grammar itself enforces a decision ID for this mutation.
    assert!(peer.try_run(&handler, missing).await.is_err());
    let wrong = request(
        "operation.get",
        "foreign",
        None,
        json!({"operation_id":"role"}),
    );
    assert_eq!(
        peer.try_run(&handler, wrong).await.unwrap_err(),
        DispatchError::Authority
    );
    let mut wrong = request(
        "operation.get",
        &conversation.id,
        Some("other"),
        json!({"operation_id":"role"}),
    );
    wrong["actor_id"] = json!("forged");
    assert_eq!(
        peer.run(&handler, wrong).await[0].original()["payload"]["code"],
        403
    );
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
}

#[tokio::test]
async fn canonical_conversation_family_preserves_policy_roles_pagination_and_close_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let owner = store.create_identity("owner").unwrap().id;
    let other = store.create_identity("other").unwrap().id;
    let token = store
        .issue(
            &owner,
            StoreScope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    let peer = Peer::new(&store, credential(&store, &token.token).await).await;
    let token = token.id;
    let handler = CanonicalHandler::new(Service::new(store.clone()));
    let mut create = request(
        "conversation.create",
        "unused",
        Some("create-rich"),
        json!({"tenant_id":"tnt_default","policy":"ephemeral","idle_close_secs":120,"metadata":{"label":"private content","number":1.0},"initial_participants":[{"identity_id":other,"role":"guest"}]}),
    );
    create.as_object_mut().unwrap().remove("cid");
    let first = peer.run(&handler, create.clone()).await;
    assert_eq!(first.len(), 2);
    assert_eq!(first.last().unwrap().kind(), "ack");
    let frame = first[0].original();
    assert_eq!(frame["type"], "conversation.opened", "{:?}", frame);
    assert_eq!(frame["in_reply_to"], create["id"]);
    assert_eq!(frame["operation_id"], "create-rich");
    assert_eq!(frame["payload"]["policy"], "ephemeral");
    assert_eq!(frame["payload"]["idle_close_secs"], 120);
    assert!(frame["payload"]["participants"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["identity_id"] == other && p["role"] == "guest"));
    let cid = frame["cid"].as_str().unwrap().to_owned();
    create["id"] = json!(format!("retry_{}", uuid::Uuid::new_v4().simple()));
    let replay = peer.run(&handler, create.clone()).await;
    assert_eq!(replay[0].original()["cid"], cid);
    let mut list = request(
        "conversation.list",
        "unused",
        None,
        json!({"filter":{"state":"all"},"limit":1}),
    );
    list.as_object_mut().unwrap().remove("cid");
    let listed = peer.run(&handler, list.clone()).await;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].original()["cid"], cid);
    assert_eq!(listed[1].original()["type"], "ack");
    assert_eq!(listed[1].original()["in_reply_to"], list["id"]);
    let mut bad_list = list.clone();
    bad_list["id"] = json!(format!("bad_{}", uuid::Uuid::new_v4().simple()));
    bad_list["payload"]["cursor"] = json!("tampered_cursor");
    let rejected = peer.run(&handler, bad_list).await;
    assert_eq!(rejected[0].original()["type"], "error");
    assert_eq!(rejected[0].original()["payload"]["code"], 400);
    let revision = store
        .list_conversations(&token, ListQuery::default())
        .unwrap()
        .conversations[0]
        .revision;
    let mut close = request(
        "conversation.close",
        &cid,
        Some("close-rich"),
        json!({"reason_code":77,"reason":"done"}),
    );
    close["expected_revision"] = json!(revision);
    let closed = peer.run(&handler, close.clone()).await;
    assert_eq!(closed[0].original()["type"], "conversation.closed");
    assert_eq!(closed[0].original()["payload"]["reason_code"], 77);
    assert_eq!(closed[0].original()["operation_id"], "close-rich");
    close["id"] = json!(format!("retry_{}", uuid::Uuid::new_v4().simple()));
    assert_eq!(
        peer.run(&handler, close).await[0].original()["payload"],
        closed[0].original()["payload"]
    );
    create["id"] = json!(format!("retry_{}", uuid::Uuid::new_v4().simple()));
    let replay = peer.run(&handler, create).await;
    assert_eq!(replay.len(), 3);
    assert_eq!(replay.last().unwrap().kind(), "ack");
    assert_eq!(replay[1].original()["type"], "conversation.closed");
    list["id"] = json!(format!("list_{}", uuid::Uuid::new_v4().simple()));
    let listed = peer.run(&handler, list).await;
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[1].original()["type"], "conversation.closed");
    assert_eq!(listed[2].original()["type"], "ack");
}

#[tokio::test]
async fn canonical_membership_add_update_remove_share_receipts_and_preserve_reason() {
    use connect_protocol::{
        membership::*, provisioning::IdentityKind, transitions::OperationAction,
    };
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let owner = store.create_identity("owner").unwrap().id;
    let target = store.create_identity("target").unwrap().id;
    let key = store
        .issue(
            &owner,
            StoreScope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    let service = Service::new(store.clone());
    let handler = CanonicalHandler::new(service.clone());
    let peer = Peer::new(&store, credential(&store, &key.token).await).await;
    let Outcome::Conversation(room) = service
        .execute(
            key.id.clone(),
            Operation::Create {
                request_id: "membership-room".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let mut add = request(
        "conversation.participant.add",
        &room.id,
        Some("member-add"),
        json!({"identity_id":target,"kind":"human","role":"guest"}),
    );
    add["expected_revision"] = json!(room.revision);
    let frames = peer.run(&handler, add.clone()).await;
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[2].original()["type"], "ack");
    assert_eq!(
        frames[2].original()["payload"]["details"]["reply_complete"],
        true
    );
    assert_eq!(frames[0].original()["type"], "operation.updated");
    assert_eq!(frames[0].original()["payload"]["state"], "completed");
    assert_eq!(frames[1].original()["payload"]["kind"], "added");
    let result = frames[0].original()["payload"]["result"].clone();
    let Outcome::Membership(added) = serde_json::from_value(result.clone()).unwrap() else {
        panic!()
    };
    let MembershipOutcome::Participant { participant, .. } = &added else {
        panic!()
    };
    let pid = participant.participant_id.clone();
    assert_eq!(participant.role, "guest");
    assert_eq!(added.receipt().unwrap().operation_id, "member-add");
    let control = service
        .execute(
            key.id.clone(),
            Operation::Membership {
                request: MembershipRequest {
                    operation_id: "member-add".into(),
                    conversation_id: room.id.clone(),
                    expected_revision: room.revision,
                    change: MembershipAction::Add {
                        identity_id: target,
                        kind: IdentityKind::Human,
                        role: "guest".into(),
                    },
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(control).unwrap(), result);
    add["id"] = json!(format!("retry_{}", uuid::Uuid::new_v4().simple()));
    assert_eq!(
        peer.run(&handler, add).await[0].original()["payload"]["result"],
        result
    );
    let revision = store
        .membership_snapshot(&key.id, &room.id)
        .unwrap()
        .revision;
    let mut update = request(
        "conversation.participant.update",
        &room.id,
        Some("member-update"),
        json!({"participant_id":pid,"role":"reviewer"}),
    );
    update["expected_revision"] = json!(revision);
    let frames = peer.run(&handler, update).await;
    assert_eq!(frames[1].original()["payload"]["kind"], "updated");
    assert_eq!(
        frames[0].original()["payload"]["result"]["data"]["participant"]["role"],
        "reviewer"
    );
    let revision = store
        .membership_snapshot(&key.id, &room.id)
        .unwrap()
        .revision;
    let mut remove = request(
        "conversation.participant.remove",
        &room.id,
        Some("member-remove"),
        json!({"participant_id":pid,"reason":"application requested removal"}),
    );
    remove["expected_revision"] = json!(revision);
    let frames = peer.run(&handler, remove.clone()).await;
    assert_eq!(frames[1].original()["payload"]["kind"], "removed");
    assert_eq!(
        frames[0].original()["payload"]["result"]["data"]["participant"]["removal_reason"],
        "application requested removal"
    );
    remove["id"] = json!(format!("retry_{}", uuid::Uuid::new_v4().simple()));
    assert_eq!(
        peer.run(&handler, remove.clone()).await[0].original()["payload"],
        frames[0].original()["payload"]
    );
    remove["id"] = json!(format!("changed_{}", uuid::Uuid::new_v4().simple()));
    remove["payload"]["reason"] = json!("changed intent");
    assert_eq!(
        peer.run(&handler, remove).await[0].original()["payload"]["code"],
        409
    );
    let status = store
        .execute_operation_action(
            &key.id,
            OperationAction::Get {
                operation_id: "member-remove".into(),
            },
        )
        .unwrap();
    assert_eq!(
        status.result,
        Some(frames[0].original()["payload"]["result"].clone())
    );
}
