//! A real authenticated QUIC peer for deliberate malformed/reordered RTP.
//! Application state is still created through the public Connect SDK.

use connect_client::ClientConfig;
use rvoip_quic::UctpQuicClient;
use rvoip_uctp::{
    envelope::UctpEnvelope,
    substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload},
    types::MessageType,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

pub(crate) struct WireAudioPeer {
    pub(crate) peer: Arc<UctpQuicClient>,
    _endpoint: quinn::Endpoint,
    pub(crate) local_id: u16,
}

impl WireAudioPeer {
    pub(crate) async fn open(
        config: &ClientConfig,
        conversation: &str,
        session: &str,
        remote_identity: &str,
        publisher: &str,
    ) -> Self {
        let cert = rustls::pki_types::CertificateDer::from(
            std::fs::read(&config.trust_certificate).unwrap(),
        );
        let tls = Arc::new(rvoip_uctp::substrate::dev_client_config_trusting(&cert).unwrap());
        let endpoint = rvoip_uctp::substrate::make_client_endpoint(
            "127.0.0.1:0".parse().unwrap(),
            tls.clone(),
        )
        .unwrap();
        let peer = UctpQuicClient::connect(&endpoint, config.endpoint, &config.server_name, tls)
            .await
            .unwrap();
        let mut inbox = peer.take_inbound().unwrap();
        peer.send(UctpEnvelope::new(
            MessageType::AuthHello,
            json!({"device":{"id":"wire-fault-fixture","kind":"desktop","platform":"test","sdk_version":"test"},"auth_methods":["bearer"],"capabilities":{}}),
        ))
        .await
        .unwrap();
        next(&mut inbox, MessageType::AuthChallenge).await;
        peer.send(UctpEnvelope::new(
            MessageType::AuthResponse,
            json!({"method":"bearer","credential":config.credential}),
        ))
        .await
        .unwrap();
        let authenticated = next(&mut inbox, MessageType::AuthSession).await;
        let participant = authenticated.payload["participant_id"].as_str().unwrap();
        let connection = uuid::Uuid::new_v4().to_string();
        peer.send(
            UctpEnvelope::new(
                MessageType::SessionInvite,
                json!({"from":participant,"to":[remote_identity],"medium":"voice","intent":"connect-session","capabilities_offer":{"audio":["g.711-mu"]}}),
            )
            .with_cid(conversation)
            .with_sid(session),
        )
        .await
        .unwrap();
        peer.send(
            UctpEnvelope::new(
                MessageType::ConnectionOffer,
                json!({"by_participant":participant,"substrate":"quic","capabilities":{},"streams_offered":[{"id":publisher,"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"substrate_setup":null}),
            )
            .with_sid(session)
            .with_connid(&connection),
        )
        .await
        .unwrap();
        peer.send(
            UctpEnvelope::new(MessageType::ConnectionReady, json!({}))
                .with_sid(session)
                .with_connid(&connection),
        )
        .await
        .unwrap();
        let opened = next(&mut inbox, MessageType::StreamOpened).await;
        assert_eq!(opened.payload["stream"]["strm_id"], publisher);
        let local_id = opened.payload["stream"]["stream_local_id"]
            .as_u64()
            .unwrap() as u16;
        // Drain reliable notifications independently of datagrams throughout
        // the rehearsal. The fixture's endpoint owns the physical connection.
        tokio::spawn(async move { while inbox.recv().await.is_some() {} });
        Self {
            peer,
            _endpoint: endpoint,
            local_id,
        }
    }

    pub(crate) fn send(&self, sequence: u32, local_id: u16, value: u8) {
        let datagram = RtpDatagram {
            flags: 0,
            stream_local_id: local_id,
            seq: sequence,
            rtp: RtpMediaPayload {
                payload: vec![value; 160].into(),
                payload_type: 0,
                sequence_number: sequence as u16,
                timestamp: sequence.wrapping_mul(160),
                ssrc: 0x434f_4e33,
            },
        };
        self.peer
            .connection
            .send_datagram(pack_rtp_datagram(&datagram).unwrap())
            .unwrap();
    }
}

impl Drop for WireAudioPeer {
    fn drop(&mut self) {
        self.peer.connection.close(0u32.into(), b"fixture-complete");
        self._endpoint.close(0u32.into(), b"fixture-complete");
    }
}

/// Inspect the durable binding, rather than inferring it from a Session whose
/// other peer can legitimately remain ready after this connection disappears.
pub(crate) async fn wait_binding_ended(
    database: &std::path::Path,
    session: &str,
    credential: &str,
) {
    let connection =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (total, ready): (u32, u32) = connection.query_row(
                "SELECT COUNT(*),COALESCE(SUM(state='ready'),0) FROM session_bindings WHERE session_id=?1 AND credential_id=?2",
                [session, credential], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            if total == 1 && ready == 0 { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
}

async fn next(
    inbox: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>,
    kind: MessageType,
) -> UctpEnvelope {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = inbox.recv().await.unwrap();
            assert_ne!(
                event.msg_type,
                MessageType::Error,
                "wire fixture rejected: {}",
                event.payload
            );
            if event.msg_type == kind {
                return event;
            }
        }
    })
    .await
    .unwrap()
}
