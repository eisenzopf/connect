use connect_connector_api::DispatchPermit;
use connect_media_conformance::{Fixture, Metrics, Observation, Status};
use connect_media_connector_template::{Adapter, Admission, Remote};
use rvoip_core::{
    adapter::{ConnectionAdapter, OriginateRequest},
    error::Result,
    ids::*,
    Direction, Transport,
};
use std::sync::Arc;

pub struct Template;
#[async_trait::async_trait]
impl Fixture for Template {
    type Adapter = Adapter;
    type Backend = Remote;
    const QUEUE_CAPACITY: usize = connect_media_connector_template::QUEUE_CAPACITY;
    async fn open(backend: Arc<Remote>) -> Result<Arc<Adapter>> {
        Adapter::open(backend).await
    }
    fn manifest() -> connect_connector_api::manifest::Manifest {
        connect_media_connector_template::manifest()
    }
    fn request(adapter: &Adapter, operation: &str, permit: DispatchPermit) -> OriginateRequest {
        OriginateRequest::new(
            SessionId::from_string("fixture-session"),
            ParticipantId::from_string("fixture-participant"),
            "fixture:duplex",
            Direction::Outbound,
            adapter.capabilities(),
        )
        .with_transport(Transport::InProcessAi)
        .with_context(Admission {
            operation_id: operation.into(),
            permit,
        })
    }
    fn observe(adapter: &Adapter, operation: &str, permit: &DispatchPermit) -> Result<Observation> {
        let o = adapter.observe(operation, permit)?;
        Ok(Observation {
            connection_id: o.connection_id,
            effect_attempts: o.effect_attempts,
            state: match o.state {
                connect_media_connector_template::Status::Prepared => Status::Prepared,
                connect_media_connector_template::Status::Active => Status::Active,
                connect_media_connector_template::Status::Stopped => Status::Stopped,
            },
        })
    }
    fn metrics(adapter: &Adapter, connection: &ConnectionId) -> Result<Metrics> {
        let m = adapter.metrics(connection)?;
        Ok(Metrics {
            generated: m.generated,
            decoded: m.decoded,
            last_sample: m.last_sample,
        })
    }
}
/// Actual deliberately broken admission: discards current host custody. This
/// drives the real adapter; it cannot set or replace harness pass/fail fields.
pub struct BrokenCustody;
struct AlwaysCurrent(std::time::Instant);
impl connect_connector_api::dispatch::Authority for AlwaysCurrent {
    fn current(&self) -> bool {
        true
    }
    fn deadline(&self) -> std::time::Instant {
        self.0
    }
}
#[async_trait::async_trait]
impl Fixture for BrokenCustody {
    type Adapter = Adapter;
    type Backend = Remote;
    const QUEUE_CAPACITY: usize = Template::QUEUE_CAPACITY;
    async fn open(backend: Arc<Remote>) -> Result<Arc<Adapter>> {
        Template::open(backend).await
    }
    fn manifest() -> connect_connector_api::manifest::Manifest {
        Template::manifest()
    }
    fn request(adapter: &Adapter, operation: &str, _: DispatchPermit) -> OriginateRequest {
        Template::request(
            adapter,
            operation,
            DispatchPermit::from_host(Arc::new(AlwaysCurrent(
                std::time::Instant::now() + std::time::Duration::from_secs(5),
            ))),
        )
    }
    fn observe(adapter: &Adapter, operation: &str, permit: &DispatchPermit) -> Result<Observation> {
        Template::observe(adapter, operation, permit)
    }
    fn metrics(adapter: &Adapter, connection: &ConnectionId) -> Result<Metrics> {
        Template::metrics(adapter, connection)
    }
}
