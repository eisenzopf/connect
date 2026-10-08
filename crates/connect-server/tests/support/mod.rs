#![allow(dead_code)]
//! Raw conformance peers, deliberately separate from the native public SDK.
use connect_client::ClientConfig;
use connect_protocol::{Operation, Outcome, Request, Response, PROFILE, REQUEST_TYPE, RESULT_TYPE};
use futures_util::{SinkExt, StreamExt};
use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

pub enum Peer {
    Quic {
        endpoint: quinn::Endpoint,
        client: Arc<rvoip_quic::UctpQuicClient>,
        incoming: tokio::sync::mpsc::Receiver<UctpEnvelope>,
    },
    WebSocket(Box<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>),
}
impl Peer {
    pub fn incoming_len(&self) -> usize {
        match self {
            Self::Quic { incoming, .. } => incoming.len(),
            Self::WebSocket(_) => 0,
        }
    }
    pub async fn quic(config: &ClientConfig) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rustls::pki_types::CertificateDer::from(
            std::fs::read(&config.trust_certificate).unwrap(),
        );
        let tls = Arc::new(rvoip_uctp::substrate::dev_client_config_trusting(&cert).unwrap());
        let endpoint = rvoip_uctp::substrate::make_client_endpoint(
            "127.0.0.1:0".parse().unwrap(),
            tls.clone(),
        )
        .unwrap();
        let client = rvoip_quic::UctpQuicClient::connect(
            &endpoint,
            config.endpoint,
            &config.server_name,
            tls,
        )
        .await
        .unwrap();
        let incoming = client.take_inbound().unwrap();
        Self::Quic {
            endpoint,
            client,
            incoming,
        }
    }
    pub async fn websocket(address: std::net::SocketAddr) -> Self {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        Self::WebSocket(Box::new(ws))
    }
    pub async fn send(&mut self, envelope: UctpEnvelope) {
        match self {
            Self::Quic { client, .. } => client.send(envelope).await.unwrap(),
            Self::WebSocket(ws) => ws
                .send(Message::Text(
                    serde_json::to_string(&envelope).unwrap().into(),
                ))
                .await
                .unwrap(),
        }
    }
    pub async fn receive(&mut self) -> UctpEnvelope {
        tokio::time::timeout(Duration::from_secs(5), async {
            match self {
                Self::Quic { incoming, .. } => incoming.recv().await.unwrap(),
                Self::WebSocket(ws) => loop {
                    match ws.next().await.unwrap().unwrap() {
                        Message::Text(text) => break serde_json::from_str(&text).unwrap(),
                        Message::Ping(_) | Message::Pong(_) => {}
                        other => panic!("unexpected frame: {other:?}"),
                    }
                },
            }
        })
        .await
        .unwrap()
    }
    pub async fn exchange(&mut self, envelope: UctpEnvelope) -> UctpEnvelope {
        let id = envelope.id.clone();
        self.send(envelope).await;
        loop {
            let response = self.receive().await;
            if response.in_reply_to.as_ref() == Some(&id) {
                return response;
            }
        }
    }
    pub async fn authenticate(&mut self, token: &str) -> bool {
        let challenge=self.exchange(UctpEnvelope::new(MessageType::AuthHello,serde_json::json!({"device":{"id":"conformance-fixture","kind":"desktop","platform":"test","sdk_version":"fixture"},"auth_methods":["bearer"],"capabilities":{}}))).await;
        assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
        assert!(
            challenge.payload["server_capabilities"]["application_profiles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == PROFILE)
        );
        let reply = self
            .exchange(UctpEnvelope::new(
                MessageType::AuthResponse,
                serde_json::json!({"method":"bearer","credential":token}),
            ))
            .await;
        reply.msg_type == MessageType::AuthSession
    }
    pub async fn request(&mut self, operation: Operation) -> connect_protocol::Result<Outcome> {
        let envelope = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(Request::new(operation)).unwrap(),
        );
        let reply = self.exchange(envelope).await;
        assert_eq!(reply.msg_type, MessageType::Unknown(RESULT_TYPE.into()));
        let response: Response = reply.decode_payload().unwrap();
        assert_eq!(response.profile, PROFILE);
        response.result
    }
    pub async fn close(&mut self) {
        match self {
            Self::Quic {
                endpoint, client, ..
            } => {
                client.connection.close(0u32.into(), b"fixture closed");
                endpoint.close(0u32.into(), b"fixture closed");
                let _ = tokio::time::timeout(Duration::from_secs(2), endpoint.wait_idle()).await;
            }
            Self::WebSocket(ws) => {
                let _ = ws.close().await;
            }
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Self::Quic {
            endpoint, client, ..
        } = self
        {
            client.connection.close(0u32.into(), b"fixture dropped");
            endpoint.close(0u32.into(), b"fixture dropped");
        }
    }
}
