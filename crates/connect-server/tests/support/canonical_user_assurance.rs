//! Actual installed consent HTTP API, signed peer, Store policy and original journal.
use super::*;
use connect_client::application::ApplicationClient;
use connect_protocol::user_delegation::*;

async fn setup(algorithm: &str) -> (Fixture, String, String, String) {
    let mut f = Fixture::new(algorithm, 8).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = create(&f, &mut client, "assurance-room").await;
    client.close();
    drop(client);
    let user = f
        .server
        .store
        .create_identity("actual-human-consenter")
        .unwrap()
        .id;
    let issued = f
        .server
        .store
        .issue(
            &user,
            Scope {
                operations: vec![
                    "credential.delegate".into(),
                    "close".into(),
                    "history".into(),
                    "operation.get".into(),
                ],
                recipients: vec![f.client.identity_id.clone(), f.peer.identity_id.clone()],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    f.config.bind = f.server.address;
    f.config.http_bind = f.server.http_address;
    f.config.user_assurance = vec![UserAssuranceRule {
        conversation_id: cid.clone(),
        command: "conversation.close".into(),
        user_id: user.clone(),
    }];
    let settings = f.temp.path().join("user-assurance-server.toml");
    std::fs::write(&settings, toml::to_string(&f.config).unwrap()).unwrap();
    f.config = CanonicalServerConfig::load(&settings).unwrap();
    f.server.shutdown().await.unwrap();
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    (f, cid, user, issued.token)
}
async fn consent(
    f: &Fixture,
    user: &str,
    credential: &str,
    operation: &str,
    operations: Vec<String>,
) -> UserDelegationOutcome {
    let mut configuration = canonical_http_config(f);
    configuration.credential = credential.into();
    let mut application = ApplicationClient::open(configuration).await.unwrap();
    let request = UserDelegationRequest {
        operation_id: operation.into(),
        identity_id: f.client.identity_id.clone(),
        operations,
        recipients: vec![f.client.identity_id.clone(), f.peer.identity_id.clone()],
        expires_at: chrono::Utc::now().timestamp() + 300,
    };
    let issued = application
        .issue_user_delegation(user, request.clone())
        .await
        .unwrap();
    let replay = application
        .issue_user_delegation(user, request.clone())
        .await
        .unwrap();
    assert_eq!(issued.credential_id, replay.credential_id);
    assert_eq!(issued.credential, replay.credential);
    assert_eq!(issued.expires_at, replay.expires_at);
    if request.operations.len() > 1 {
        let mut changed = request;
        changed.operations.pop();
        assert!(
            matches!(application.issue_user_delegation(user,changed).await,Err(ClientError::Application(e)) if e.code=="conflict")
        );
    }
    assert!(!format!("{issued:?}").contains(&issued.credential));
    issued
}
fn enroll(f: &mut Fixture, credential: &str, operation: &str) {
    let old = f.server.store.authenticate(&f.client.credential).unwrap();
    let key = f
        .server
        .store
        .resolve_peer_signing_key(
            &old.credential_id,
            "tnt_default",
            &f.client.signing_key_id,
            &f.client.signing_algorithm,
        )
        .unwrap();
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let result = f
        .server
        .store
        .execute_provisioning(
            &admin.credential_id,
            ProvisioningCommand::SigningKey {
                operation_id: operation.into(),
                identity_id: f.client.identity_id.clone(),
                credential_id: credential.into(),
                device_id: f.client.device_id.clone(),
                key_id: f.client.signing_key_id.clone(),
                algorithm: if f.client.signing_algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(key.public_key),
                not_before: key.not_before,
                expires_at: key.expires_at,
                expected_revision: Some(f.key_revision),
            },
        )
        .unwrap();
    let ProvisioningOutcome::SigningKey { revision, .. } = result else {
        panic!("signing binding")
    };
    f.key_revision = revision;
}
async fn challenge(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
) -> connect_client::canonical::ReplyBatch {
    client
        .request(
            payload(
                "conversation.close",
                json!({"reason_code":200,"reason":"explicit-close"}),
            ),
            CommandScope {
                conversation_id: Some(cid.into()),
                operation_id: Some(operation.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
}
fn mutations(f: &Fixture, operation: &str) -> u64 {
    rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE command_type='close' AND operation_id=?1",
            [operation],
            |r| r.get(0),
        )
        .unwrap()
}
#[tokio::test]
async fn production_user_assurance_resumes_original_close_with_real_consent_and_blocks_http_bypass()
{
    for algorithm in ["EdDSA", "ES256"] {
        let (mut f, cid, user, issuer) = setup(algorithm).await;
        let mut http =
            connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
                .await
                .unwrap();
        let refused = http
            .close_conversation("assurance-http-bypass", &cid, Default::default())
            .await;
        assert!(matches!(refused,Err(ClientError::Application(e)) if e.code=="forbidden"));
        assert_eq!(mutations(&f, "assurance-http-bypass"), 0);
        let issued = consent(
            &f,
            &user,
            &issuer,
            "explicit-actual-user-consent",
            vec!["close".into(), "history".into(), "operation.get".into()],
        )
        .await;
        enroll(&mut f, &issued.credential_id, "enroll-actual-consent");
        let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
        let blocked = challenge(&mut client, &cid, "assurance-original-close").await;
        assert_eq!(
            blocked.step_up_request().unwrap().required,
            "user-authorized"
        );
        assert_eq!(mutations(&f, "assurance-original-close"), 0);
        let completed = client
            .complete_step_up(issued.credential.clone())
            .await
            .unwrap();
        assert_eq!(completed.authentication.assurance, "user-authorized");
        assert_eq!(completed.replies.request_id, blocked.request_id);
        completed.replies.ack().unwrap();
        assert_eq!(mutations(&f, "assurance-original-close"), 1);
        let mut successor = f.client.clone();
        successor.credential = issued.credential.clone();
        let mut reconnected = CanonicalClient::open(successor.clone()).await.unwrap();
        let replay = reconnected
            .close_conversation(
                "assurance-original-close",
                &cid,
                connect_client::conversations::CloseOptions {
                    reason_code: Some(200),
                    reason: Some("explicit-close".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(replay.id, cid);
        assert_eq!(mutations(&f, "assurance-original-close"), 1);
        let mut delegated_http_config = canonical_http_config(&f);
        delegated_http_config.credential = issued.credential.clone();
        let mut delegated_http =
            connect_client::canonical::CanonicalHttpClient::open(delegated_http_config)
                .await
                .unwrap();
        let replay = delegated_http
            .close_conversation(
                "assurance-original-close",
                &cid,
                connect_client::conversations::CloseOptions {
                    reason_code: Some(200),
                    reason: Some("explicit-close".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(replay.closed);
        assert_eq!(mutations(&f, "assurance-original-close"), 1);
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let source = f.server.store.authenticate(&issuer).unwrap();
        f.server
            .store
            .revoke(&admin, &source.credential_id)
            .unwrap();
        assert!(f.server.store.authenticate(&issued.credential).is_err());
        assert!(CanonicalClient::open(successor).await.is_err());
        assert!(f.server.store.authenticate(&f.client.credential).is_ok());
        client.close();
        reconnected.close();
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_user_assurance_refuses_ordinary_wrong_user_scope_and_revoked_consent_without_mutation(
) {
    for algorithm in ["EdDSA", "ES256"] {
        for case in [
            "ordinary",
            "wrong-user",
            "missing-scope",
            "revoked-issuer",
            "unbound-key",
        ] {
            let (mut f, cid, user, issuer) = setup(algorithm).await;
            let candidate = if case == "ordinary" {
                f.client.credential.clone()
            } else {
                let (actual_user, actual_issuer) = if case == "wrong-user" {
                    let other = f
                        .server
                        .store
                        .create_identity("different-human-consenter")
                        .unwrap()
                        .id;
                    let key = f
                        .server
                        .store
                        .issue(
                            &other,
                            Scope::operator(),
                            chrono::Utc::now().timestamp() + 600,
                        )
                        .unwrap();
                    (other, key.token)
                } else {
                    (user.clone(), issuer.clone())
                };
                let operations = if case == "missing-scope" {
                    vec!["history".into()]
                } else {
                    vec!["close".into()]
                };
                let issued = consent(
                    &f,
                    &actual_user,
                    &actual_issuer,
                    "denied-actual-consent",
                    operations,
                )
                .await;
                if case != "unbound-key" {
                    enroll(&mut f, &issued.credential_id, "enroll-denied-consent");
                }
                issued.credential
            };
            let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
            let blocked = challenge(&mut client, &cid, "assurance-refused-close").await;
            assert!(blocked.step_up_request().is_some());
            if case == "revoked-issuer" {
                let admin = f.server.store.authenticate(&f.admin).unwrap();
                let source = f.server.store.authenticate(&issuer).unwrap();
                f.server
                    .store
                    .revoke(&admin, &source.credential_id)
                    .unwrap();
            }
            assert!(
                matches!(
                    client.complete_step_up(candidate).await,
                    Err(ClientError::Application(_))
                ),
                "{algorithm}/{case}"
            );
            assert_eq!(mutations(&f, "assurance-refused-close"), 0);
            let original = client.receive().await.unwrap();
            assert_eq!(
                original.scope(rvoip_uctp::v2::Scope::Reply),
                Some(blocked.request_id.as_str())
            );
            assert_eq!(original.kind(), "error");
            let heartbeat = client.keepalive().await.unwrap();
            reply(&mut client, &heartbeat, "ack").await;
            client.close();
            f.server.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn production_consent_http_authenticates_before_body_and_rejects_caller_selected_user() {
    let f = Fixture::new("EdDSA", 8).await;
    let http =
        tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
            .await
            .unwrap();
    let url = format!(
        "http://{}/v2/user-delegations",
        f.server.http_address.unwrap()
    );
    let missing = http
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 401);
    let forbidden = http
        .post(&url)
        .bearer_auth(&f.client.credential)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status().as_u16(), 403);
    let user = f
        .server
        .store
        .create_identity("http-human-consenter")
        .unwrap()
        .id;
    let source = f
        .server
        .store
        .issue(
            &user,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    let selected=http.post(&url).bearer_auth(&source.token).json(&json!({"operation_id":"caller-selected-human","identity_id":f.client.identity_id,"operations":["close"],"recipients":[],"expires_at":chrono::Utc::now().timestamp()+100,"user_id":"different-user"})).send().await.unwrap();
    assert_eq!(selected.status().as_u16(), 400);
    assert_eq!(
        selected.headers()[reqwest::header::CACHE_CONTROL],
        "no-store"
    );
    let count: u64 = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM user_delegations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    f.server.shutdown().await.unwrap();
}
