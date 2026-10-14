//! Native UCTP/QUIC audio binding. Control remains on `connect-control/1`;
//! audio uses stock UCTP session/connection/stream envelopes and RTP datagrams.
use crate::{ClientError, ConnectClient, Result};
use rvoip_core::{
    capability::CodecInfo,
    connection::Direction,
    ids::StreamId,
    stream::{MediaFrame, MediaStream, StreamKind},
};
use rvoip_quic::{QuicDatagramMediaStream, QuicMediaQueueConfig};
use rvoip_uctp::{
    envelope::UctpEnvelope,
    payloads::{connection, session, stream},
    substrate::{PeerMediaRegistration, PeerMediaRouteKey},
    types::MessageType,
};
use std::{fmt, sync::Arc, time::Duration};

/// Live native UCTP/QUIC audio leg bound to one durable Session.
pub struct NativeAudioSession {
    /// Durable parent conversation identifier.
    pub conversation_id: String,
    /// Durable bounded Session identifier.
    pub session_id: String,
    /// Live UCTP Connection identifier.
    pub connection_id: String,
    /// Published audio Stream identifier used by subscribers.
    pub publisher_stream_id: String,
    /// Peer-local datagram routing identifier.
    pub stream_local_id: u16,
    stream: Arc<QuicDatagramMediaStream>,
    inbound: Option<tokio::sync::mpsc::Receiver<MediaFrame>>,
    events: tokio::sync::broadcast::Receiver<UctpEnvelope>,
    media_router: Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    connection: quinn::Connection,
    owner: rvoip_core::identity::PrincipalOwnershipKey,
    next_timestamp: u32,
    ended: bool,
}

#[derive(Clone, PartialEq, Eq)]
/// One received native audio datagram after UCTP/RTP validation.
pub struct NativeAudioFrame {
    /// Encoded audio payload. Gate 3 uses a 160-byte PCMU frame.
    pub payload: Vec<u8>,
    /// RTP clock timestamp.
    pub timestamp_rtp: u32,
    /// Negotiated RTP payload type when present.
    pub payload_type: Option<u8>,
}

impl fmt::Debug for NativeAudioFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAudioFrame")
            .field("bytes", &self.payload.len())
            .field("timestamp_rtp", &self.timestamp_rtp)
            .field("payload_type", &self.payload_type)
            .finish()
    }
}

impl NativeAudioSession {
    /// Send one 20 ms PCMU frame (160 samples at 8 kHz).
    pub async fn send_pcmu(&mut self, payload: &[u8]) -> Result<()> {
        if self.ended {
            return Err(ClientError::MediaEnded);
        }
        if self.connection.close_reason().is_some() {
            return Err(ClientError::Transport);
        }
        if payload.len() != 160 {
            return Err(ClientError::Protocol);
        }
        let frame = MediaFrame {
            source_clock: None,
            stream_id: self.stream.id(),
            kind: StreamKind::Audio,
            payload: payload.to_vec().into(),
            timestamp_rtp: self.next_timestamp,
            captured_at: chrono::Utc::now(),
            payload_type: Some(0),
        };
        self.next_timestamp = self.next_timestamp.wrapping_add(160);
        let frames = self.stream.frames_out();
        tokio::select! {
            _ = self.connection.closed() => Err(ClientError::Transport),
            result = frames.send(frame) => result.map_err(|_| ClientError::Transport),
        }
    }

    /// Receive one authorized media frame before the caller's deadline.
    /// Reliable normal hangup returns [`ClientError::MediaEnded`]; physical
    /// peer closure returns [`ClientError::Transport`] without waiting for
    /// the audio deadline, including authorization expiry or revocation.
    pub async fn receive(&mut self, timeout: Duration) -> Result<NativeAudioFrame> {
        if self.ended {
            return Err(ClientError::MediaEnded);
        }
        let frame = tokio::time::timeout(timeout, async {
            loop {
                if let Some(inbound) = self.inbound.as_mut() {
                    tokio::select! {
                        biased;
                        event = self.events.recv() => self.accept_stream_event(event).await?,
                        _ = self.connection.closed() => return Err(ClientError::Transport),
                        frame = inbound.recv() => return frame.ok_or(ClientError::Transport),
                    }
                } else {
                    tokio::select! {
                        biased;
                        event = self.events.recv() => self.accept_stream_event(event).await?,
                        _ = self.connection.closed() => return Err(ClientError::Transport),
                    }
                }
            }
        })
        .await
        .map_err(|_| ClientError::Timeout)??;
        Ok(NativeAudioFrame {
            payload: frame.payload.to_vec(),
            timestamp_rtp: frame.timestamp_rtp,
            payload_type: frame.payload_type,
        })
    }

    async fn accept_stream_event(
        &mut self,
        event: std::result::Result<UctpEnvelope, tokio::sync::broadcast::error::RecvError>,
    ) -> Result<()> {
        let event = event.map_err(|_| ClientError::Transport)?;
        if crate::media_end::is_terminal(
            &event,
            &self.conversation_id,
            &self.session_id,
            &self.connection_id,
        )? {
            self.ended = true;
            return Err(ClientError::MediaEnded);
        }
        if event.msg_type != MessageType::StreamOpened
            || event.sid.as_deref() != Some(self.session_id.as_str())
            || event.connid.as_deref() != Some(self.connection_id.as_str())
        {
            return Ok(());
        }
        let opened = event
            .decode_payload::<stream::StreamOpened>()
            .map_err(|_| ClientError::Protocol)?;
        tracing::debug!(
            stream_local_id = opened.stream.stream_local_id,
            stream_id = %opened.stream.strm_id,
            "Connect client registering subscriber media stream"
        );
        if opened.stream.kind != "audio" || opened.stream.stream_local_id == 0 {
            return Ok(());
        }
        let Some(local_id) = std::num::NonZeroU16::new(opened.stream.stream_local_id) else {
            return Ok(());
        };
        if self.media_router.lookup(local_id).is_some() {
            return Ok(());
        }
        let codec = pcmu_codec();
        let stream_id = StreamId::from_string(opened.stream.strm_id);
        let stream = QuicDatagramMediaStream::start_with_queue_config(
            stream_id.clone(),
            StreamKind::Audio,
            codec,
            Direction::Inbound,
            opened.stream.stream_local_id,
            self.connection.clone(),
            QuicMediaQueueConfig {
                capacity: connect_protocol::media_profile::AUDIO_QUEUE_CAPACITY,
                ..Default::default()
            },
        );
        let inbound = stream.try_frames_in().map_err(|_| ClientError::Transport)?;
        register_stream(
            &self.media_router,
            self.owner.clone(),
            &self.session_id,
            &self.connection_id,
            stream_id,
            stream.clone(),
            opened.stream.stream_local_id,
        )?;
        self.inbound = Some(inbound);
        Ok(())
    }
}

fn pcmu_codec() -> CodecInfo {
    CodecInfo {
        name: "g.711-mu".into(),
        clock_rate_hz: 8_000,
        channels: 1,
        fmtp: None,
        payload_type: Some(0),
    }
}

fn register_stream(
    router: &Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    owner: rvoip_core::identity::PrincipalOwnershipKey,
    session_id: &str,
    connection_id: &str,
    stream_id: StreamId,
    stream: Arc<QuicDatagramMediaStream>,
    expected_local_id: u16,
) -> Result<()> {
    let reservation = router.reserve().map_err(|_| ClientError::Protocol)?;
    if reservation.local_id().get() != expected_local_id {
        return Err(ClientError::Protocol);
    }
    let stream_dyn: Arc<dyn MediaStream> = stream.clone();
    reservation
        .commit(PeerMediaRegistration::new(
            owner,
            PeerMediaRouteKey::new(
                rvoip_core::ids::SessionId::from_string(session_id),
                rvoip_core::ids::ConnectionId::from_string(connection_id),
                stream_id,
            ),
            stream_dyn,
            stream.inbound_tx(),
        ))
        .map_err(|_| ClientError::Protocol)?;
    Ok(())
}

impl ConnectClient {
    /// Bind this authenticated peer to an already accepted durable Session.
    /// Gate 3's initial native media profile is PCMU/8 kHz, mono, 20 ms.
    pub async fn open_native_audio(
        &mut self,
        conversation_id: &str,
        session_id: &str,
        remote_identity_id: &str,
    ) -> Result<NativeAudioSession> {
        let connection_id = format!("conn-{}", uuid::Uuid::new_v4());
        let publisher_stream_id = format!("audio-{}", self.identity_id);
        // Subscribe before emitting any lifecycle envelope. StreamOpened is an
        // unsolicited reliable event; creating the receiver afterward leaves
        // a real race in which a fast local server can publish it first.
        let mut events = self.events.subscribe();
        self.peer
            .send(
                UctpEnvelope::new(
                    MessageType::SessionInvite,
                    serde_json::to_value(session::SessionInvite {
                        from: self.participant_id.clone(),
                        to: vec![remote_identity_id.into()],
                        medium: "voice".into(),
                        intent: "connect-session".into(),
                        capabilities_offer: serde_json::json!({"audio":["g.711-mu"]}),
                    })
                    .map_err(|_| ClientError::Protocol)?,
                )
                .with_cid(conversation_id)
                .with_sid(session_id),
            )
            .await
            .map_err(|_| ClientError::Transport)?;
        self.peer
            .send(
                UctpEnvelope::new(
                    MessageType::ConnectionOffer,
                    serde_json::to_value(connection::ConnectionOffer {
                        by_participant: self.participant_id.clone(),
                        substrate: "quic".into(),
                        capabilities: serde_json::json!({}),
                        streams_offered: vec![connection::StreamOffer {
                            id: publisher_stream_id.clone(),
                            kind: "audio".into(),
                            direction: "sendrecv".into(),
                            codec_preferences: vec!["g.711-mu".into()],
                        }],
                        substrate_setup: serde_json::Value::Null,
                    })
                    .map_err(|_| ClientError::Protocol)?,
                )
                .with_sid(session_id)
                .with_connid(connection_id.clone()),
            )
            .await
            .map_err(|_| ClientError::Transport)?;
        self.peer
            .send(
                UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
                    .with_sid(session_id)
                    .with_connid(connection_id.clone()),
            )
            .await
            .map_err(|_| ClientError::Transport)?;

        let opened = tokio::time::timeout(self.deadline, async {
            loop {
                let envelope = events.recv().await.map_err(|_| ClientError::Transport)?;
                if envelope.msg_type == MessageType::Error {
                    return Err(negotiation_error(&envelope));
                }
                if envelope.msg_type == MessageType::StreamOpened
                    && envelope.sid.as_deref() == Some(session_id)
                    && envelope.connid.as_deref() == Some(connection_id.as_str())
                {
                    return envelope
                        .decode_payload::<stream::StreamOpened>()
                        .map_err(|_| ClientError::Protocol);
                }
            }
        })
        .await
        .map_err(|_| ClientError::Timeout)??;
        if opened.stream.strm_id != publisher_stream_id
            || opened.stream.kind != "audio"
            || opened.stream.stream_local_id == 0
        {
            return Err(ClientError::Protocol);
        }
        let codec = pcmu_codec();
        let owner = rvoip_core::identity::PrincipalOwnershipKey {
            issuer: None,
            tenant: None,
            subject: self.identity_id.clone(),
        };
        let stream = QuicDatagramMediaStream::start_with_queue_config(
            StreamId::from_string(publisher_stream_id.clone()),
            StreamKind::Audio,
            codec,
            Direction::Inbound,
            opened.stream.stream_local_id,
            self.peer.connection.clone(),
            QuicMediaQueueConfig {
                capacity: connect_protocol::media_profile::AUDIO_QUEUE_CAPACITY,
                ..Default::default()
            },
        );
        // A point-to-point bridge (for example Vapi) can send return audio on
        // the already negotiated sendrecv stream. Multi-party subscriptions
        // may later announce a separate recvonly stream and replace this
        // receiver in `accept_stream_event`.
        let inbound = stream.try_frames_in().map_err(|_| ClientError::Transport)?;
        register_stream(
            &self.media_router,
            owner.clone(),
            session_id,
            &connection_id,
            StreamId::from_string(publisher_stream_id.clone()),
            stream.clone(),
            opened.stream.stream_local_id,
        )?;
        Ok(NativeAudioSession {
            conversation_id: conversation_id.into(),
            session_id: session_id.into(),
            connection_id,
            publisher_stream_id,
            stream_local_id: opened.stream.stream_local_id,
            stream,
            inbound: Some(inbound),
            events,
            media_router: self.media_router.clone(),
            connection: self.peer.connection.clone(),
            owner,
            next_timestamp: 0,
            ended: false,
        })
    }

    /// Subscribe this connection to an authorized publisher Stream.
    pub async fn subscribe_native_audio(
        &mut self,
        session: &NativeAudioSession,
        publisher_stream_id: &str,
    ) -> Result<()> {
        let reply = self
            .exchange(
                UctpEnvelope::new(
                    MessageType::StreamSubscribe,
                    serde_json::to_value(stream::StreamSubscribe {
                        by_participant: self.participant_id.clone(),
                        subscriptions: vec![stream::StreamSubscription {
                            strm_id: Some(publisher_stream_id.into()),
                            from_participant: None,
                            kinds: Vec::new(),
                        }],
                    })
                    .map_err(|_| ClientError::Protocol)?,
                )
                .with_sid(&session.session_id)
                .with_connid(&session.connection_id),
            )
            .await?;
        if reply.msg_type == MessageType::Ack {
            Ok(())
        } else {
            Err(ClientError::Protocol)
        }
    }

    /// End this live Connection without closing the durable Conversation.
    pub async fn close_native_audio(&mut self, session: &NativeAudioSession) -> Result<()> {
        self.peer
            .send(
                UctpEnvelope::new(
                    MessageType::ConnectionEnd,
                    serde_json::to_value(connection::ConnectionEnd {
                        reason_code: 200,
                        reason: "client-ended".into(),
                    })
                    .map_err(|_| ClientError::Protocol)?,
                )
                .with_sid(&session.session_id)
                .with_connid(&session.connection_id),
            )
            .await
            .map_err(|_| ClientError::Transport)
    }
}

pub(crate) fn negotiation_error(envelope: &UctpEnvelope) -> ClientError {
    let (code, message) = match envelope.payload.get("code").and_then(|v| v.as_u64()) {
        Some(401) => (
            "unauthenticated",
            "Native media authentication was rejected",
        ),
        Some(403) => ("forbidden", "Native media is outside current authority"),
        Some(404) => (
            "media-peer-not-ready",
            "The requested media publisher is not ready",
        ),
        Some(429) => (
            "capacity-exceeded",
            "The media route has reached its bounded capacity",
        ),
        Some(488 | 501) => (
            "unsupported-media-profile",
            "Native media route is not supported by this profile",
        ),
        Some(503) => ("media-unavailable", "Native media route is unavailable"),
        _ => return ClientError::Protocol,
    };
    ClientError::Application(connect_protocol::Error::new(code, message))
}

#[cfg(test)]
mod frame_debug_tests {
    use super::*;

    #[test]
    fn frame_debug_exposes_metadata_without_recorded_media() {
        let frame = NativeAudioFrame {
            payload: vec![123; 160],
            timestamp_rtp: 400,
            payload_type: Some(0),
        };
        let debug = format!("{frame:?}");
        assert!(debug.contains("bytes: 160") && debug.contains("timestamp_rtp: 400"));
        assert!(!debug.contains("123") && debug.len() < 120);
    }
}
