#![allow(clippy::unwrap_used)]
use super::*;
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};
const MUTATION: &str = "original-host-mutation";
const ID: &str = "12345678-1234-1234-1234-123456789015";
fn query() -> VoiceReconcileQuery {
    VoiceReconcileQuery {
        request: VoiceStart {
            assistant_id: "12345678-1234-1234-1234-123456789011".into(),
            phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
            destination: "+15555550101".into(),
        },
        since: 1,
        until: 21,
    }
}
fn row(id: &str) -> Value {
    let q = query();
    json!({"id":id,"name":correlation(MUTATION),"type":"outboundPhoneCall","assistantId":q.request.assistant_id,"phoneNumberId":q.request.phone_number_id,"customer":{"number":q.request.destination},"createdAt":"1970-01-01T00:00:02Z","status":"in-progress","monitor":{"controlUrl":format!("https://fixture.vapi.ai/{id}/control")}})
}
async fn list(server: &MockServer, rows: Vec<Value>) {
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer fixture"))
        .and(path("/call"))
        .and(query_param("assistantId", query().request.assistant_id))
        .and(query_param(
            "phoneNumberId",
            query().request.phone_number_id,
        ))
        .and(query_param("limit", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows))
        .mount(server)
        .await;
}
async fn get(server: &MockServer, body: Value) {
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer fixture"))
        .and(path(format!("/call/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}
#[tokio::test]
async fn lost_create_reply_recovers_exact_correlated_resource_through_read_only_account_lookup() {
    let server = MockServer::start().await;
    let adapter = Vapi::new("fixture", &server.uri()).unwrap();
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer fixture"))
        .and(path("/call"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    assert!(matches!(
        adapter.start_receipt(MUTATION, &query().request).await,
        Err(Failure::Uncertain)
    ));
    let requests = server.received_requests().await.unwrap();
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["name"], correlation(MUTATION));
    assert_eq!(sent["name"].as_str().unwrap().len(), 40);
    assert!(!sent["name"].as_str().unwrap().contains(MUTATION));
    list(&server, vec![row(ID)]).await;
    get(&server, row(ID)).await;
    let r = adapter.reconcile_receipt(MUTATION, &query()).await.unwrap();
    assert_eq!(r.call.id, ID);
    assert_eq!(r.observation.mutation_id, MUTATION);
    assert_eq!(r.observation.verification, Verification::Reconciliation);
    assert_eq!(
        r.observation.state,
        connect_connector_api::lifecycle::State::Voice(
            connect_connector_api::lifecycle::VoiceState::Active
        )
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .count(),
        1
    );
    assert!(requests[1]
        .url
        .query_pairs()
        .any(|(k, _)| k == "createdAtGe"));
    assert!(requests[1]
        .url
        .query_pairs()
        .any(|(k, _)| k == "createdAtLe"));
}
#[tokio::test]
async fn same_caller_or_missing_history_is_never_a_correlated_resource_or_absence_proof() {
    let server = MockServer::start().await;
    let adapter = Vapi::new("fixture", &server.uri()).unwrap();
    let mut unrelated = row(ID);
    unrelated["name"] = json!("some-other-call");
    for rows in [vec![], vec![unrelated]] {
        server.reset().await;
        list(&server, rows).await;
        assert!(matches!(
            adapter.reconcile_receipt(MUTATION, &query()).await,
            Err(Failure::Uncertain)
        ));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/call");
    }
}
#[tokio::test]
async fn multiple_correlated_resources_or_mismatched_original_scope_never_supply_control() {
    let server = MockServer::start().await;
    let adapter = Vapi::new("fixture", &server.uri()).unwrap();
    let mut variants = vec![vec![row(ID), row("12345678-1234-1234-1234-123456789016")]];
    for (field, value) in [
        ("type", json!("inboundPhoneCall")),
        ("assistantId", json!("foreign")),
        ("phoneNumberId", json!("foreign")),
        ("customer", json!({"number":"+15555550999"})),
        ("createdAt", json!("1970-01-01T00:10:00Z")),
        ("id", json!("malformed")),
    ] {
        let mut v = row(ID);
        v[field] = value;
        variants.push(vec![v]);
    }
    for rows in variants {
        server.reset().await;
        list(&server, rows).await;
        assert!(matches!(
            adapter.reconcile_receipt(MUTATION, &query()).await,
            Err(Failure::Uncertain)
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
#[tokio::test]
async fn fresh_resource_read_must_retain_the_full_original_binding() {
    let server = MockServer::start().await;
    let adapter = Vapi::new("fixture", &server.uri()).unwrap();
    list(&server, vec![row(ID)]).await;
    let mut changed = row(ID);
    changed["customer"] = json!({"number":"+15555550999"});
    get(&server, changed).await;
    assert!(matches!(
        adapter.reconcile_receipt(MUTATION, &query()).await,
        Err(Failure::Uncertain)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
#[tokio::test]
async fn saturated_unsplittable_or_over_budget_history_cannot_select_the_first_match() {
    let server = MockServer::start().await;
    let adapter = Vapi::new("fixture", &server.uri()).unwrap();
    list(&server, vec![row(ID); 100]).await;
    assert!(matches!(
        adapter.reconcile_receipt(MUTATION, &query()).await,
        Err(Failure::Uncertain)
    ));
    let requests = server.received_requests().await.unwrap();
    assert!(requests.len() <= 128);
    assert!(requests
        .iter()
        .all(|r| r.method.as_str() == "GET" && r.url.path() == "/call"));
}
#[tokio::test]
async fn complete_split_history_deduplicates_timestamp_ties_and_finds_one_original_call() {
    let server = MockServer::start().await;
    let adapter = Vapi::new("fixture", &server.uri()).unwrap();
    let rows: Vec<_> = (0..151)
        .map(|n| {
            let mut v = row(if n == 0 {
                ID
            } else {
                "12345678-1234-1234-1234-123456789016"
            });
            if n > 0 {
                v["name"] = json!(format!("unrelated-{n}"));
            }
            v["createdAt"] = json!(chrono::DateTime::from_timestamp_millis(2000 + n * 60)
                .unwrap()
                .to_rfc3339());
            v
        })
        .collect();
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer fixture"))
        .and(path("/call"))
        .respond_with(move |r: &wiremock::Request| {
            let q: std::collections::HashMap<_, _> = r.url.query_pairs().into_owned().collect();
            let lo = chrono::DateTime::parse_from_rfc3339(&q["createdAtGe"])
                .unwrap()
                .timestamp_millis();
            let hi = chrono::DateTime::parse_from_rfc3339(&q["createdAtLe"])
                .unwrap()
                .timestamp_millis();
            let filtered: Vec<_> = rows
                .iter()
                .filter(|v| {
                    let t = chrono::DateTime::parse_from_rfc3339(v["createdAt"].as_str().unwrap())
                        .unwrap()
                        .timestamp_millis();
                    t >= lo && t <= hi
                })
                .take(100)
                .cloned()
                .collect();
            ResponseTemplate::new(200).set_body_json(filtered)
        })
        .mount(&server)
        .await;
    get(&server, row(ID)).await;
    assert_eq!(
        adapter
            .reconcile_receipt(MUTATION, &query())
            .await
            .unwrap()
            .call
            .id,
        ID
    );
    assert!(server.received_requests().await.unwrap().len() >= 4);
}
#[tokio::test]
async fn malformed_or_unbounded_query_refuses_before_provider_io() {
    let server = MockServer::start().await;
    let adapter = Vapi::new("fixture", &server.uri()).unwrap();
    for (mutation, q) in [
        ("", query()),
        ("bad\n", query()),
        (
            MUTATION,
            VoiceReconcileQuery {
                until: 10000,
                ..query()
            },
        ),
        (
            MUTATION,
            VoiceReconcileQuery {
                since: -1,
                ..query()
            },
        ),
    ] {
        assert!(matches!(
            adapter.reconcile_receipt(mutation, &q).await,
            Err(Failure::Rejected)
        ));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
