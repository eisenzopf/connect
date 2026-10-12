use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rvoip_webrtc::{Answer, IceCandidate, Offer, Signaler};
use std::time::Duration;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
    MaybeTlsStream, WebSocketStream,
};

type SignalingSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) struct AuthenticatedSignaler {
    url: String,
    token: String,
    socket: tokio::sync::Mutex<Option<SignalingSocket>>,
}

impl AuthenticatedSignaler {
    pub(crate) fn new(url: String, token: String) -> Self {
        Self {
            url,
            token,
            socket: tokio::sync::Mutex::new(None),
        }
    }

    pub(crate) async fn wait_ready(&self) {
        let mut socket = self
            .socket
            .lock()
            .await
            .take()
            .expect("signaling lease must remain open");
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if !message.is_text() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(message.to_text().unwrap()).unwrap();
            match value["type"].as_str() {
                Some("ready") => break,
                Some("rejected") => panic!("Connect rejected authenticated WebRTC route"),
                _ => {}
            }
        }
        *self.socket.lock().await = Some(socket);
    }

    pub(crate) async fn end_route(&self, connection_id: &str) {
        let mut socket = self
            .socket
            .lock()
            .await
            .take()
            .expect("signaling lease must remain open");
        socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "bye",
                    "connection_id": connection_id,
                    "request_id": uuid::Uuid::new_v4().to_string(),
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(Ok(message)) = socket.next().await {
                if message.is_close() {
                    break;
                }
                if message.is_text()
                    && serde_json::from_str::<serde_json::Value>(message.to_text().unwrap())
                        .is_ok_and(|value| value["type"] == "bye")
                {
                    break;
                }
            }
        })
        .await;
        let _ = socket.close(None).await;
    }

    pub(crate) async fn wait_remote_end(&self) {
        let mut socket = self
            .socket
            .lock()
            .await
            .take()
            .expect("signaling lease must remain open");
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(message) = socket.next().await {
                match message.unwrap() {
                    Message::Close(_) => break,
                    Message::Text(body)
                        if serde_json::from_str::<serde_json::Value>(&body)
                            .is_ok_and(|value| value["type"] == "bye") =>
                    {
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("server should end the paired WebRTC signaling lease");
        let _ = socket.close(None).await;
    }
}

#[async_trait]
impl Signaler for AuthenticatedSignaler {
    async fn send_offer(&self, offer: &Offer) -> rvoip_webrtc::Result<Answer> {
        let mut request = self.url.clone().into_client_request().unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_str(&format!("rvoip.webrtc.v1, token.{}", self.token)).unwrap(),
        );
        let (mut socket, _) = connect_async(request).await.unwrap();
        let request_id = uuid::Uuid::new_v4().to_string();
        socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "offer-ready",
                    "sdp": offer.0,
                    "request_id": request_id,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if !message.is_text() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(message.to_text().unwrap()).unwrap();
            match value["type"].as_str() {
                Some("answer") => {
                    let answer = Answer {
                        sdp: value["sdp"].as_str().unwrap().to_owned(),
                        connection_id: value["connection_id"].as_str().map(str::to_owned),
                    };
                    *self.socket.lock().await = Some(socket);
                    return Ok(answer);
                }
                Some("rejected") => panic!("Connect rejected authenticated WebRTC route"),
                _ => {}
            }
        }
    }

    async fn send_answer(&self, _answer: &Answer) -> rvoip_webrtc::Result<()> {
        Ok(())
    }

    async fn send_ice(&self, _candidate: &IceCandidate) -> rvoip_webrtc::Result<()> {
        Ok(())
    }
}
