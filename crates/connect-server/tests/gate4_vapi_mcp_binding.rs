//! Real MCP admission with trusted fixture bindings, never live provider proof.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/vapi_bindings.rs"]
mod provider;
use connect_client::ConnectClient;
use connect_mcp::{RunningMcp, VapiBindingAuth, VapiOrigin};
use connect_protocol::tasks::TaskPermission;
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::json;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_chat_and_session_bearers_cannot_select_another_task_or_origin() {
    for origin in [VapiOrigin::Call, VapiOrigin::Chat, VapiOrigin::ChatSession] {
        let f = fixture::Fixture::start_permitted(
            false,
            vec![TaskPermission::History, TaskPermission::Send],
        )
        .await;
        let second = provider::second_task(&f).await;
        let expiry = chrono::Utc::now().timestamp() + 180;
        let (binding, grant) = provider::grant(&f, &f.task, origin, "origin-one", expiry).await;
        let (_, other) = provider::grant(&f, &second, origin, "origin-two", expiry).await;
        for kind in [
            connect_protocol::tasks::InvocationKind::Native,
            connect_protocol::tasks::InvocationKind::HostSession,
        ] {
            let mut local = binding.clone();
            local.kind = kind;
            assert!(VapiBindingAuth::new(f.server.service.clone(), local, origin).is_err());
        }
        let mut wrong_kind = binding.clone();
        wrong_kind.kind = if origin == VapiOrigin::Call {
            connect_protocol::tasks::InvocationKind::ProviderChat
        } else {
            connect_protocol::tasks::InvocationKind::ProviderCall
        };
        assert!(VapiBindingAuth::new(f.server.service.clone(), wrong_kind, origin).is_err());
        let mut malformed = binding.clone();
        malformed.external_id = "unverified space".into();
        assert!(VapiBindingAuth::new(f.server.service.clone(), malformed, origin).is_err());
        let auth = VapiBindingAuth::new(f.server.service.clone(), binding, origin).unwrap();
        let listener = RunningMcp::start_local(
            f.server.service.clone(),
            Arc::new(auth),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        let url = format!("http://{}/mcp", listener.address);
        let valid_headers = provider::headers(origin, "origin-one");
        let http = provider::http(valid_headers.clone()).await;
        let client = rmcp::model::ClientConfig::default()
            .serve(StreamableHttpClientTransport::with_client(
                http.clone(),
                StreamableHttpClientTransportConfig::with_uri(url.clone())
                    .auth_header(grant.expose_secret()),
            ))
            .await
            .unwrap();
        let status = client
            .call_tool(CallToolRequestParams::new("connect_task_status"))
            .await
            .unwrap();
        assert_eq!(status.structured_content.unwrap()["data"]["id"], f.task.id);
        let arguments =
            json!({"request_id":"draft-once","step_id":"draft","body":"Provider-scoped draft"});
        let mut receipt = None;
        for _ in 0..2 {
            let result = client
                .call_tool(
                    CallToolRequestParams::new("connect_send")
                        .with_arguments(arguments.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            assert_ne!(result.is_error, Some(true));
            let result = result.structured_content.unwrap();
            assert_eq!(result["data"]["task_id"], f.task.id);
            if let Some(first) = &receipt {
                assert_eq!(&result, first);
            } else {
                receipt = Some(result);
            }
        }
        client.cancel().await.unwrap();
        let mut cases = vec![
            (valid_headers.clone(), "".to_owned()),
            (valid_headers.clone(), f.owner.credential.clone()),
            (valid_headers.clone(), f.issued.expose_secret().to_owned()),
            (valid_headers.clone(), other.expose_secret().to_owned()),
            (
                provider::headers(origin, "origin-two"),
                grant.expose_secret().to_owned(),
            ),
            (
                reqwest_mcp::header::HeaderMap::new(),
                grant.expose_secret().to_owned(),
            ),
        ];
        let primary = match origin {
            VapiOrigin::Call => "x-call-id",
            VapiOrigin::Chat => "x-chat-id",
            VapiOrigin::ChatSession => "x-session-id",
        };
        for value in ["", "origin-one,origin-two", "forged id"] {
            let mut headers = valid_headers.clone();
            headers.insert(primary, value.parse().unwrap());
            cases.push((headers, grant.expose_secret().to_owned()));
        }
        let mut oversized = valid_headers.clone();
        oversized.insert(primary, "x".repeat(129).parse().unwrap());
        cases.push((oversized, grant.expose_secret().to_owned()));
        if origin == VapiOrigin::ChatSession {
            let mut missing_chat = valid_headers.clone();
            missing_chat.remove("x-chat-id");
            cases.push((missing_chat, grant.expose_secret().to_owned()));
            for chat in [String::new(), "chat,other".to_owned(), "x".repeat(129)] {
                let mut malformed_chat = valid_headers.clone();
                malformed_chat.insert("x-chat-id", chat.parse().unwrap());
                cases.push((malformed_chat, grant.expose_secret().to_owned()));
            }
            let mut duplicate_chat = valid_headers.clone();
            duplicate_chat.append("x-chat-id", "other-chat".parse().unwrap());
            cases.push((duplicate_chat, grant.expose_secret().to_owned()));
        }
        let mut duplicate = valid_headers.clone();
        duplicate.append(primary, "origin-one".parse().unwrap());
        cases.push((duplicate, grant.expose_secret().to_owned()));
        let mut conflicting = valid_headers.clone();
        conflicting.insert(
            if origin == VapiOrigin::Call {
                "x-chat-id"
            } else {
                "x-call-id"
            },
            "conflicting".parse().unwrap(),
        );
        cases.push((conflicting, grant.expose_secret().to_owned()));
        let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
        for (index, (headers, token)) in cases.into_iter().enumerate() {
            let request = provider::http(reqwest_mcp::header::HeaderMap::new()).await;
            let response = request
                .post(&url)
                .headers(headers)
                .header("accept", "application/json, text/event-stream")
                .bearer_auth(token)
                .json(&provider::mutation(&format!("denied-{index}")))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 401, "{origin:?}: case {index}");
            let body = response.text().await.unwrap();
            assert!(!body.contains(grant.expose_secret()));
            assert!(!body.contains(&f.owner.credential));
        }
        assert_eq!(
            connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before,
            "denied provider requests mutated durable state"
        );
        f.server
            .service
            .revoke_task_grant(f.owner.credential_id.clone(), grant.grant.id.clone())
            .await
            .unwrap();
        assert_eq!(
            http.post(&url)
                .header("accept", "application/json, text/event-stream")
                .bearer_auth(grant.expose_secret())
                .json(&provider::mutation("draft-once"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
        assert_eq!(
            peer.history(&f.task.conversation_id, None, 100)
                .await
                .unwrap()
                .messages
                .len(),
            1
        );
        peer.close().await;
        listener.shutdown().await.unwrap();
        f.close().await;
    }
}
