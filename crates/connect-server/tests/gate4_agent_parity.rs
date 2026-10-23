//! Ordinary MCP reuses native operation receipts, authorization and discovery.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
#[path = "support/mcp_operations.rs"]
mod operations;
use connect_client::ClientError;
use connect_protocol::{AvailabilityState, Operation, Outcome, PROFILE};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_communication_tools_match_native_normal_credential_outcomes() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut native = f.agent_client().await;
    let cid = native
        .create("ordinary-parity-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let cases = operations::cases(&cid, &f.peer.identity_id);
    assert_eq!(cases.len(), 29);
    let mut passed = 0;
    let mut denied = 0;
    for operation in cases {
        let name = operation.name();
        let response = mcp::operation(&client, &operation).await;
        match native.request(operation.clone()).await {
            Ok(expected) => {
                assert_eq!(mcp::outcome(response), expected, "{name}");
                passed += 1;
            }
            Err(ClientError::Application(expected)) => {
                assert_eq!(mcp::error(response), expected, "{name}");
                denied += 1;
            }
            Err(error) => panic!("{name} failed outside the application contract: {error}"),
        }
    }
    assert_eq!((passed, denied), (7, 22));
    assert_eq!(
        native
            .history(&cid, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    native.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_discovery_availability_delivery_and_close_do_not_expose_private_configuration() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut native = f.agent_client().await;
    let cid = native
        .create("ordinary-extra-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    for operation in [
        Operation::AvailabilitySet {
            request_id: "ordinary-available".into(),
            state: AvailabilityState::Available,
            preferred_route: None,
            expires_at: chrono::Utc::now().timestamp() + 300,
        },
        Operation::AvailabilityGet {
            identity_id: f.agent.identity_id.clone(),
        },
        Operation::ProviderStatus {
            conversation_id: cid.clone(),
        },
    ] {
        let actual = mcp::outcome(mcp::operation(&client, &operation).await);
        assert_eq!(native.request(operation).await.unwrap(), actual);
    }
    let health = mcp::outcome(mcp::operation(&client, &Operation::Health {}).await);
    let Outcome::Health {
        installation_id,
        profile,
        ..
    } = health
    else {
        panic!("health result")
    };
    assert_eq!(profile, PROFILE);
    assert_eq!(
        installation_id,
        native.health().await.unwrap().installation_id
    );
    let binding = mcp::operation(
        &client,
        &Operation::BrowserBinding {
            conversation_id: None,
            session_id: None,
        },
    )
    .await;
    match native
        .request(Operation::BrowserBinding {
            conversation_id: None,
            session_id: None,
        })
        .await
    {
        Ok(expected) => assert_eq!(mcp::outcome(binding), expected),
        Err(ClientError::Application(error)) => assert_eq!(mcp::error(binding), error),
        Err(error) => panic!("browser discovery failed outside application contract: {error}"),
    }
    let close = Operation::Close {
        request_id: "ordinary-close".into(),
        conversation_id: cid.clone(),
        options: Default::default(),
    };
    let closed = mcp::outcome(mcp::operation(&client, &close).await);
    assert!(matches!(&closed, Outcome::Conversation(value) if value.closed));
    assert_eq!(native.request(close).await.unwrap(), closed);
    assert!(native
        .send("closed-send", &cid, "Do not reopen a closed conversation.")
        .await
        .is_err());
    native.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}
