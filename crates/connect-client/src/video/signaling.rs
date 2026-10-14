//! Reliable UCTP negotiation and ownership-checked stream registration.
use super::*;
use crate::{ClientError, ConnectClient, Result};
use rvoip_core::{
    capability::CodecInfo,
    connection::Direction,
    ids::StreamId,
    stream::{MediaFrame, MediaStream, StreamKind},
};
use rvoip_quic::{QuicDatagramMediaStream, QuicMediaQueueConfig};
use rvoip_uctp::{
    payloads::{connection, session, stream},
    substrate::{PeerMediaRegistration, PeerMediaRouteKey},
};
use std::{num::NonZeroUsize, sync::Arc};

pub(super) struct BindRuntime {
    pub cancellation: CancellationToken,
    pub diagnostics: Arc<rvoip_core::media_diagnostics::MediaDiagnostics>,
}

pub(super) fn bind(
    router: &Arc<PeerMediaRouter>,
    owner: &PrincipalOwnershipKey,
    physical: &quinn::Connection,
    sid: &str,
    cid: &str,
    opened: &stream::StreamOpened,
    runtime: BindRuntime,
) -> Result<(Arc<QuicDatagramMediaStream>, mpsc::Receiver<MediaFrame>)> {
    let (kind, codec, capacity) = profile_codec(opened)?;
    let reservation = router.reserve().map_err(|_| ClientError::Protocol)?;
    if reservation.local_id().get() != opened.stream.stream_local_id {
        return Err(ClientError::Protocol);
    }
    let id = StreamId::from_string(&opened.stream.strm_id);
    let media = QuicDatagramMediaStream::start_with_queue_config(
        id.clone(),
        kind,
        codec,
        Direction::Inbound,
        opened.stream.stream_local_id,
        physical.clone(),
        QuicMediaQueueConfig {
            capacity,
            cancellation: runtime.cancellation,
        },
    );
    media.set_media_diagnostics(runtime.diagnostics);
    let rx = media.try_frames_in().map_err(|_| ClientError::Transport)?;
    reservation
        .commit(PeerMediaRegistration::new(
            owner.clone(),
            PeerMediaRouteKey::new(
                rvoip_core::ids::SessionId::from_string(sid),
                rvoip_core::ids::ConnectionId::from_string(cid),
                id,
            ),
            media.clone() as Arc<dyn MediaStream>,
            media.inbound_tx(),
        ))
        .map_err(|_| ClientError::Protocol)?;
    Ok((media, rx))
}

fn profile_codec(opened: &stream::StreamOpened) -> Result<(StreamKind, CodecInfo, NonZeroUsize)> {
    let (kind, codec, capacity) = match opened.stream.kind.as_str() {
        "audio" => (
            StreamKind::Audio,
            CodecInfo {
                name: "g.711-mu".into(),
                clock_rate_hz: 8000,
                channels: 1,
                fmtp: None,
                payload_type: Some(0),
            },
            connect_protocol::media_profile::AUDIO_QUEUE_CAPACITY,
        ),
        "video" => (
            StreamKind::Video,
            CodecInfo {
                name: "vp8".into(),
                clock_rate_hz: 90000,
                channels: 0,
                fmtp: None,
                payload_type: Some(96),
            },
            NonZeroUsize::new(VIDEO_QUEUE_PACKETS).ok_or(ClientError::Protocol)?,
        ),
        _ => return Err(ClientError::Protocol),
    };
    // Stock UCTP announces the selected codec by name. A full codec
    // description may also be supplied; any supplied fields must agree with
    // this fixed profile rather than silently changing its clock or RTP PT.
    let metadata = &opened.stream.codec;
    let name = metadata
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or(ClientError::Protocol)?;
    if !name.eq_ignore_ascii_case(&codec.name) {
        return Err(ClientError::Protocol);
    }
    for (field, expected) in [
        ("clock_rate_hz", u64::from(codec.clock_rate_hz)),
        ("channels", u64::from(codec.channels)),
        (
            "payload_type",
            u64::from(codec.payload_type.ok_or(ClientError::Protocol)?),
        ),
    ] {
        if let Some(value) = metadata.get(field).filter(|v| !v.is_null()) {
            if value.as_u64() != Some(expected) {
                return Err(ClientError::Protocol);
            }
        }
    }
    if metadata.get("fmtp").is_some_and(|value| !value.is_null()) {
        return Err(ClientError::Protocol);
    }
    Ok((kind, codec, capacity))
}

fn negotiation_error_matches(
    event: &UctpEnvelope,
    requests: &[String],
    sid: &str,
    connid: &str,
) -> bool {
    if event.msg_type != MessageType::Error {
        return false;
    }
    match event.in_reply_to.as_deref() {
        Some(id) => requests.iter().any(|request| request == id),
        None => event.sid.as_deref() == Some(sid) && event.connid.as_deref() == Some(connid),
    }
}
impl ConnectClient {
    /// Join an accepted durable Session using separate PCMU and VP8 streams.
    /// The caller owns capture/encoding; live packets never become MCP tool data.
    pub async fn open_native_video(
        &mut self,
        conversation_id: &str,
        session_id: &str,
        remote_identity_id: &str,
    ) -> Result<NativeVideoSession> {
        let cid = format!("conn-{}", uuid::Uuid::new_v4());
        let audio_id = format!("audio-{}-{session_id}", self.identity_id);
        let video_id = format!("video-{}-{session_id}", self.identity_id);
        let mut events = self.events.subscribe();
        let terminal_events = self.events.subscribe();
        let cancellation = CancellationToken::new();
        let result = tokio::time::timeout(self.deadline, async {
            let invite = session::SessionInvite {
                from: self.participant_id.clone(),
                to: vec![remote_identity_id.into()],
                medium: "voice-video".into(),
                intent: "connect-session".into(),
                capabilities_offer: serde_json::json!({"audio":["g.711-mu"],"video":["vp8"]}),
            };
            let invite = UctpEnvelope::new(
                MessageType::SessionInvite,
                serde_json::to_value(invite).map_err(|_| ClientError::Protocol)?,
            )
            .with_cid(conversation_id)
            .with_sid(session_id);
            let offer = connection::ConnectionOffer {
                by_participant: self.participant_id.clone(),
                substrate: "quic".into(),
                capabilities: serde_json::json!({}),
                streams_offered: vec![
                    connection::StreamOffer {
                        id: audio_id.clone(),
                        kind: "audio".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["g.711-mu".into()],
                    },
                    connection::StreamOffer {
                        id: video_id.clone(),
                        kind: "video".into(),
                        direction: "sendrecv".into(),
                        codec_preferences: vec!["vp8".into()],
                    },
                ],
                substrate_setup: serde_json::Value::Null,
            };
            let offer = UctpEnvelope::new(
                MessageType::ConnectionOffer,
                serde_json::to_value(offer).map_err(|_| ClientError::Protocol)?,
            )
            .with_sid(session_id)
            .with_connid(&cid);
            let ready = UctpEnvelope::new(MessageType::ConnectionReady, serde_json::json!({}))
                .with_sid(session_id)
                .with_connid(&cid);
            let requests = [invite.id.clone(), offer.id.clone(), ready.id.clone()];
            // The one outer deadline includes control-queue backpressure and
            // all negotiation replies, rather than starting after three writes.
            for envelope in [invite, offer, ready] {
                self.peer
                    .send(envelope)
                    .await
                    .map_err(|_| ClientError::Transport)?;
            }
            let opened = async {
                let mut opened = Vec::new();
                while opened.len() < 2 {
                    let event = events.recv().await.map_err(|_| ClientError::Transport)?;
                    if negotiation_error_matches(&event, &requests, session_id, &cid) {
                        return Err(crate::media::negotiation_error(&event));
                    }
                    if crate::media_end::is_terminal(&event, conversation_id, session_id, &cid)? {
                        return Err(ClientError::MediaEnded);
                    }
                    if event.msg_type == MessageType::ConnectionAnswer
                        && event.sid.as_deref() == Some(session_id)
                        && event.connid.as_deref() == Some(&cid)
                    {
                        let answer = event
                            .decode_payload::<connection::ConnectionAnswer>()
                            .map_err(|_| ClientError::Protocol)?;
                        if answer.streams_answered.len() != 2
                            || !answer
                                .streams_answered
                                .iter()
                                .any(|s| s.id == audio_id && s.kind == "audio")
                            || !answer
                                .streams_answered
                                .iter()
                                .any(|s| s.id == video_id && s.kind == "video")
                        {
                            return Err(ClientError::Application(connect_protocol::Error::new(
                                "unsupported-media-profile",
                                "The peer did not accept both audio and VP8 video",
                            )));
                        }
                    }
                    if event.msg_type == MessageType::StreamOpened
                        && event.sid.as_deref() == Some(session_id)
                        && event.connid.as_deref() == Some(&cid)
                    {
                        let stream = event
                            .decode_payload::<stream::StreamOpened>()
                            .map_err(|_| ClientError::Protocol)?;
                        if (stream.stream.strm_id != audio_id && stream.stream.strm_id != video_id)
                            || opened.iter().any(|s: &stream::StreamOpened| {
                                s.stream.strm_id == stream.stream.strm_id
                            })
                        {
                            return Err(ClientError::Protocol);
                        }
                        opened.push(stream);
                    }
                }
                Ok(opened)
            }
            .await?;
            let mut opened = opened;
            opened.sort_by_key(|s| s.stream.stream_local_id);
            let owner = PrincipalOwnershipKey {
                issuer: None,
                tenant: None,
                subject: self.identity_id.clone(),
            };
            let diagnostics = Arc::new(rvoip_core::media_diagnostics::MediaDiagnostics::default());
            diagnostics.configure_quic_send_buffer(
                connect_protocol::video_profile::VIDEO_QUEUE_PACKETS
                    * (connect_protocol::video_profile::MAX_VIDEO_RTP_BYTES + 8),
            );
            let mut audio = None;
            let mut video = None;
            for s in &opened {
                let (media, rx) = bind(
                    &self.media_router,
                    &owner,
                    &self.peer.connection,
                    session_id,
                    &cid,
                    s,
                    BindRuntime {
                        cancellation: cancellation.clone(),
                        diagnostics: diagnostics.clone(),
                    },
                )?;
                match media.kind() {
                    StreamKind::Audio => audio = Some((media, rx)),
                    StreamKind::Video => video = Some((media, rx)),
                    _ => return Err(ClientError::Protocol),
                }
            }
            let (audio, audio_in) = audio.ok_or(ClientError::Protocol)?;
            let (video, video_in) = video.ok_or(ClientError::Protocol)?;
            let terminal_reason = Arc::new(AtomicU8::new(0));
            let binding = Arc::new(super::lifecycle::Binding {
                conversation_id: conversation_id.into(),
                session_id: session_id.into(),
                connection_id: cid.clone(),
            });
            Ok(NativeVideoSession {
                conversation_id: conversation_id.into(),
                session_id: session_id.into(),
                connection_id: cid.clone(),
                audio_publisher_stream_id: audio_id,
                video_publisher_stream_id: video_id,
                audio,
                video,
                audio_in,
                video_in,
                events,
                router: self.media_router.clone(),
                owner,
                binding: binding.clone(),
                connection: self.peer.connection.clone(),
                cancellation: cancellation.clone(),
                terminal_observer: super::lifecycle::observe(
                    terminal_events,
                    self.peer.connection.clone(),
                    self.media_router.clone(),
                    cancellation.clone(),
                    terminal_reason.clone(),
                    binding,
                ),
                terminal_reason,
                diagnostics,
                sequence: 0,
                timebase: timebase::MediaTimebase::default(),
                camera_muted: false,
                microphone_muted: false,
                ended: false,
            })
        })
        .await
        .unwrap_or(Err(ClientError::Timeout));
        if result.is_err() {
            cancellation.cancel();
            self.media_router.remove_connection(
                &rvoip_uctp::substrate::PeerMediaConnectionKey::new(
                    rvoip_core::ids::SessionId::from_string(session_id),
                    rvoip_core::ids::ConnectionId::from_string(&cid),
                ),
            );
            if matches!(
                &result,
                Err(ClientError::Timeout | ClientError::Protocol | ClientError::Transport)
            ) {
                // Partial negotiation may have consumed peer-global stream IDs.
                // Reconnect with current authority instead of reusing an
                // uncertain allocator, as other native exchanges already do.
                self.disconnect();
                return result;
            }
            // Preserve the original negotiation failure. Cleanup is bounded and
            // cannot turn an uncertain or rejected join into a success.
            let _ = tokio::time::timeout(
                self.deadline.min(Duration::from_secs(5)),
                self.peer.send(
                    UctpEnvelope::new(
                        MessageType::ConnectionEnd,
                        serde_json::json!({"reason_code":500,"reason":"negotiation-failed"}),
                    )
                    .with_sid(session_id)
                    .with_connid(&cid),
                ),
            )
            .await;
        }
        result
    }
    /// Subscribe to the one native audio/video publisher for a session participant.
    /// The profile permits one published audio/video connection per participant.
    pub async fn subscribe_native_video_participant(
        &mut self,
        session: &NativeVideoSession,
        participant: &str,
    ) -> Result<()> {
        self.subscribe_native_video(
            session,
            &format!("video-{participant}-{}", session.binding.session_id),
            &format!("audio-{participant}-{}", session.binding.session_id),
        )
        .await
    }
    /// Subscribe a native/native participant to its permitted counterpart's media.
    /// Direct browser/native bridges own their return stream and need no subscription.
    pub async fn subscribe_native_video(
        &mut self,
        session: &NativeVideoSession,
        video_publisher: &str,
        audio_publisher: &str,
    ) -> Result<()> {
        if !Arc::ptr_eq(&self.media_router, &session.router) {
            return Err(ClientError::Protocol);
        }
        let request = stream::StreamSubscribe {
            by_participant: self.participant_id.clone(),
            subscriptions: [video_publisher, audio_publisher]
                .into_iter()
                .map(|id| stream::StreamSubscription {
                    strm_id: Some(id.into()),
                    from_participant: None,
                    kinds: vec![],
                })
                .collect(),
        };
        let reply = self
            .exchange(
                UctpEnvelope::new(
                    MessageType::StreamSubscribe,
                    serde_json::to_value(request).map_err(|_| ClientError::Protocol)?,
                )
                .with_sid(&session.binding.session_id)
                .with_connid(&session.binding.connection_id),
            )
            .await?;
        if reply.msg_type == MessageType::Ack {
            Ok(())
        } else {
            Err(crate::media::negotiation_error(&reply))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn negotiation_errors_require_current_request_or_exact_connection() {
        let requests: [String; 3] = ["invite".into(), "offer".into(), "ready".into()];
        for id in &requests {
            let mut error = UctpEnvelope::new(MessageType::Error, serde_json::json!({}));
            error.in_reply_to = Some(id.clone());
            assert!(negotiation_error_matches(
                &error,
                &requests,
                "session",
                "connection"
            ));
        }
        let mut unrelated = UctpEnvelope::new(MessageType::Error, serde_json::json!({}))
            .with_sid("session")
            .with_connid("connection");
        assert!(negotiation_error_matches(
            &unrelated,
            &requests,
            "session",
            "connection"
        ));
        unrelated.in_reply_to = Some("other-request".into());
        assert!(!negotiation_error_matches(
            &unrelated,
            &requests,
            "session",
            "connection"
        ));
        unrelated.in_reply_to = None;
        assert!(!negotiation_error_matches(
            &unrelated,
            &requests,
            "other-session",
            "connection"
        ));
        assert!(!negotiation_error_matches(
            &unrelated,
            &requests,
            "session",
            "other-connection"
        ));
        let unscoped = UctpEnvelope::new(MessageType::Error, serde_json::json!({}));
        assert!(!negotiation_error_matches(
            &unscoped,
            &requests,
            "session",
            "connection"
        ));
    }
    #[test]
    fn selected_codec_metadata_cannot_silently_change_the_fixed_profile() {
        for (kind, name, clock, channels, pt) in [
            ("audio", "g.711-mu", 8000, 1, 0),
            ("video", "vp8", 90000, 0, 96),
        ] {
            let mut opened = stream::StreamOpened {
                stream: stream::StreamInfo {
                    strm_id: "publisher".into(),
                    kind: kind.into(),
                    codec: serde_json::json!({"name":name,"clock_rate_hz":clock,"channels":channels,"payload_type":pt,"fmtp":null}),
                    direction: "sendrecv".into(),
                    stream_local_id: 1,
                    opened_at: chrono::Utc::now(),
                },
            };
            assert!(profile_codec(&opened).is_ok());
            let valid = opened.stream.codec.clone();
            for field in ["clock_rate_hz", "channels", "payload_type"] {
                opened.stream.codec = valid.clone();
                opened.stream.codec[field] = serde_json::json!(valid[field].as_u64().unwrap() + 1);
                assert!(
                    matches!(profile_codec(&opened), Err(ClientError::Protocol)),
                    "{kind} {field}"
                );
                opened.stream.codec[field] = serde_json::json!("invalid");
                assert!(matches!(profile_codec(&opened), Err(ClientError::Protocol)));
            }
            opened.stream.codec = valid.clone();
            opened.stream.codec["fmtp"] = serde_json::json!("unsupported-profile");
            assert!(matches!(profile_codec(&opened), Err(ClientError::Protocol)));
            opened.stream.codec = serde_json::json!({"name":name});
            assert!(
                profile_codec(&opened).is_ok(),
                "legacy fixed-name profile remains supported"
            );
        }
    }
}
