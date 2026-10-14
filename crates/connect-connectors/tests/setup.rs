#![allow(clippy::unwrap_used)]
use connect_connectors::{
    setup::{apply_with_clients, Plan},
    Telnyx, Vapi,
};
use serde_json::json;
#[path = "support/setup_vault.rs"]
mod vault_fixture;
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};
#[tokio::test]
async fn programmatic_resources_are_owned_reconciled_and_never_purchase_numbers() {
    let vault = vault_fixture::FixtureVault::new();
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("setup.json");
    Mock::given(method("POST"))
        .and(path("/assistant"))
        .and(body_partial_json(json!({"metadata":{"connect":true}})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"id":"12345678-1234-1234-1234-123456789012"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/assistant/12345678-1234-1234-1234-123456789012"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"12345678-1234-1234-1234-123456789012"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/messaging_profiles"))
        .and(body_partial_json(
            json!({"webhook_url":"https://test.example/webhooks/telnyx", "whitelisted_destinations":["US"],"webhook_api_version":"2"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"data":{"id":"profile"}})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/credential_connections"))
        .and(body_partial_json(
            json!({"outbound":{"outbound_voice_profile_id":"777"}}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"data":{"id":"sip"}})))
        .expect(1)
        .mount(&server)
        .await;
    for suffix in ["", "/messaging"] {
        Mock::given(method("GET"))
            .and(path(format!("/phone_numbers/123{suffix}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"id":"123","phone_number":"+15555550102"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("PATCH"))
        .and(path("/phone_numbers/123/messaging"))
        .and(body_partial_json(json!({"messaging_profile_id":"profile"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"123"}})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/phone_numbers/123"))
        .and(body_partial_json(json!({"connection_id":"sip"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"123"}})))
        .expect(1)
        .mount(&server)
        .await;
    let plan = Plan {
        messaging_profile_id: None,
        sip_connection_id: None,
        name: "connect".into(),
        assistant_id: None,
        phone_number_id: Some("123".into()),
        callback_url: Some("https://test.example/webhooks/telnyx".into()),
        create_sip: true,
        outbound_voice_profile_id: Some("777".into()),
    };
    let t = Telnyx::new("test", &server.uri()).unwrap();
    let v = Vapi::new("test", &server.uri()).unwrap();
    connect_connectors::setup::apply_with_clients_and_secrets(&plan, &state, &v, &t, &vault)
        .await
        .unwrap();
    connect_connectors::setup::apply_with_clients_and_secrets(&plan, &state, &v, &t, &vault)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert!(value["number_messaging_applied"].as_bool().unwrap());
    assert!(value["number_sip_applied"].as_bool().unwrap());
    assert!(value["previous_number"].is_object());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(state).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[tokio::test]
async fn unknown_creation_outcome_is_reconciled_by_owned_name_before_any_retry() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("setup.json");
    let plan = Plan {
        messaging_profile_id: None,
        sip_connection_id: None,
        name: "connect".into(),
        assistant_id: None,
        phone_number_id: None,
        callback_url: None,
        create_sip: false,
        outbound_voice_profile_id: None,
    };
    let t = Telnyx::new("test", &server.uri()).unwrap();
    let v = Vapi::new("test", &server.uri()).unwrap();
    Mock::given(method("POST"))
        .and(path("/assistant"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert!(apply_with_clients(&plan, &state, &v, &t).await.is_err());
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert_eq!(value["pending"], "assistant");
    Mock::given(method("GET"))
        .and(path("/assistant"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"id":"12345678-1234-1234-1234-123456789012","name":value["name"]}]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/assistant/12345678-1234-1234-1234-123456789012"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"12345678-1234-1234-1234-123456789012"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    apply_with_clients(&plan, &state, &v, &t).await.unwrap();
}

#[tokio::test]
async fn assigned_number_is_not_sms_ready_until_real_campaign_is_carrier_approved() {
    use connect_connectors::setup::check_with_clients;
    for (status, ready) in [
        ("TELNYX_FAILED", false),
        ("TCR_ACCEPTED", false),
        ("MNO_ACCEPTED", true),
    ] {
        let mock = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("setup.json");
        let assistant = "12345678-1234-1234-1234-123456789012";
        std::fs::write(&state,serde_json::to_vec(&json!({"plan":{"name":"test","assistant_id":assistant,"phone_number_id":"123","callback_url":"https://test.example/webhooks/telnyx","create_sip":false,"outbound_voice_profile_id":null},"name":"test","assistant_id":assistant,"messaging_profile_id":"profile","sip_connection_id":null,"sip_user":"unused","sip_password":"unused","pending":null,"number_messaging_applied":true,"number_sip_applied":false,"previous_number":null})).unwrap()).unwrap();
        let rows = [
            (format!("/assistant/{assistant}"), json!({"id":assistant})),
            (
                "/messaging_profiles/profile".into(),
                json!({"data":{"enabled":true,"webhook_url":"https://test.example/webhooks/telnyx"}}),
            ),
            (
                "/phone_numbers/123/messaging".into(),
                json!({"data":{"phone_number":"+14155550101","messaging_profile_id":"profile"}}),
            ),
            (
                "/10dlc/phone_number_campaigns/+14155550101".into(),
                json!({"data":{"phoneNumber":"+14155550101","assignmentStatus":"ASSIGNED","telnyxCampaignId":"campaign"}}),
            ),
            (
                "/10dlc/campaign/campaign".into(),
                json!({"data":{"campaignId":"campaign","campaignStatus":status,"mock":false}}),
            ),
        ];
        for (url, body) in rows {
            Mock::given(method("GET"))
                .and(path(url))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(&mock)
                .await;
        }
        let t = Telnyx::new("fake", &mock.uri()).unwrap();
        let v = Vapi::new("fake", &mock.uri()).unwrap();
        let result = check_with_clients(&state, &v, &t).await.unwrap();
        assert_eq!(result["sms_carrier_ready"], ready);
        assert_eq!(result["carrier_campaign_status"], status);
        assert_eq!(result["number_sms_associated"], true);
    }
}

#[tokio::test]
async fn exact_existing_resources_are_reused_without_edits_or_ownership_claims() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("setup.json");
    let assistant = "12345678-1234-1234-1234-123456789012";
    for endpoint in [
        format!("/assistant/{assistant}"),
        "/messaging_profiles/existing-profile".into(),
        "/credential_connections/existing-trunk".into(),
    ] {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"existing"}})),
            )
            .expect(2)
            .mount(&server)
            .await;
    }
    let plan = Plan {
        name: "reuse".into(),
        assistant_id: Some(assistant.into()),
        messaging_profile_id: Some("existing-profile".into()),
        sip_connection_id: Some("existing-trunk".into()),
        phone_number_id: None,
        callback_url: None,
        create_sip: false,
        outbound_voice_profile_id: None,
    };
    let v = Vapi::new("fixture-only", &server.uri()).unwrap();
    let t = Telnyx::new("fixture-only", &server.uri()).unwrap();
    apply_with_clients(&plan, &state, &v, &t).await.unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    apply_with_clients(&plan, &state, &v, &t).await.unwrap();
    let repeated: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert_eq!(original["operation_id"], repeated["operation_id"]);
    assert!(original.get("sip_password").is_none());
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method == "GET"));
}

fn legacy_checkpoint() -> serde_json::Value {
    json!({"plan":{"name":"connect","assistant_id":"12345678-1234-1234-1234-123456789012","phone_number_id":null,"callback_url":null,"create_sip":true,"outbound_voice_profile_id":null},"name":"connect-owned-legacy","assistant_id":"12345678-1234-1234-1234-123456789012","messaging_profile_id":null,"sip_connection_id":"existing-owned-sip","sip_user":"connect-fixture-user","sip_password":"original-disposable-fixture-password","pending":"sip","number_messaging_applied":false,"number_sip_applied":false,"previous_number":{"selected":"unchanged"}})
}
fn migration_directory() -> tempfile::TempDir {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/gate5-secret-migrations");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}
fn protected_checkpoint(path: &std::path::Path, value: &serde_json::Value) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .unwrap()
        .write_all(&serde_json::to_vec(value).unwrap())
        .unwrap();
}
#[test]
fn legacy_secret_migration_survives_accepted_vault_write_and_preserves_provider_intent() {
    use connect_connectors::setup::SetupSecretStore;
    use std::sync::atomic::Ordering;
    let directory = migration_directory();
    let path = directory.path().join("legacy.json");
    let legacy = legacy_checkpoint();
    protected_checkpoint(&path, &legacy);
    let original = std::fs::read(&path).unwrap();
    let vault = vault_fixture::FixtureVault::new();
    vault.fail_read.store(true, Ordering::SeqCst);
    assert!(connect_connectors::setup::migrate_secrets(&path, &vault).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    vault.fail_read.store(false, Ordering::SeqCst);
    let migrated = connect_connectors::setup::migrate_secrets(&path, &vault).unwrap();
    assert!(migrated.changed);
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(state.get("sip_password").is_none());
    let reference = migrated.secret_reference.unwrap();
    assert_eq!(
        vault.read(&reference).unwrap(),
        legacy["sip_password"].as_str().unwrap()
    );
    for field in [
        "name",
        "pending",
        "sip_connection_id",
        "sip_user",
        "assistant_id",
        "previous_number",
    ] {
        assert_eq!(state[field], legacy[field]);
    }
    assert!(
        !connect_connectors::setup::migrate_secrets(&path, &vault)
            .unwrap()
            .changed
    );
    vault.deny.store(true, Ordering::SeqCst);
    let before = std::fs::read(&path).unwrap();
    assert!(connect_connectors::setup::migrate_secrets(&path, &vault).is_err());
    assert_eq!(before, std::fs::read(&path).unwrap());
}
#[test]
fn missing_vault_restore_requires_original_secret_and_refuses_permissions_or_foreign_intent() {
    use connect_connectors::setup::SetupSecretStore;
    let directory = migration_directory();
    let path = directory.path().join("legacy.json");
    protected_checkpoint(&path, &legacy_checkpoint());
    let vault = vault_fixture::FixtureVault::new();
    let migration = connect_connectors::setup::migrate_secrets(&path, &vault).unwrap();
    let reference = migration.secret_reference.unwrap();
    let before = std::fs::read(&path).unwrap();
    vault.remove(&reference);
    assert!(connect_connectors::setup::migrate_secrets(&path, &vault).is_err());
    assert!(connect_connectors::setup::restore_secret(&path, &vault, "another-password").is_err());
    assert!(vault.read(&reference).is_err());
    connect_connectors::setup::restore_secret(
        &path,
        &vault,
        "original-disposable-fixture-password",
    )
    .unwrap();
    assert_eq!(before, std::fs::read(&path).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(connect_connectors::setup::migrate_secrets(&path, &vault).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut state: serde_json::Value = serde_json::from_slice(&before).unwrap();
    state["sip_password_ref"] = json!("fixture-sip/ffffffffffffffffffffffffffffffff");
    std::fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(connect_connectors::setup::migrate_secrets(&path, &vault).is_err());
}

#[tokio::test]
async fn legacy_discovery_retains_selection_metadata_and_discards_keys_passwords_and_configuration()
{
    let server = MockServer::start().await;
    let secret = "never-export-disposable-provider-secret";
    for endpoint in ["/phone_numbers", "/messaging_profiles", "/connections"] {
        Mock::given(method("GET")).and(path(endpoint)).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data":[{"id":"resource-id", "name":"fixture", "active":true,
                    "features":["voice", {"private_key":secret}], "password":secret, "api_key":secret,
                    "credentials":{"password":secret}}], "meta":{"private_key":secret}
        }))).expect(1).mount(&server).await;
    }
    Mock::given(method("GET")).and(path("/assistant")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"id":"assistant-id", "name":"fixture assistant", "model":{"api_key":secret}, "voice":{"password":secret}}
    ]))).expect(1).mount(&server).await;
    let vapi = Vapi::new("fixture-only", &server.uri()).unwrap();
    let telnyx = Telnyx::new("fixture-only", &server.uri()).unwrap();
    let catalog = connect_connectors::setup::discover_with_clients(&vapi, &telnyx)
        .await
        .unwrap();
    let output = serde_json::to_string(&catalog).unwrap();
    assert!(!output.contains(secret));
    assert!(!output.contains("password") && !output.contains("api_key"));
    assert_eq!(catalog["connections"]["data"][0]["id"], "resource-id");
    assert_eq!(
        catalog["connections"]["data"][0]["features"],
        json!(["voice"])
    );
    assert_eq!(catalog["assistants"][0]["id"], "assistant-id");
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}
