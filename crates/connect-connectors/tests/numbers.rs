#![allow(clippy::unwrap_used)]
use connect_connectors::{numbers::order_with_client, Telnyx};
use serde_json::json;
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};
#[tokio::test]
async fn purchase_checkpoint_prevents_duplicate_orders_and_tracks_exact_sip_connection() {
    let mock = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("order.json");
    let t = Telnyx::new("test", &mock.uri()).unwrap();
    Mock::given(method("POST"))
        .and(path("/number_orders"))
        .and(body_partial_json(
            json!({"connection_id":"123","phone_numbers":[{"phone_number":"+15555550101"}]}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"id":"order-1","status":"pending"}})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/number_orders/order-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"id":"order-1","status":"success"}})),
        )
        .expect(2)
        .mount(&mock)
        .await;
    order_with_client(&t, "+15555550101", "123", &state)
        .await
        .unwrap();
    order_with_client(&t, "+15555550101", "123", &state)
        .await
        .unwrap();
    assert!(order_with_client(&t, "+15555550102", "123", &state)
        .await
        .is_err());
}
#[tokio::test]
async fn purchase_network_uncertainty_is_never_reordered() {
    let mock = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("order.json");
    let t = Telnyx::new("test", &mock.uri()).unwrap();
    Mock::given(method("POST"))
        .and(path("/number_orders"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/number_orders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
        .expect(1)
        .mount(&mock)
        .await;
    assert!(order_with_client(&t, "+15555550101", "123", &state)
        .await
        .is_err());
    assert!(order_with_client(&t, "+15555550101", "123", &state)
        .await
        .is_err());
}
