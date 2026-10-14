//! Fixtures distinguish unknown, rejected, cancelled and reconciled outcomes.
#![allow(clippy::unwrap_used)]
use connect_connectors::provisioning::*;
use std::sync::Mutex;
struct Fixture {
    calls: Mutex<Vec<String>>,
    accepted: Mutex<Option<Receipt>>,
    unknown: bool,
    reject: bool,
}
#[async_trait::async_trait]
impl Provisioner for Fixture {
    async fn attempt(&self, id: &str, _: &Selection) -> anyhow::Result<Attempt> {
        self.calls.lock().unwrap().push(id.into());
        if self.reject {
            return Ok(Attempt::Rejected);
        }
        let receipt = Receipt {
            resource_id: "owned-fixture".into(),
            created: true,
        };
        *self.accepted.lock().unwrap() = Some(receipt.clone());
        if self.unknown {
            anyhow::bail!("fixture crash after acceptance");
        }
        Ok(Attempt::Accepted(receipt))
    }
    async fn reconcile(&self, _: &str, _: &Selection) -> anyhow::Result<Reconciliation> {
        Ok(self
            .accepted
            .lock()
            .unwrap()
            .clone()
            .map(Reconciliation::Accepted)
            .unwrap_or(Reconciliation::Unknown))
    }
}
fn intent() -> Intent {
    Intent {
        profile: "connect-provider-setup/1".into(),
        selections: vec![Selection::Create {
            kind: ResourceKind::Assistant,
            name: "isolated".into(),
        }],
    }
}
#[tokio::test]
async fn accepted_before_receipt_is_reconciled_without_duplicate_create() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup.json");
    let fixture = Fixture {
        calls: Mutex::new(vec![]),
        accepted: Mutex::new(None),
        unknown: true,
        reject: false,
    };
    let intent = intent();
    assert!(execute(&path, &intent, &fixture).await.is_err());
    let first = inspect_execution(&path).unwrap();
    assert_eq!(first.steps[0].state, StepState::Running);
    let recovered = execute(&path, &intent, &fixture).await.unwrap();
    assert_eq!(first.operation_id, recovered.operation_id);
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    assert_eq!(cleanup_owned(&recovered).unwrap().len(), 1);
    assert_eq!(execute(&path, &intent, &fixture).await.unwrap(), recovered);
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    let mut changed = intent.clone();
    changed.selections.push(Selection::Create {
        kind: ResourceKind::Trunk,
        name: "different".into(),
    });
    assert!(execute(&path, &changed, &fixture).await.is_err());
}
#[tokio::test]
async fn uncertain_definite_failure_and_cancellation_remain_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup.json");
    let fixture = Fixture {
        calls: Mutex::new(vec![]),
        accepted: Mutex::new(None),
        unknown: false,
        reject: true,
    };
    assert_eq!(
        execute(&path, &intent(), &fixture).await.unwrap().steps[0].state,
        StepState::Failed
    );
    execute(&path, &intent(), &fixture).await.unwrap();
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    let mut state = inspect_execution(&path).unwrap();
    state.steps[0].state = StepState::Running;
    std::fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(
        execute(&path, &intent(), &fixture).await.unwrap().steps[0].state,
        StepState::Running
    );
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    assert!(cleanup_owned(&cancel(&path).unwrap()).is_err());
    assert!(execute(&path, &intent(), &fixture).await.unwrap().cancelled);
    *fixture.accepted.lock().unwrap() = Some(Receipt {
        resource_id: "reconciled-owned".into(),
        created: true,
    });
    let reconciled = reconcile_cancelled(&path, &fixture).await.unwrap();
    assert_eq!(cleanup_owned(&reconciled).unwrap().len(), 1);
    assert_eq!(
        fixture.calls.lock().unwrap().len(),
        1,
        "cancelled reconciliation makes no new attempt"
    );
}
#[test]
fn exact_preview_is_read_only_bounded_and_has_no_implicit_number_purchase() {
    let empty = Catalog {
        authenticated: true,
        complete: true,
        resources: vec![],
    };
    assert!(preview(&empty, &intent()).is_ok());
    assert!(preview(
        &Catalog {
            authenticated: false,
            ..empty.clone()
        },
        &intent()
    )
    .is_err());
    assert!(preview(
        &Catalog {
            complete: false,
            ..empty.clone()
        },
        &intent()
    )
    .is_err());
    let mut catalog = empty;
    catalog.resources = (0..10000)
        .map(|i| Resource {
            kind: ResourceKind::Assistant,
            id: format!("id-{i}"),
            name: "unrelated".into(),
            capabilities: vec!["chat".into()],
        })
        .collect();
    let selection = Intent {
        profile: intent().profile,
        selections: vec![Selection::Reuse {
            kind: ResourceKind::Assistant,
            id: "id-9999".into(),
            required_capabilities: vec!["video".into()],
        }],
    };
    let result = preview(&catalog, &selection).unwrap();
    assert_eq!(result.gaps.len(), 1);
    assert!(!result.purchases);
    catalog.resources.push(catalog.resources[0].clone());
    assert!(preview(&catalog, &selection).is_err());
    let purchase = Intent {
        profile: intent().profile,
        selections: vec![Selection::Create {
            kind: ResourceKind::Number,
            name: "number".into(),
        }],
    };
    assert!(preview(
        &Catalog {
            authenticated: true,
            complete: true,
            resources: vec![]
        },
        &purchase
    )
    .is_err());
}
#[test]
fn callback_success_does_not_verify_sip_media_sms_or_delivery() {
    let observations = vec![
        Observation {
            prerequisite: Prerequisite::Callback,
            direction: Direction::Inbound,
            state: ObservationState::Ready,
            observed_at: 100,
            valid_until: 120,
        },
        Observation {
            prerequisite: Prerequisite::SipRegistration,
            direction: Direction::Outbound,
            state: ObservationState::Ready,
            observed_at: 90,
            valid_until: 100,
        },
    ];
    let report = readiness(&observations, 110).unwrap();
    assert_eq!(report.len(), 14);
    assert!(report
        .iter()
        .any(|r| r.prerequisite == Prerequisite::Callback
            && r.direction == Direction::Inbound
            && r.state == ObservationState::Ready));
    assert!(report
        .iter()
        .any(|r| r.prerequisite == Prerequisite::SipRegistration
            && r.state == ObservationState::Stale));
    assert!(report
        .iter()
        .filter(|r| matches!(
            r.prerequisite,
            Prerequisite::Rtp | Prerequisite::Media | Prerequisite::SmsEligibility
        ))
        .all(|r| r.state == ObservationState::Unknown));
}

#[tokio::test]
async fn discovery_uses_read_only_pages_and_discards_provider_secret_fields() {
    use wiremock::{
        matchers::{method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/assistant")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id":"assistant-1","name":"existing","model":{},"voice":{},"private_key":"must-not-export"}]))).expect(1).mount(&server).await;
    for endpoint in [
        "/phone_numbers",
        "/credential_connections",
        "/messaging_profiles",
    ] {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .and(query_param("page[number]", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":[],"meta":{"total_pages":1}})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let vapi = connect_connectors::Vapi::new("fixture-only", &server.uri()).unwrap();
    let telnyx = connect_connectors::Telnyx::new("fixture-only", &server.uri()).unwrap();
    let catalog = discover_with_clients(&vapi, &telnyx).await.unwrap();
    assert!(catalog.complete);
    assert_eq!(catalog.resources.len(), 1);
    assert!(!serde_json::to_string(&catalog)
        .unwrap()
        .contains("must-not-export"));
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method == "GET"));
    let rejected = MockServer::start().await;
    Mock::given(path("/assistant"))
        .respond_with(ResponseTemplate::new(401).set_body_string("provider-secret-error-body"))
        .mount(&rejected)
        .await;
    let vapi = connect_connectors::Vapi::new("fixture-only", &rejected.uri()).unwrap();
    let error = discover_with_clients(&vapi, &telnyx).await.unwrap_err();
    assert!(!error.to_string().contains("provider-secret-error-body"));
}

#[tokio::test]
async fn interruption_before_request_retries_only_after_authoritative_absence_with_same_id() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct BeforeSend {
        interrupted: AtomicBool,
        calls: Mutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl Provisioner for BeforeSend {
        async fn attempt(&self, id: &str, _: &Selection) -> anyhow::Result<Attempt> {
            if self.interrupted.swap(false, Ordering::SeqCst) {
                anyhow::bail!("fixture interruption before sending");
            }
            self.calls.lock().unwrap().push(id.into());
            Ok(Attempt::Accepted(Receipt {
                resource_id: "one-owned-resource".into(),
                created: true,
            }))
        }
        async fn reconcile(&self, _: &str, _: &Selection) -> anyhow::Result<Reconciliation> {
            Ok(Reconciliation::NotAccepted)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("checkpoint.json");
    let adapter = BeforeSend {
        interrupted: AtomicBool::new(true),
        calls: Mutex::new(vec![]),
    };
    assert!(execute(&path, &intent(), &adapter).await.is_err());
    let before = inspect_execution(&path).unwrap();
    assert_eq!(before.steps[0].state, StepState::Running);
    let after = execute(&path, &intent(), &adapter).await.unwrap();
    assert_eq!(before.operation_id, after.operation_id);
    assert_eq!(
        adapter.calls.lock().unwrap().as_slice(),
        [before.steps[0].mutation_id.clone()]
    );
    assert!(matches!(after.steps[0].state, StepState::Accepted(_)));
}
