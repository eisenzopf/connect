use async_trait::async_trait;
use connect_connector_api::{
    lifecycle::*, manifest::Manifest, Descriptor, Failure, InboundCall, RemoteCall, VoiceConnector,
    VoiceQuery, VoiceStart,
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Driver {
    calls: AtomicUsize,
    bad: bool,
}
#[async_trait]
impl VoiceConnector for Driver {
    fn voice_manifest(&self) -> Manifest {
        Manifest::messaging(Descriptor::one_way("receipt-fixture"))
    }
    fn normalize(&self, id: &str, call: &RemoteCall) -> Result<Observation, Failure> {
        if self.bad {
            return Err(Failure::Rejected);
        }
        Ok(Observation {
            mutation_id: id.into(),
            external_id: Some(call.id.clone()),
            event_id: "provider-event".into(),
            state: State::Voice(VoiceState::Active),
            verification: Verification::AuthenticatedResponse,
        })
    }
    async fn inbound(&self, _: &VoiceQuery) -> Result<Vec<InboundCall>, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![InboundCall {
            provider_id: "provider-call".into(),
            assistant_id: "assistant".into(),
            phone_number_id: "phone".into(),
            caller: "+15555550100".into(),
            created_at: 1,
            state: "in-progress".into(),
            control_url: None,
            ended_reason: None,
        }])
    }
    async fn start(&self, _: &VoiceStart) -> Result<RemoteCall, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(remote())
    }
    async fn status(&self, _: &str) -> Result<RemoteCall, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(remote())
    }
    async fn end(&self, _: &str, _: Option<&str>) -> Result<RemoteCall, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(remote())
    }
}
#[tokio::test]
async fn discovery_returns_attributed_unbound_receipts_and_preserves_decoder_failure() {
    let query = VoiceQuery {
        assistant_id: "assistant".into(),
        phone_number_id: "phone".into(),
        since: 0,
        scanned_until: 0,
        scan_until: 1,
    };
    for bad in [false, true] {
        let d = Driver {
            calls: AtomicUsize::new(0),
            bad,
        };
        match d.inbound_receipts(&query).await {
            Ok(receipts) => {
                assert!(!bad);
                assert_eq!(receipts.len(), 1);
                let r = &receipts[0];
                assert_eq!(r.call.provider_id, "provider-call");
                assert_eq!(r.state, VoiceState::Active);
                assert_eq!(r.event_id, "discover:provider-event");
                assert_eq!(r.verification, Verification::Reconciliation);
            }
            Err(Failure::Uncertain) => assert!(bad),
            _ => panic!("unexpected discovery result"),
        }
        assert_eq!(d.calls.load(Ordering::SeqCst), 1);
    }
}
fn remote() -> RemoteCall {
    RemoteCall {
        id: "provider-call".into(),
        state: "in-progress".into(),
        control_url: None,
        ended_reason: None,
    }
}
fn request() -> VoiceStart {
    VoiceStart {
        assistant_id: "assistant".into(),
        phone_number_id: "phone".into(),
        destination: "+15555550100".into(),
    }
}
#[tokio::test]
async fn invalid_mutation_never_reaches_transport() {
    let d = Driver {
        calls: AtomicUsize::new(0),
        bad: false,
    };
    for id in [String::new(), "bad\nreference".into(), "x".repeat(257)] {
        assert!(matches!(
            d.start_receipt(&id, &request()).await,
            Err(Failure::Rejected)
        ));
        assert!(matches!(
            d.status_receipt(&id, "provider").await,
            Err(Failure::Rejected)
        ));
        assert!(matches!(
            d.end_receipt(&id, "provider", None).await,
            Err(Failure::Rejected)
        ));
    }
    assert_eq!(d.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn normalization_failure_after_possible_io_remains_uncertain() {
    let d = Driver {
        calls: AtomicUsize::new(0),
        bad: true,
    };
    assert!(matches!(
        d.start_receipt("logical-start", &request()).await,
        Err(Failure::Uncertain)
    ));
    assert!(matches!(
        d.status_receipt("logical-status", "provider").await,
        Err(Failure::Uncertain)
    ));
    assert!(matches!(
        d.end_receipt("logical-end", "provider", None).await,
        Err(Failure::Uncertain)
    ));
    assert_eq!(d.calls.load(Ordering::SeqCst), 3);
}
#[tokio::test]
async fn successful_receipts_keep_logical_binding_and_status_is_reconciliation() {
    let d = Driver {
        calls: AtomicUsize::new(0),
        bad: false,
    };
    let start = d.start_receipt("logical-start", &request()).await.unwrap();
    assert_eq!(start.observation.mutation_id, "logical-start");
    assert_eq!(
        start.observation.external_id.as_deref(),
        Some(start.call.id.as_str())
    );
    let status = d
        .status_receipt("logical-start", &start.call.id)
        .await
        .unwrap();
    assert_eq!(
        status.observation.verification,
        Verification::Reconciliation
    );
    assert_eq!(status.observation.event_id, "reconcile:provider-event");
    assert_eq!(status.observation.mutation_id, "logical-start");
    assert_eq!(d.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn undeclared_correlation_default_never_submits_a_call_or_asserts_absence() {
    let d = Driver {
        calls: AtomicUsize::new(0),
        bad: false,
    };
    let q = connect_connector_api::VoiceReconcileQuery {
        request: request(),
        since: 1,
        until: 100,
    };
    for mutation in ["original", "", "bad\n"] {
        assert!(matches!(
            d.reconcile_receipt(mutation, &q).await,
            Err(Failure::Rejected)
        ));
    }
    assert_eq!(d.calls.load(Ordering::SeqCst), 0);
}
