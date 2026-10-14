//! Account catalogs are independently optional; only read-only loopback fixtures.
#![allow(clippy::unwrap_used)]
use connect_connectors::{provisioning::*, Telnyx, Vapi};
use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn vapi_only_catalog_omits_telnyx_and_cannot_authorize_creates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/assistant"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":"assistant-1","name":"fixture", "private_key":"never-export", "model":{}}
        ])))
        .expect(2)
        .mount(&server)
        .await;
    let vapi = Vapi::new("disposable-fixture", &server.uri()).unwrap();
    let catalog = discover_selected_clients(Some(&vapi), None).await.unwrap();
    assert!(catalog.authenticated && !catalog.complete);
    assert_eq!(catalog.resources.len(), 1);
    assert_eq!(catalog.resources[0].capabilities, vec!["chat"]);
    let intent = Intent {
        profile: "connect-provider-setup/1".into(),
        selections: vec![Selection::Reuse {
            kind: ResourceKind::Assistant,
            id: "assistant-1".into(),
            required_capabilities: vec!["chat".into()],
        }],
    };
    assert!(preview(&catalog, &intent).unwrap().gaps.is_empty());
    let create = Intent {
        selections: vec![Selection::Create {
            kind: ResourceKind::Assistant,
            name: "new".into(),
        }],
        ..intent
    };
    assert!(preview(&catalog, &create).is_err());
    assert!(!serde_json::to_string(&catalog)
        .unwrap()
        .contains("never-export"));
    let legacy = connect_connectors::setup::discover_selected_clients(Some(&vapi), None)
        .await
        .unwrap();
    assert_eq!(legacy["complete"], false);
    assert!(legacy.get("assistants").is_some());
    assert!(legacy.get("phone_numbers").is_none() && legacy.get("connections").is_none());
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.url.path() == "/assistant"));
}

#[tokio::test]
async fn telnyx_only_catalog_omits_assistants_and_unknown_capabilities_stay_unknown() {
    let server = MockServer::start().await;
    for endpoint in [
        "/phone_numbers",
        "/credential_connections",
        "/messaging_profiles",
        "/connections",
    ] {
        Mock::given(method("GET")).and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[{"id":"resource-1", "name":"fixture", "password":"never-export", "model":{}}],
                "meta":{"total_pages":1}
            }))).mount(&server).await;
    }
    let telnyx = Telnyx::new("disposable-fixture", &server.uri()).unwrap();
    let catalog = discover_selected_clients(None, Some(&telnyx))
        .await
        .unwrap();
    assert!(catalog.authenticated && !catalog.complete);
    assert_eq!(catalog.resources.len(), 3);
    assert!(catalog
        .resources
        .iter()
        .all(|r| r.kind != ResourceKind::Assistant && r.capabilities.is_empty()));
    let legacy = connect_connectors::setup::discover_selected_clients(None, Some(&telnyx))
        .await
        .unwrap();
    assert_eq!(legacy["complete"], false);
    assert!(legacy.get("assistants").is_none());
    assert!(legacy.get("phone_numbers").is_some());
    assert!(!serde_json::to_string(&catalog)
        .unwrap()
        .contains("never-export"));
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method == "GET" && r.url.path() != "/assistant"));
    assert!(discover_selected_clients(None, None).await.is_err());
}
