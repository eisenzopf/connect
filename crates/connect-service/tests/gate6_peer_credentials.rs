//! Real local issuer metadata and Store-backed EdDSA/ES256 key verification.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::{provisioning::*, Limits};
use connect_service::{StoreCredentialBinding, StoreKeyResolver, StoreValidator};
use connect_store::{Scope, Store};
use ring::signature::{EcdsaKeyPair, Ed25519KeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use rvoip_auth_core::{sig9421::*, BearerValidator};
use serde_json::json;
use std::sync::Arc;

fn device(store: &Store, admin: &str, identity: &str, id: &str) {
    store
        .execute_provisioning(
            admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("device-{id}"),
                identity_id: identity.into(),
                device: DeviceSpec {
                    id: id.into(),
                    kind: "desktop".into(),
                    platform: "fixture".into(),
                },
                transport: EndpointTransport::Quic,
                address: format!("quic://localhost:4444/{id}"),
                capabilities: vec!["data".into()],
                secret_ref: None,
            },
        )
        .unwrap();
}
fn key(algorithm: SigningAlgorithm) -> (Sig9421Signer, Vec<u8>) {
    let random = ring::rand::SystemRandom::new();
    match algorithm {
        SigningAlgorithm::EdDSA => {
            let bytes = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
            let public = Ed25519KeyPair::from_pkcs8(bytes.as_ref())
                .unwrap()
                .public_key()
                .as_ref()
                .to_vec();
            (
                Sig9421Signer::ed25519("device-key", bytes.as_ref()).unwrap(),
                public,
            )
        }
        SigningAlgorithm::ES256 => {
            let bytes =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &random).unwrap();
            let public =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, bytes.as_ref(), &random)
                    .unwrap()
                    .public_key()
                    .as_ref()
                    .to_vec();
            (
                Sig9421Signer::es256("device-key", bytes.as_ref()).unwrap(),
                public,
            )
        }
    }
}
fn request() -> serde_json::Value {
    json!({"v":2,"type":"message.history","id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":"fixture-conversation","payload":{}})
}

#[tokio::test]
async fn persisted_issuance_and_ed25519_binding_survive_restart_and_refresh_without_borrowing_keys()
{
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let store = Arc::new(Store::initialize(&path, Limits::default()).unwrap());
    let identity = store.create_identity("owner").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let admin = store
        .issue(&identity, Scope::operator(), now + 600)
        .unwrap();
    let first = store
        .issue(&identity, Scope::collaboration(vec!["*".into()]), now + 600)
        .unwrap();
    let second = store
        .issue(&identity, Scope::collaboration(vec!["*".into()]), now + 600)
        .unwrap();
    device(&store, &admin.id, &identity, "device-one");
    let (signer, public) = key(SigningAlgorithm::EdDSA);
    let mut command = ProvisioningCommand::SigningKey {
        operation_id: "key".into(),
        identity_id: identity.clone(),
        credential_id: first.id.clone(),
        device_id: "device-one".into(),
        key_id: "device-key".into(),
        algorithm: SigningAlgorithm::EdDSA,
        public_key: URL_SAFE_NO_PAD.encode(public),
        not_before: now - 1,
        expires_at: now + 300,
        expected_revision: None,
    };
    store
        .execute_provisioning(&admin.id, command.clone())
        .unwrap();
    let credential = StoreValidator::canonical(store.clone())
        .validate_credential(&first.token)
        .await
        .unwrap();
    let issued_at = credential.issued_at.unwrap();
    assert_eq!(credential.token_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(
        issued_at
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        store.credential_metadata(&first.id).unwrap().issued_at
    );
    let resolver = StoreKeyResolver::new(store.clone());
    assert!(resolver.resolve("device-key").is_none());
    Sig9421Verifier::new(resolver.clone())
        .verify_for_principal(&signer.sign(&request()).unwrap(), &credential.principal)
        .await
        .unwrap();
    let next = StoreValidator::canonical(store.clone())
        .validate_credential(&second.token)
        .await
        .unwrap();
    assert!(credential.principal.has_same_owner(&next.principal));
    let next_binding = StoreCredentialBinding::from_validated(&store, &next).unwrap();
    assert!(next_binding
        .resolve_key(&store, "device-key", "EdDSA")
        .is_err());
    if let ProvisioningCommand::SigningKey {
        operation_id,
        credential_id,
        expected_revision,
        ..
    } = &mut command
    {
        *operation_id = "bind-next".into();
        *credential_id = second.id.clone();
        *expected_revision = Some(0);
    }
    store.execute_provisioning(&admin.id, command).unwrap();
    Sig9421Verifier::new(resolver)
        .verify_for_principal(&signer.sign(&request()).unwrap(), &next.principal)
        .await
        .unwrap();
    let backup = dir.path().join("backup.sqlite3");
    let snapshot = store.backup(&backup).unwrap();
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
    drop(store);
    let reopened = Arc::new(Store::open(&path, Limits::default()).unwrap());
    let restored = StoreValidator::canonical(reopened.clone())
        .validate_credential(&first.token)
        .await
        .unwrap();
    assert_eq!(restored.issued_at, Some(issued_at));
    assert_eq!(restored.token_id, credential.token_id);
    let resolver = StoreKeyResolver::new(reopened.clone());
    Sig9421Verifier::new(resolver.clone())
        .verify_for_principal(&signer.sign(&request()).unwrap(), &restored.principal)
        .await
        .unwrap();
    for i in 0..2 {
        let mut spoof = restored.principal.clone();
        match i {
            0 => spoof.issuer = Some("foreign".into()),
            _ => spoof.tenant = Some("tnt_default".into()),
        };
        {
            assert!(resolver
                .resolve_for_principal(&spoof, "device-key", "EdDSA", chrono::Utc::now())
                .is_none());
        }
    }
    reopened
        .execute_provisioning(
            &admin.id,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire-key".into(),
                key_id: "device-key".into(),
                expected_revision: 1,
            },
        )
        .unwrap();
    assert!(Sig9421Verifier::new(resolver)
        .verify_for_principal(&signer.sign(&request()).unwrap(), &restored.principal)
        .await
        .is_err());
}

#[tokio::test]
async fn es256_keys_require_exact_owner_algorithm_current_credential_device_and_validity() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let identity = store.create_identity("owner").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let admin = store
        .issue(&identity, Scope::operator(), now + 600)
        .unwrap();
    let user = store
        .issue(&identity, Scope::collaboration(vec!["*".into()]), now + 600)
        .unwrap();
    device(&store, &admin.id, &identity, "device-one");
    let (signer, public) = key(SigningAlgorithm::ES256);
    let command = ProvisioningCommand::SigningKey {
        operation_id: "key".into(),
        identity_id: identity.clone(),
        credential_id: user.id.clone(),
        device_id: "device-one".into(),
        key_id: "device-key".into(),
        algorithm: SigningAlgorithm::ES256,
        public_key: URL_SAFE_NO_PAD.encode(public),
        not_before: now - 1,
        expires_at: now + 300,
        expected_revision: None,
    };
    let mut binding = command.clone();
    store.execute_provisioning(&admin.id, command).unwrap();
    let validated = StoreValidator::canonical(store.clone())
        .validate_credential(&user.token)
        .await
        .unwrap();
    let principal = validated.principal.clone();
    let user_binding = StoreCredentialBinding::from_validated(&store, &validated).unwrap();
    let resolver = StoreKeyResolver::new(store.clone());
    let bytes = signer.sign(&request()).unwrap();
    Sig9421Verifier::new(resolver.clone())
        .verify_for_principal(&bytes, &principal)
        .await
        .unwrap();
    assert!(resolver
        .resolve_for_principal(&principal, "device-key", "EdDSA", chrono::Utc::now())
        .is_none());
    if let ProvisioningCommand::SigningKey {
        operation_id,
        credential_id,
        expected_revision,
        ..
    } = &mut binding
    {
        *operation_id = "bind-admin".into();
        *credential_id = admin.id.clone();
        *expected_revision = Some(0);
    }
    store.execute_provisioning(&admin.id, binding).unwrap();
    let admin_principal = StoreValidator::canonical(store.clone())
        .validate_principal(&admin.token)
        .await
        .unwrap();
    let actor = store.revalidate(&admin.id).unwrap();
    store.revoke(&actor, &user.id).unwrap();
    assert!(StoreCredentialBinding::from_validated(&store, &validated).is_err());
    assert!(user_binding
        .resolve_key(&store, "device-key", "ES256")
        .is_err());
    Sig9421Verifier::new(resolver.clone())
        .verify_for_principal(&signer.sign(&request()).unwrap(), &admin_principal)
        .await
        .unwrap();
    store
        .execute_provisioning(
            &admin.id,
            ProvisioningCommand::RevokeDevice {
                operation_id: "retire-device".into(),
                device_id: "device-one".into(),
                expected_revision: 0,
            },
        )
        .unwrap();
    assert!(Sig9421Verifier::new(resolver)
        .verify_for_principal(&signer.sign(&request()).unwrap(), &admin_principal)
        .await
        .is_err());
    assert!(StoreValidator::canonical(store)
        .validate_credential(&user.token)
        .await
        .is_err());
}

#[tokio::test]
async fn future_and_expired_keys_never_resolve_from_a_still_current_credential() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let id = store.create_identity("owner").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let admin = store.issue(&id, Scope::operator(), now + 600).unwrap();
    device(&store, &admin.id, &id, "device-one");
    let (_, public) = key(SigningAlgorithm::EdDSA);
    for (name, start, end) in [("future", now + 60, now + 120), ("short", now - 1, now + 3)] {
        store
            .execute_provisioning(
                &admin.id,
                ProvisioningCommand::SigningKey {
                    operation_id: name.into(),
                    identity_id: id.clone(),
                    credential_id: admin.id.clone(),
                    device_id: "device-one".into(),
                    key_id: name.into(),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(&public),
                    not_before: start,
                    expires_at: end,
                    expected_revision: None,
                },
            )
            .unwrap();
    }
    let p = StoreValidator::canonical(store.clone())
        .validate_principal(&admin.token)
        .await
        .unwrap();
    let resolver = StoreKeyResolver::new(store.clone());
    assert!(resolver
        .resolve_for_principal(&p, "future", "EdDSA", chrono::Utc::now())
        .is_none());
    assert!(resolver
        .resolve_for_principal(&p, "short", "EdDSA", chrono::Utc::now())
        .is_some());
    let wait = ((now + 3) * 1000 - chrono::Utc::now().timestamp_millis() + 10).max(0) as u64;
    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    assert!(resolver
        .resolve_for_principal(&p, "short", "EdDSA", chrono::Utc::now())
        .is_none());
    assert!(StoreValidator::canonical(store)
        .validate_credential(&admin.token)
        .await
        .is_ok());
}

// Explicit Store policy composition; listener installation remains separate.
#[tokio::test]
async fn actual_canonical_peer_refresh_keeps_identity_and_requires_a_new_explicit_credential_binding(
) {
    use connect_protocol::{Operation, Outcome};
    use rvoip_uctp::v2::{
        auth_dispatch::AuthenticationMethods,
        auth_peer::{CanonicalPeer, PeerConfig},
        Envelope,
    };
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let identity = store.create_identity("owner").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let admin = store
        .issue(&identity, Scope::operator(), now + 600)
        .unwrap();
    let first = store
        .issue(&identity, Scope::collaboration(vec!["*".into()]), now + 600)
        .unwrap();
    let second = store
        .issue(&identity, Scope::collaboration(vec!["*".into()]), now + 600)
        .unwrap();
    device(&store, &admin.id, &identity, "device-one");
    let (signer, public) = key(SigningAlgorithm::EdDSA);
    let mut enrollment = ProvisioningCommand::SigningKey {
        operation_id: "enroll".into(),
        identity_id: identity.clone(),
        credential_id: first.id.clone(),
        device_id: "device-one".into(),
        key_id: "device-key".into(),
        algorithm: SigningAlgorithm::EdDSA,
        public_key: URL_SAFE_NO_PAD.encode(public),
        not_before: now - 1,
        expires_at: now + 300,
        expected_revision: None,
    };
    store
        .execute_provisioning(&admin.id, enrollment.clone())
        .unwrap();
    let service = connect_service::Service::new(store.clone());
    let handler = connect_service::CanonicalHandler::new(service.clone());
    let Outcome::Conversation(conversation) = service
        .execute(
            first.id.clone(),
            Operation::Create {
                request_id: "conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let (server_signer, _) = key(SigningAlgorithm::EdDSA);
    let config = Arc::new(PeerConfig {
        methods: Arc::new(AuthenticationMethods::new(Some(StoreValidator::canonical(
            store.clone(),
        )))),
        verifier: Arc::new(Sig9421Verifier::new(StoreKeyResolver::new(store.clone()))),
        signer: Arc::new(server_signer),
        authority: connect_service::StorePeerAuthority::new(store.clone(), 8, None)
            .await
            .unwrap(),
        pending_capacity: 8,
    });
    let mut peer = CanonicalPeer::new(config, tokio_util::sync::CancellationToken::new()).unwrap();
    let env = |kind: &str, payload: serde_json::Value| json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload});
    let deadline = || tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    let hello = env(
        "auth.hello",
        json!({"auth_methods":["bearer"],"application_profiles":["connect/2"],"device":{"id":"device-one","kind":"desktop","platform":"fixture","sdk_version":"fixture"}}),
    );
    let output = peer
        .handle(
            &serde_json::to_vec(&hello).unwrap(),
            handler.as_ref(),
            deadline(),
        )
        .await
        .unwrap();
    let challenge = output.envelopes()[0].clone();
    drop(output);
    let mut response = env(
        "auth.response",
        json!({"method":"bearer","credential":first.token}),
    );
    response["in_reply_to"] = json!(challenge.id());
    let output = peer
        .handle(
            &serde_json::to_vec(&signer.sign(&response).unwrap()).unwrap(),
            handler.as_ref(),
            deadline(),
        )
        .await
        .unwrap();
    let session = output.envelopes()[0].clone();
    assert_eq!(session.kind(), "auth.session");
    assert_eq!(session.original()["payload"]["identity_id"], identity);
    drop(output);
    let refresh = || {
        serde_json::to_vec(
            &signer
                .sign(&env(
                    "auth.refresh",
                    json!({"method":"bearer","credential":second.token}),
                ))
                .unwrap(),
        )
        .unwrap()
    };
    assert!(peer
        .handle(&refresh(), handler.as_ref(), deadline())
        .await
        .is_err());
    assert_eq!(peer.generation(), Some(1));
    if let ProvisioningCommand::SigningKey {
        operation_id,
        credential_id,
        expected_revision,
        ..
    } = &mut enrollment
    {
        *operation_id = "bind-next".into();
        *credential_id = second.id.clone();
        *expected_revision = Some(0);
    }
    store.execute_provisioning(&admin.id, enrollment).unwrap();
    let output = peer
        .handle(&refresh(), handler.as_ref(), deadline())
        .await
        .unwrap();
    let renewed = output.envelopes()[0].clone();
    assert_eq!(renewed.kind(), "auth.session");
    assert_eq!(renewed.original()["payload"]["identity_id"], identity);
    assert_ne!(
        renewed.original()["payload"]["session_token"],
        session.original()["payload"]["session_token"]
    );
    drop(output);
    assert_eq!(peer.generation(), Some(2));
    store
        .revoke(&store.revalidate(&admin.id).unwrap(), &first.id)
        .unwrap();
    let mut send = env(
        "message.send",
        json!({"msg_id":"after-refresh","to":"all","content_type":"text/plain","body":"same identity"}),
    );
    send["cid"] = json!(conversation.id);
    send["operation_id"] = json!("after-refresh");
    let wire = serde_json::to_vec(&signer.sign(&send).unwrap()).unwrap();
    assert!(Envelope::parse(&wire).is_ok());
    let output = peer
        .handle(&wire, handler.as_ref(), deadline())
        .await
        .unwrap();
    assert_eq!(output.envelopes()[0].kind(), "ack");
    drop(output);
    let Outcome::History { messages, .. } = service
        .execute(
            second.id,
            Operation::History {
                conversation_id: conversation.id,
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, identity);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn record_delivery_validity_reuses_original_fences_while_an_identity_writer_is_queued() {
    use connect_protocol::{Operation, Outcome};
    use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};
    use rvoip_uctp::v2::{payloads::Device, Envelope, Identifier};
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let owner = store.create_identity("record-owner").unwrap().id;
    let peer = store.create_identity("record-peer").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let admin = store.issue(&owner, Scope::operator(), now + 600).unwrap();
    let first = store
        .issue(&owner, Scope::collaboration(vec!["*".into()]), now + 600)
        .unwrap();
    device(&store, &admin.id, &owner, "device-one");
    let (signer, public) = key(SigningAlgorithm::EdDSA);
    store
        .execute_provisioning(
            &admin.id,
            ProvisioningCommand::SigningKey {
                operation_id: "record-key".into(),
                identity_id: owner.clone(),
                credential_id: first.id.clone(),
                device_id: "device-one".into(),
                key_id: "device-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: now - 1,
                expires_at: now + 300,
                expected_revision: None,
            },
        )
        .unwrap();
    let Outcome::Conversation(room) = store
        .execute(
            &first.id,
            &Operation::Create {
                request_id: "record-room".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    peer.clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!("room");
    };
    let Outcome::Session(session) = store
        .execute(
            &first.id,
            &Operation::SessionInvite {
                request_id: "record-invite".into(),
                conversation_id: room.id.clone(),
                participant_id: peer,
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!("session");
    };
    store
        .execute(
            &first.id,
            &Operation::SessionCancel {
                request_id: "record-cancel".into(),
                conversation_id: room.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    connect_service::Service::new(store.clone())
        .finalize_record_outcomes(1)
        .await
        .unwrap();
    let id = store
        .list_records("tnt_default", &room.id, Some(&session.id))
        .unwrap()
        .pop()
        .unwrap()
        .location
        .strip_prefix("connect:vcon/")
        .unwrap()
        .to_owned();
    let credential = StoreValidator::canonical(store.clone())
        .validate_credential(&first.token)
        .await
        .unwrap();
    let policy = connect_service::StorePeerAuthority::new(store.clone(), 8, None)
        .await
        .unwrap();
    let bound = policy
        .invocation_authority(
            credential.clone(),
            Device {
                id: Identifier::try_from("device-one".to_owned()).unwrap(),
                kind: "desktop".into(),
                platform: "fixture".into(),
                sdk_version: "0.1.0".into(),
            },
        )
        .unwrap();
    let raw=signer.sign(&json!({"v":2,"type":"recording.vcon-fetch","id":"record-current","ts":chrono::Utc::now(),"cid":room.id,"sid":session.id,"payload":{"uuid":id}})).unwrap();
    let envelope = Envelope::from_value(raw).unwrap();
    let lease = bound
        .acquire(&credential.principal, &envelope)
        .await
        .unwrap();
    assert!(lease.current());
    let writer_store = store.clone();
    let writer_owner = owner.clone();
    let writer = tokio::task::spawn_blocking(move || {
        writer_store.acquire_authority(
            &[AuthorityScope::identity(&writer_owner, AuthorityMode::Write).unwrap()],
            &AuthorityWait::new(Duration::from_secs(3)),
        )
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match store.try_acquire_authority(&[
            AuthorityScope::identity(&owner, AuthorityMode::Read).unwrap()
        ]) {
            Err(error) if error.code == "authority-busy" => break,
            Ok(read) => drop(read),
            Err(error) => panic!("unexpected fence failure: {}", error.code),
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "writer never queued"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(!writer.is_finished());
    let start = Instant::now();
    for _ in 0..20 {
        assert!(lease.current());
    }
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "validity attempted a queued policy read"
    );
    drop(lease);
    let held = tokio::time::timeout(Duration::from_secs(2), writer)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(held);
}
