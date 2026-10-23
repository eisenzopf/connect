//! Explicit tenant readers use actual public APIs without Conversation membership.
use super::*;
fn command(
    identity: &str,
    operation: &str,
    revision: Option<u64>,
    active: bool,
) -> ProvisioningCommand {
    ProvisioningCommand::RecordReadRole {
        operation_id: operation.into(),
        identity_id: identity.into(),
        role: "auditor".into(),
        private_sources: false,
        active,
        expected_revision: revision,
    }
}
fn reader_key(
    r: &RecordFixture,
    identity: &str,
    recipients: Vec<String>,
) -> connect_store::IssuedCredential {
    r.f.server
        .store
        .issue(
            identity,
            Scope {
                operations: vec![
                    "record_get".into(),
                    "record_redact".into(),
                    "record_delete".into(),
                ],
                recipients,
            },
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
}
async fn reference(r: &RecordFixture, client: &mut CanonicalHttpClient) -> Value {
    let result = client
        .request_value(
            "recording.vcon-fetch",
            json!({"uuid":r.uuid}),
            connect_client::canonical::CommandScope {
                conversation_id: Some(r.cid.clone()),
                session_id: Some(r.sid.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    result.ack().unwrap();
    result
        .frames
        .iter()
        .find(|f| f.kind() == "recording.vcon-fetched")
        .unwrap()
        .original()["payload"]
        .clone()
}
fn native(
    r: &RecordFixture,
    identity: &str,
    key: &connect_store::IssuedCredential,
) -> CanonicalClientConfig {
    let admin =
        r.f.server
            .store
            .authenticate(&r.f.admin)
            .unwrap()
            .credential_id;
    let mut config = r.f.client.clone();
    config.identity_id = identity.into();
    config.credential = key.token.clone();
    config.device_id = "record-reader-device".into();
    config.signing_key_id = "record-reader-key".into();
    config.signing_private_key = r.f.temp.path().join("record-reader-key.der");
    let public = signing(&config.signing_private_key, &config.signing_algorithm);
    r.f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: "reader-device".into(),
                identity_id: identity.into(),
                device: DeviceSpec {
                    id: config.device_id.clone(),
                    kind: config.device_kind.clone(),
                    platform: config.device_platform.clone(),
                },
                transport: EndpointTransport::Quic,
                address: format!(
                    "quic://localhost:{}/record-reader",
                    r.f.server.address.port()
                ),
                capabilities: vec!["audio".into()],
                secret_ref: None,
            },
        )
        .unwrap();
    r.f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: "reader-key".into(),
                identity_id: identity.into(),
                credential_id: key.id.clone(),
                device_id: config.device_id.clone(),
                key_id: config.signing_key_id.clone(),
                algorithm: if config.signing_algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 300,
                expected_revision: None,
            },
        )
        .unwrap();
    config
}
async fn sample(r: &RecordFixture, key: &str, request: &ProvisioningCommand, case: &str) {
    let Some(directory) = std::env::var_os("CONNECT_G6_RECORD_READER_SAMPLES") else {
        return;
    };
    let route = format!(
        "/v2/identities/{}/record-read-role",
        match request {
            ProvisioningCommand::RecordReadRole { identity_id, .. } => identity_id,
            _ => unreachable!(),
        }
    );
    let response = reqwest::Client::new()
        .post(format!(
            "http://{}{}",
            r.f.server.http_address.unwrap(),
            route
        ))
        .bearer_auth(key)
        .json(request)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let value: Value = response.json().await.unwrap();
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{}-{case}.json", r.f.config.signing_algorithm)),
        serde_json::to_vec_pretty(
            &json!({"route":route,"case":case,"status":status,"request":request,"response":value}),
        )
        .unwrap(),
    )
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tenant_role_reads_native_http_and_tls_without_membership_and_revalidates_role_revision() {
    for algorithm in ["EdDSA", "ES256"] {
        let r = RecordFixture::new(algorithm, false).await.https(30).await;
        let identity =
            r.f.server
                .store
                .create_identity("outside-record-reader")
                .unwrap()
                .id;
        let key = reader_key(&r, &identity, vec!["*".into()]);
        let config = native(&r, &identity, &key);
        let mut signed = CanonicalClient::open(config.clone()).await.unwrap();
        let mut reader = super::http(&r.f, &key.token)
            .await
            .with_record_tls(r.pin())
            .unwrap();
        assert!(signed
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .is_err());
        assert!(reader
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .is_err());
        let create = command(&identity, "grant-record-reader", None, true);
        let mut admin = mutation_client(&r, &r.f.admin).await;
        assert!(matches!(
            admin.provision(create.clone()).await.unwrap(),
            ProvisioningOutcome::RecordReadRole {
                active: true,
                revision: 1,
                ..
            }
        ));
        sample(&r, &r.f.admin, &create, "create-replay").await;
        sample(&r, &key.token, &create, "ordinary-cannot-provision").await;
        let mut changed = create.clone();
        if let ProvisioningCommand::RecordReadRole {
            private_sources, ..
        } = &mut changed
        {
            *private_sources = true;
        };
        assert!(admin.provision(changed.clone()).await.is_err());
        sample(&r, &r.f.admin, &changed, "changed-intent").await;
        signed.close();
        signed = CanonicalClient::open(config.clone()).await.unwrap();
        let first = signed
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        assert_eq!(
            first.content_hash,
            reader
                .fetch_record(&r.cid, &r.sid, r.selector())
                .await
                .unwrap()
                .content_hash
        );
        let url = reference(&r, &mut reader).await;
        assert_eq!(
            r.downloader()
                .get(url["url"].as_str().unwrap())
                .bearer_auth(&key.token)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::OK
        );
        let narrow = reader_key(&r, &identity, vec![]);
        let mut narrow_reader = super::http(&r.f, &narrow.token)
            .await
            .with_record_tls(r.pin())
            .unwrap();
        let filtered = narrow_reader
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        assert!(!filtered.body.contains(&r.own));
        assert_eq!(
            r.downloader()
                .get(url["url"].as_str().unwrap())
                .bearer_auth(&narrow.token)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::CONFLICT
        );
        let mutation = connect_protocol::record_mutations::Request {
            operation_id: "read-role-not-erasure".into(),
            conversation_id: r.cid.clone(),
            session_id: r.sid.clone(),
            uuid: r.uuid.clone(),
            expected_revision: r.handle().revision,
            action: connect_protocol::record_mutations::Action::Delete,
        };
        assert!(
            matches!(mutation_client(&r,&key.token).await.mutate_record(mutation).await,Err(ClientError::Application(e)) if e.code=="forbidden")
        );
        let revoke = command(&identity, "revoke-record-reader", Some(1), false);
        admin.provision(revoke.clone()).await.unwrap();
        sample(&r, &r.f.admin, &revoke, "revoke-replay").await;
        assert!(reader
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .is_err());
        assert!(signed
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .is_err());
        assert!(matches!(
            admin.provision(create.clone()).await.unwrap(),
            ProvisioningOutcome::RecordReadRole {
                active: false,
                revision: 2,
                ..
            }
        ));
        sample(&r, &r.f.admin, &create, "old-create-stays-revoked").await;
        assert_eq!(
            r.downloader()
                .get(url["url"].as_str().unwrap())
                .bearer_auth(&key.token)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        admin
            .provision(command(
                &identity,
                "reactivate-record-reader",
                Some(2),
                true,
            ))
            .await
            .unwrap();
        signed.close();
        signed = CanonicalClient::open(config.clone()).await.unwrap();
        assert_eq!(
            first.content_hash,
            signed
                .fetch_record(&r.cid, &r.sid, r.selector())
                .await
                .unwrap()
                .content_hash
        );
        assert_eq!(
            r.downloader()
                .get(url["url"].as_str().unwrap())
                .bearer_auth(&key.token)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        assert_eq!(
            rusqlite::Connection::open(&r.f.config.database)
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM participants WHERE identity_id=?1",
                    [identity],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
        signed.shutdown().await.unwrap();
        r.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tenant_role_revocation_waits_for_actual_tls_body_then_invalidates_issued_view() {
    let r = RecordFixture::new("EdDSA", false).await.https(30).await;
    let identity =
        r.f.server
            .store
            .create_identity("held-record-reader")
            .unwrap()
            .id;
    let key = reader_key(&r, &identity, vec!["*".into()]);
    mutation_client(&r, &r.f.admin)
        .await
        .provision(command(&identity, "grant-held-reader", None, true))
        .await
        .unwrap();
    let mut client = super::http(&r.f, &key.token).await;
    let view = reference(&r, &mut client).await;
    let url = view["url"].as_str().unwrap();
    let (stream, length) = r.blocked_https(url, &key.token).await;
    let store = r.f.server.store.clone();
    let admin = store.authenticate(&r.f.admin).unwrap().credential_id;
    let revoke = command(&identity, "revoke-held-reader", Some(1), false);
    let revoking = tokio::task::spawn_blocking(move || store.execute_provisioning(&admin, revoke));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let read = AuthorityScope::identity(&identity, AuthorityMode::Read).unwrap();
    loop {
        match r.f.server.store.try_acquire_authority(&[read.clone()]) {
            Err(e) if e.code == "authority-busy" => break,
            Ok(held) => drop(held),
            Err(e) => panic!("unexpected authority {}", e.code),
        };
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(!revoking.is_finished());
    let bytes = stream.finish(true).await;
    assert_eq!(bytes.len(), length);
    revoking.await.unwrap().unwrap();
    assert_eq!(
        r.downloader()
            .get(url)
            .bearer_auth(&key.token)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    r.f.server.shutdown().await.unwrap();
}
