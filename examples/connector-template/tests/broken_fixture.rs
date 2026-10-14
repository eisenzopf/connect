//! The common harness must reject an adapter that trusts unauthenticated callbacks.
use async_trait::async_trait;
use connect_connector_api::{
    conformance::{self, Fixture, Injection},
    lifecycle::*,
    manifest::*,
    Failure,
};
use connect_connector_template::LocalProvider;
struct Broken(LocalProvider);
#[async_trait]
impl MessageExtension for Broken {
    async fn message_status(&self, id: &str) -> Result<MessageReceipt, Failure> {
        self.0.message_status(id).await
    }
    async fn send(&self, r: &MessageRequest) -> Result<MessageReceipt, Failure> {
        self.0.send(r).await
    }
}
#[async_trait]
impl MediaExtension for Broken {
    async fn open(&self, r: &VoiceRequest) -> Result<Observation, Failure> {
        self.0.open(r).await
    }
    async fn reconcile(&self, id: &str) -> Result<Observation, Failure> {
        self.0.reconcile(id).await
    }
    async fn close(&self, id: &str) -> Result<Observation, Failure> {
        self.0.close(id).await
    }
    async fn exchange(&self, id: &str, pcm: &[i16]) -> Result<Vec<i16>, Failure> {
        self.0.exchange(id, pcm).await
    }
}
#[async_trait]
impl Fixture for Broken {
    fn manifest(&self) -> Manifest {
        Fixture::manifest(&self.0)
    }
    fn configuration(&self) -> std::collections::BTreeMap<String, String> {
        Fixture::configuration(&self.0)
    }
    fn readiness(&self) -> Readiness {
        Fixture::readiness(&self.0)
    }
    fn inject(&self, mode: Injection) {
        self.0.inject(mode);
    }
    fn restart(&mut self) -> Result<(), Failure> {
        self.0.restart()
    }
    fn effects(&self, id: &str) -> usize {
        self.0.effects(id)
    }
    fn independent_message(&self, id: &str, body: &str) -> Result<IncomingMessage, Failure> {
        self.0.independent_message(id, body)
    }
    fn authenticate_callback(&self, body: &[u8], _auth: &[u8]) -> Result<Observation, Failure> {
        serde_json::from_slice(body).map_err(|_| Failure::Rejected)
    }
    fn signed_callback(&self, event: &Observation) -> (Vec<u8>, Vec<u8>) {
        self.0.signed_callback(event)
    }
}
#[tokio::test]
async fn unauthenticated_adapter_fails_expected_mandatory_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let mut broken = Broken(LocalProvider::open_fixture(tmp.path().join("provider.json")).unwrap());
    let report = conformance::run(&mut broken).await;
    assert!(!report.passed());
    let failed: Vec<_> = report
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(failed, ["invalid-callback-auth", "tampered-callback"]);
}
