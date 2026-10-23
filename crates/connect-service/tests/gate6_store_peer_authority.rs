//! Isolated Store policy qualification, including actual length-prefixed TCP.
//! TCP fixtures exercise peer lifetime, not TLS/ALPN or listener installation.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::{provisioning::*, Limits};
use connect_service::{
    CanonicalHandler, Service, StoreKeyResolver, StorePeerAuthority, StoreValidator,
};
use connect_store::{InstallationOwnership, Scope, Store};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rvoip_auth_core::{bearer::ValidatedBearer, sig9421::*, BearerValidator};
use rvoip_uctp::v2::{auth_dispatch::*, auth_peer::*, payloads::Device, Envelope, Identifier};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{timeout, Instant},
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    _dir: tempfile::TempDir,
    root: Arc<InstallationOwnership>,
    store: Arc<Store>,
    admin: String,
    token: String,
    credential: ValidatedBearer,
    device: Device,
    identity: Identifier,
    signer: Sig9421Signer,
    policy: Arc<StorePeerAuthority>,
}
fn envelope(kind: &str, payload: Value) -> Value {
    json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload})
}
fn signing_key() -> (Sig9421Signer, String) {
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    (
        Sig9421Signer::ed25519("device-key", bytes.as_ref()).unwrap(),
        URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
    )
}
impl Fixture {
    async fn new(capacity: usize, key_seconds: i64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        drop(Store::initialize(&path, Limits::default()).unwrap());
        let root = InstallationOwnership::acquire(&path).unwrap();
        let store = Arc::new(Store::open_with_ownership(&path, Limits::default(), &root).unwrap());
        let identity = store.create_identity("owner").unwrap().id;
        let now = chrono::Utc::now().timestamp();
        let admin = store
            .issue(&identity, Scope::operator(), now + 600)
            .unwrap()
            .id;
        let user = store
            .issue(&identity, Scope::collaboration(vec!["*".into()]), now + 600)
            .unwrap();
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
        let (signer, public_key) = signing_key();
        store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::SigningKey {
                    operation_id: "key".into(),
                    identity_id: identity.clone(),
                    credential_id: user.id.clone(),
                    device_id: "device".into(),
                    key_id: "device-key".into(),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key,
                    not_before: now - 1,
                    expires_at: chrono::Utc::now().timestamp() + key_seconds,
                    expected_revision: None,
                },
            )
            .unwrap();
        let credential = StoreValidator::canonical(store.clone())
            .validate_credential(&user.token)
            .await
            .unwrap();
        let policy = StorePeerAuthority::new(store.clone(), capacity, None)
            .await
            .unwrap();
        Self {_dir:dir,root,store,admin,token:user.token,credential,device:serde_json::from_value(json!({"id":"device","kind":"desktop","platform":"fixture","sdk_version":"fixture"})).unwrap(),identity:Identifier::try_from(identity).unwrap(),signer,policy}
    }
    fn response(&self) -> Envelope {
        Envelope::from_value(
            self.signer
                .sign(&envelope(
                    "auth.response",
                    json!({"method":"bearer","credential":self.token}),
                ))
                .unwrap(),
        )
        .unwrap()
    }
    async fn lease(&self) -> AuthorityLease {
        self.policy
            .acquire(
                &self.credential,
                &self.device,
                &self.identity,
                &self.response(),
            )
            .await
            .unwrap()
    }
    async fn retire(&self, kind: usize) {
        let store = self.store.clone();
        let admin = self.admin.clone();
        let identity = self.identity.as_str().to_owned();
        let credential = self.credential.token_id.clone().unwrap();
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || match kind {
            0 => {
                store
                    .execute_provisioning(
                        &admin,
                        ProvisioningCommand::RevokeSigningKey {
                            operation_id: "retire".into(),
                            key_id: "device-key".into(),
                            expected_revision: 0,
                        },
                    )
                    .unwrap();
            }
            1 => {
                store
                    .execute_provisioning(
                        &admin,
                        ProvisioningCommand::RevokeDevice {
                            operation_id: "retire".into(),
                            device_id: "device".into(),
                            expected_revision: 0,
                        },
                    )
                    .unwrap();
            }
            2 => {
                store
                    .revoke(&store.revalidate(&admin).unwrap(), &credential)
                    .unwrap();
            }
            3 => {
                store
                    .execute_provisioning(
                        &admin,
                        ProvisioningCommand::RevokeIdentity {
                            operation_id: "retire".into(),
                            identity_id: identity,
                            expected_revision: 0,
                        },
                    )
                    .unwrap();
            }
            4 => root.retire(),
            _ => {}
        })
        .await
        .unwrap();
    }
}
#[tokio::test]
async fn store_peer_lifetime_capacity_follows_last_clone_and_reclaims_dropped_registration() {
    let f = Fixture::new(1, 300).await;
    let lease = f.lease().await;
    let first = f
        .policy
        .authority_lifetime(&f.credential, &f.device, &f.identity, &lease)
        .unwrap();
    let clone = first.clone();
    drop(first);
    assert!(matches!(
        f.policy
            .authority_lifetime(&f.credential, &f.device, &f.identity, &lease),
        Err(DispatchError::Capacity)
    ));
    assert_eq!(clone.reason(), None);
    drop(clone);
    let next = f
        .policy
        .authority_lifetime(&f.credential, &f.device, &f.identity, &lease)
        .unwrap();
    drop(lease);
    assert_eq!(next.reason(), None);
    f.retire(0).await;
    assert_eq!(
        timeout(Duration::from_secs(2), next.retired())
            .await
            .unwrap(),
        RetirementReason::AuthorityRetired
    );
}
#[tokio::test]
async fn store_peer_rejects_forged_bearer_snapshot_device_and_foreign_policy_lifetime() {
    let f = Fixture::new(8, 300).await;
    for n in 0..4 {
        let mut forged = f.credential.clone();
        match n {
            0 => forged.principal.scopes.push("admin".into()),
            1 => forged.issued_at = forged.issued_at.map(|t| t + Duration::from_secs(1)),
            2 => forged.token_id = Some(f.admin.clone()),
            _ => forged.principal.subject = "other".into(),
        }
        assert!(
            f.policy
                .acquire(&forged, &f.device, &f.identity, &f.response())
                .await
                .is_err(),
            "forged {n}"
        );
    }
    let mut bad = f.device.clone();
    bad.platform = "foreign".into();
    assert!(f.policy.identity(&f.credential, &bad).await.is_err());
    assert!(f
        .policy
        .acquire(&f.credential, &bad, &f.identity, &f.response())
        .await
        .is_err());
    let lease = f.lease().await;
    let other = StorePeerAuthority::new(f.store.clone(), 8, None)
        .await
        .unwrap();
    assert!(other
        .authority_lifetime(&f.credential, &f.device, &f.identity, &lease)
        .is_err());
    f.root.retire();
    assert!(!lease.current());
    assert!(f
        .policy
        .acquire(&f.credential, &f.device, &f.identity, &f.response())
        .await
        .is_err());
}
async fn write(stream: &mut TcpStream, value: &Value) {
    let bytes = serde_json::to_vec(value).unwrap();
    stream.write_u32(bytes.len() as u32).await.unwrap();
    stream.write_all(&bytes).await.unwrap();
}
async fn read(stream: &mut TcpStream) -> Value {
    let len = timeout(Duration::from_secs(10), stream.read_u32())
        .await
        .unwrap()
        .unwrap();
    assert!(len < 1024 * 1024);
    let mut bytes = vec![0; len as usize];
    stream.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_tcp_idle_signed_peers_retire_for_store_key_device_credential_identity_root_and_key_expiry(
) {
    for kind in 0..6 {
        let f = Fixture::new(8, if kind == 5 { 6 } else { 300 }).await;
        let (server_signer, _) = signing_key();
        let config = Arc::new(PeerConfig {
            methods: Arc::new(AuthenticationMethods::new(Some(StoreValidator::canonical(
                f.store.clone(),
            )))),
            verifier: Arc::new(Sig9421Verifier::new(StoreKeyResolver::new(f.store.clone()))),
            signer: Arc::new(server_signer),
            authority: f.policy.clone(),
            pending_capacity: 8,
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handler = CanonicalHandler::new(Service::new(f.store.clone()));
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, writer) = stream.into_split();
            CanonicalPeer::new(config, CancellationToken::new())
                .unwrap()
                .run(reader, writer, handler.as_ref())
                .await
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        write(&mut client,&envelope("auth.hello",json!({"auth_methods":["bearer"],"application_profiles":["connect/2"],"device":{"id":"device","kind":"desktop","platform":"fixture","sdk_version":"fixture"}}))).await;
        let challenge = read(&mut client).await;
        assert_eq!(challenge["type"], "auth.challenge");
        let mut response = envelope(
            "auth.response",
            json!({"method":"bearer","credential":f.token}),
        );
        response["in_reply_to"] = challenge["id"].clone();
        write(&mut client, &f.signer.sign(&response).unwrap()).await;
        assert_eq!(read(&mut client).await["type"], "auth.session");
        f.retire(kind).await;
        let changed = read(&mut client).await;
        assert_eq!(
            changed["type"], "identity.assurance-changed",
            "retirement {kind}"
        );
        assert_eq!(changed["payload"]["current"], "anonymous");
        assert_eq!(changed["payload"]["reason"], "authority-retired");
        assert_eq!(read(&mut client).await["type"], "auth.bye");
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_secs(3), client.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
}
#[tokio::test]
async fn store_peer_key_deadline_invalidates_retained_transport_lease_without_new_store_poll() {
    let f = Fixture::new(8, 6).await;
    let lease = f.lease().await;
    let deadline = lease.deadline().unwrap();
    assert!(lease.current());
    timeout(
        Duration::from_secs(8),
        tokio::time::sleep_until(deadline + Duration::from_millis(10)),
    )
    .await
    .unwrap();
    assert!(!lease.current());
    assert!(Instant::now() >= deadline);
}

struct WaitingResolver {
    resolver: Arc<StoreKeyResolver>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl KeyResolver for WaitingResolver {
    fn resolve(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
    async fn resolve_for_principal_async(
        &self,
        p: &rvoip_core::AuthenticatedPrincipal,
        key: &str,
        algorithm: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<ResolvedVerificationKey> {
        self.entered.notify_one();
        self.release.notified().await;
        self.resolver
            .resolve_for_principal_async(p, key, algorithm, now)
            .await
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signed_verification_retains_original_store_lease_through_async_lookup_and_queued_revocation(
) {
    use connect_protocol::{Operation, Outcome};
    use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};
    let f = Fixture::new(8, 300).await;
    let user = f.credential.token_id.clone().unwrap();
    let Outcome::Conversation(conversation) = f
        .store
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
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let resolver = Arc::new(WaitingResolver {
        resolver: StoreKeyResolver::new(f.store.clone()),
        entered: entered.clone(),
        release: release.clone(),
    });
    let dispatch = AuthenticatedDispatch::new(
        f.credential.principal.clone(),
        Arc::new(Sig9421Verifier::new(resolver)),
        f.policy
            .invocation_authority(f.credential.clone(), f.device.clone())
            .unwrap(),
        CancellationToken::new(),
        8,
    )
    .unwrap();
    let mut send = envelope(
        "message.send",
        json!({"msg_id":"held-verification","to":"all","content_type":"text/plain","body":"one durable send"}),
    );
    send["cid"] = json!(conversation.id);
    send["operation_id"] = json!("held-verification");
    let bytes = serde_json::to_vec(&f.signer.sign(&send).unwrap()).unwrap();
    let handler = CanonicalHandler::new(Service::new(f.store.clone()));
    let pending = tokio::spawn(async move {
        dispatch
            .dispatch(
                &bytes,
                handler.as_ref(),
                Instant::now() + Duration::from_secs(5),
            )
            .await
    });
    timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    let peer = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    let admin = f.admin.clone();
    let writer = tokio::task::spawn_blocking(move || {
        peer.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "revoke".into(),
                key_id: "device-key".into(),
                expected_revision: 0,
            },
        )
    });
    // A bounded fresh reader proves the real writer reserved admission; an
    // elapsed sleep alone would not establish that ordering.
    let store = f.store.clone();
    let identity = f.identity.as_str().to_owned();
    tokio::task::spawn_blocking(move || {
        let end = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            match store.acquire_authority(
                &[AuthorityScope::identity(&identity, AuthorityMode::Read).unwrap()],
                &AuthorityWait::new(Duration::from_millis(15)),
            ) {
                Err(error) if error.code == "authority-busy" => break,
                Ok(guard) => drop(guard),
                Err(error) => panic!("unexpected {}", error.code),
            }
            assert!(std::time::Instant::now() < end, "writer never queued");
            std::thread::yield_now();
        }
    })
    .await
    .unwrap();
    assert!(!writer.is_finished());
    release.notify_one();
    assert!(matches!(
        timeout(Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        DispatchOutcome::Handled(_)
    ));
    writer.await.unwrap().unwrap();
    let Outcome::History { messages, .. } = f
        .store
        .message_history(&user, &conversation.id, &Default::default())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages.len(), 1);
    assert!(f
        .policy
        .acquire(&f.credential, &f.device, &f.identity, &f.response())
        .await
        .is_err());
}

#[tokio::test]
async fn refreshed_store_lifetime_is_independent_of_retired_old_credential_incarnation() {
    let f = Fixture::new(8, 300).await;
    let old_lease = f.lease().await;
    let old = f
        .policy
        .authority_lifetime(&f.credential, &f.device, &f.identity, &old_lease)
        .unwrap();
    drop(old_lease);
    let next = f
        .store
        .issue(
            f.identity.as_str(),
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let next_credential = StoreValidator::canonical(f.store.clone())
        .validate_credential(&next.token)
        .await
        .unwrap();
    let refresh = Envelope::from_value(
        f.signer
            .sign(&envelope(
                "auth.refresh",
                json!({"method":"bearer","credential":next.token}),
            ))
            .unwrap(),
    )
    .unwrap();
    assert!(f
        .policy
        .refresh(
            &f.credential,
            &next_credential,
            &f.device,
            &f.identity,
            &refresh
        )
        .await
        .is_err());
    let meta = f
        .store
        .credential_metadata(f.credential.token_id.as_deref().unwrap())
        .unwrap();
    let key = f
        .store
        .resolve_peer_signing_key(
            f.credential.token_id.as_deref().unwrap(),
            &meta.tenant_id,
            "device-key",
            "EdDSA",
        )
        .unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-next".into(),
                identity_id: f.identity.as_str().into(),
                credential_id: next.id,
                device_id: "device".into(),
                key_id: "device-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(key.public_key),
                not_before: key.not_before,
                expires_at: key.expires_at,
                expected_revision: Some(0),
            },
        )
        .unwrap();
    let union = f
        .policy
        .refresh(
            &f.credential,
            &next_credential,
            &f.device,
            &f.identity,
            &refresh,
        )
        .await
        .unwrap();
    let successor = f
        .policy
        .authority_lifetime(&next_credential, &f.device, &f.identity, &union)
        .unwrap();
    drop(union);
    f.retire(2).await;
    assert_eq!(
        timeout(Duration::from_secs(2), old.retired())
            .await
            .unwrap(),
        RetirementReason::AuthorityRetired
    );
    assert_eq!(successor.reason(), None);
    assert!(f
        .policy
        .acquire(&next_credential, &f.device, &f.identity, &refresh)
        .await
        .unwrap()
        .current());
    f.root.retire();
    assert_eq!(successor.reason(), Some(RetirementReason::AuthorityRetired));
}

struct FactsOnly(connect_service::StoreCredentialBinding);
#[async_trait::async_trait]
impl CurrentAuthority for FactsOnly {
    async fn acquire(
        &self,
        _: &rvoip_core::AuthenticatedPrincipal,
        _: &Envelope,
    ) -> Result<AuthorityLease, DispatchError> {
        // Deliberately lacks installed peer proof; a public facts carrier must
        // not suffice even with a valid signature and legitimate credential.
        Ok(AuthorityLease::holding(
            connect_service::StoreInvocationAuthority::holding(self.0.clone(), ()),
        ))
    }
}
#[tokio::test]
async fn signed_handler_rejects_facts_only_public_carrier_without_resource_mutation() {
    use connect_protocol::{Operation, Outcome};
    let f = Fixture::new(8, 300).await;
    let user = f.credential.token_id.clone().unwrap();
    let Outcome::Conversation(conversation) = f
        .store
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
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let dispatch = AuthenticatedDispatch::new(
        f.credential.principal.clone(),
        Arc::new(Sig9421Verifier::new(StoreKeyResolver::new(f.store.clone()))),
        Arc::new(FactsOnly(
            connect_service::StoreCredentialBinding::from_validated(&f.store, &f.credential)
                .unwrap(),
        )),
        CancellationToken::new(),
        8,
    )
    .unwrap();
    let mut send = envelope(
        "message.send",
        json!({"msg_id":"forged-proof","to":"all","content_type":"text/plain","body":"must not commit"}),
    );
    send["cid"] = json!(conversation.id);
    send["operation_id"] = json!("forged-proof");
    let bytes = serde_json::to_vec(&f.signer.sign(&send).unwrap()).unwrap();
    let handler = CanonicalHandler::new(Service::new(f.store.clone()));
    let DispatchOutcome::Handled(error) = dispatch
        .dispatch(
            &bytes,
            handler.as_ref(),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(error.kind(), "error");
    assert_eq!(error.original()["payload"]["code"], 403);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[tokio::test]
async fn retired_peer_transport_lease_holds_real_root_until_cleanup_and_old_policy_cannot_adopt_restart(
) {
    let f = Fixture::new(8, 300).await;
    let lease = f.lease().await;
    let path = f.store.path().to_owned();
    let response = f.response();
    f.root.retire();
    drop(f.root);
    assert!(!lease.current());
    assert_eq!(
        InstallationOwnership::acquire(&path).unwrap_err().code,
        "installation-busy"
    );
    drop(lease);
    let next = InstallationOwnership::acquire(&path).unwrap();
    let current = Store::open_with_ownership(&path, Limits::default(), &next).unwrap();
    assert!(current.installation_id().is_ok());
    assert!(f.store.installation_id().is_err());
    assert!(f
        .policy
        .acquire(&f.credential, &f.device, &f.identity, &response)
        .await
        .is_err());
}

#[tokio::test]
async fn refresh_union_honors_shorter_old_source_expiry_but_successor_lifetime_uses_renewed_credential(
) {
    let mut f = Fixture::new(8, 300).await;
    let meta = f
        .store
        .credential_metadata(f.credential.token_id.as_deref().unwrap())
        .unwrap();
    let key = f
        .store
        .resolve_peer_signing_key(
            f.credential.token_id.as_deref().unwrap(),
            &meta.tenant_id,
            "device-key",
            "EdDSA",
        )
        .unwrap();
    let next = f
        .store
        .issue(
            f.identity.as_str(),
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-next".into(),
                identity_id: f.identity.as_str().into(),
                credential_id: next.id,
                device_id: "device".into(),
                key_id: "device-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(key.public_key),
                not_before: key.not_before,
                expires_at: key.expires_at,
                expected_revision: Some(0),
            },
        )
        .unwrap();
    let next_credential = StoreValidator::canonical(f.store.clone())
        .validate_credential(&next.token)
        .await
        .unwrap();
    let refresh = Envelope::from_value(
        f.signer
            .sign(&envelope(
                "auth.refresh",
                json!({"method":"bearer","credential":next.token}),
            ))
            .unwrap(),
    )
    .unwrap();
    f.credential.principal.expires_at = Some(chrono::Utc::now() + chrono::Duration::seconds(3));
    let union = f
        .policy
        .refresh(
            &f.credential,
            &next_credential,
            &f.device,
            &f.identity,
            &refresh,
        )
        .await
        .unwrap();
    let proof = union
        .authority::<connect_service::StoreInvocationAuthority>()
        .unwrap();
    assert_eq!(
        proof.binding().credential_id(),
        next_credential.token_id.as_deref().unwrap()
    );
    let successor = f
        .policy
        .authority_lifetime(&next_credential, &f.device, &f.identity, &union)
        .unwrap();
    let end = union.deadline().unwrap();
    assert!(end < Instant::now() + Duration::from_secs(4));
    tokio::time::sleep_until(end + Duration::from_millis(10)).await;
    assert!(!union.current());
    assert_eq!(successor.reason(), None);
    drop(union);
    assert!(f
        .policy
        .acquire(&next_credential, &f.device, &f.identity, &refresh)
        .await
        .unwrap()
        .current());
}

struct WriteGate {
    blocked: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
}
struct GatedSocketWriter {
    socket: tokio::net::tcp::OwnedWriteHalf,
    gate: Arc<WriteGate>,
}
impl tokio::io::AsyncWrite for GatedSocketWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.gate.blocked.load(std::sync::atomic::Ordering::Acquire) {
            self.gate.entered.notify_one();
            return std::task::Poll::Pending;
        }
        std::pin::Pin::new(&mut self.socket).poll_write(cx, bytes)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_tcp_blocked_signed_response_holds_revocation_until_key_deadline_closes_socket_and_releases_proof(
) {
    use connect_protocol::{Operation, Outcome};
    let f = Fixture::new(8, 10).await;
    let user = f.credential.token_id.clone().unwrap();
    let Outcome::Conversation(conversation) = f
        .store
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
    let (server_signer, _) = signing_key();
    let config = Arc::new(PeerConfig {
        methods: Arc::new(AuthenticationMethods::new(Some(StoreValidator::canonical(
            f.store.clone(),
        )))),
        verifier: Arc::new(Sig9421Verifier::new(StoreKeyResolver::new(f.store.clone()))),
        signer: Arc::new(server_signer),
        authority: f.policy.clone(),
        pending_capacity: 8,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handler = CanonicalHandler::new(Service::new(f.store.clone()));
    let gate = Arc::new(WriteGate {
        blocked: std::sync::atomic::AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
    });
    let server_gate = gate.clone();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, socket) = stream.into_split();
        CanonicalPeer::new(config, CancellationToken::new())
            .unwrap()
            .run(
                reader,
                GatedSocketWriter {
                    socket,
                    gate: server_gate,
                },
                handler.as_ref(),
            )
            .await
    });
    let mut client = TcpStream::connect(address).await.unwrap();
    write(&mut client,&envelope("auth.hello",json!({"auth_methods":["bearer"],"application_profiles":["connect/2"],"device":{"id":"device","kind":"desktop","platform":"fixture","sdk_version":"fixture"}}))).await;
    let challenge = read(&mut client).await;
    let mut response = envelope(
        "auth.response",
        json!({"method":"bearer","credential":f.token}),
    );
    response["in_reply_to"] = challenge["id"].clone();
    write(&mut client, &f.signer.sign(&response).unwrap()).await;
    assert_eq!(read(&mut client).await["type"], "auth.session");
    gate.blocked
        .store(true, std::sync::atomic::Ordering::Release);
    let mut history = envelope("message.history", json!({}));
    history["cid"] = json!(conversation.id);
    write(&mut client, &f.signer.sign(&history).unwrap()).await;
    timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let peer = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    let admin = f.admin.clone();
    let error = tokio::task::spawn_blocking(move || {
        peer.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire".into(),
                key_id: "device-key".into(),
                expected_revision: 0,
            },
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_secs(12), client.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0,
        "no blocked response bytes may escape on expiry"
    );
    assert!(timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    f.retire(0).await;
}
