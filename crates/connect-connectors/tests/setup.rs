#![allow(clippy::unwrap_used)]
use connect_connectors::{
    setup::{apply_with_clients, Plan},
    Telnyx, Vapi,
};
use serde_json::json;
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};
#[tokio::test]
async fn programmatic_resources_are_owned_reconciled_and_never_purchase_numbers() {
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
        name: "connect".into(),
        assistant_id: None,
        phone_number_id: Some("123".into()),
        callback_url: Some("https://test.example/webhooks/telnyx".into()),
        create_sip: true,
        outbound_voice_profile_id: Some("777".into()),
    };
    let t = Telnyx::new("test", &server.uri()).unwrap();
    let v = Vapi::new("test", &server.uri()).unwrap();
    apply_with_clients(&plan, &state, &v, &t).await.unwrap();
    apply_with_clients(&plan, &state, &v, &t).await.unwrap();
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
