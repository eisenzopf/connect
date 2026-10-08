//! Minimal adapter with no server or database dependency.
use async_trait::async_trait;
use connect_connector_api::{Completion, Connector, DeliveryRequest, Failure};
struct Echo;
#[async_trait]
impl Connector for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }
    async fn deliver(&self, request: &DeliveryRequest) -> Result<Completion, Failure> {
        Ok(Completion {
            provider_id: "local-demo".into(),
            reply: Some(request.body.clone()),
        })
    }
}
#[tokio::main]
async fn main() {
    let request = DeliveryRequest {
        target: "demo".into(),
        source: None,
        body: "Hello, connector".into(),
        previous_chat_id: None,
    };
    match Echo.deliver(&request).await {
        Ok(result) => println!("{}: {}", Echo.name(), result.reply.unwrap_or_default()),
        Err(failure) => eprintln!("Delivery {}", failure.state()),
    }
}
