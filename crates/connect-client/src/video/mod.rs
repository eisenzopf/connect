//! Native two-stream audio/video participation over authenticated UCTP/QUIC.
//! Encoded video is complete RTP, while PCMU audio remains codec payload bytes.
mod assembly;
mod diagnostics;
mod lifecycle;
mod packet;
mod signaling;
mod timebase;

use crate::{ClientError, NativeAudioFrame, Result};
pub use assembly::{
    NativeVideoFrame, VideoAssemblyDiagnostics, VideoFrameAssembler, VideoSequenceRange,
};
use connect_protocol::video_profile::VIDEO_QUEUE_PACKETS;
pub use packet::NativeVideoPacket;
use rvoip_core::{
    identity::PrincipalOwnershipKey,
    stream::{MediaFrame, MediaStream, StreamKind},
};
use rvoip_quic::QuicDatagramMediaStream;
use rvoip_uctp::{
    envelope::UctpEnvelope, payloads::stream, substrate::PeerMediaRouter, types::MessageType,
};
use std::{
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

/// A validated incoming frame on either independently negotiated media stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeAvFrame {
    Audio(NativeAudioFrame),
    Video(NativeVideoPacket),
}

/// One two-party native Session with separately negotiated audio and video.
/// Call [`Self::end`] for reliable connection teardown; Drop cancels local work.
pub struct NativeVideoSession {
    /// Display snapshot of the parent conversation; ending media does not close it.
    pub conversation_id: String,
    /// Display snapshot of the accepted audio/video Session.
    pub session_id: String,
    /// Display snapshot of this handle's live Connection. Editing display fields
    /// cannot retarget the private negotiated binding or its resource cleanup.
    pub connection_id: String,
    /// Named PCMU publisher for explicitly authorized subscriptions.
    pub audio_publisher_stream_id: String,
    /// Named VP8 publisher for explicitly authorized subscriptions.
    pub video_publisher_stream_id: String,
    audio: Arc<QuicDatagramMediaStream>,
    video: Arc<QuicDatagramMediaStream>,
    audio_in: mpsc::Receiver<MediaFrame>,
    video_in: mpsc::Receiver<MediaFrame>,
    events: broadcast::Receiver<UctpEnvelope>,
    router: Arc<PeerMediaRouter>,
    owner: PrincipalOwnershipKey,
    // Display metadata is not authority. Teardown/dispatch retain the original binding.
    binding: Arc<lifecycle::Binding>,
    connection: quinn::Connection,
    cancellation: CancellationToken,
    terminal_observer: tokio::task::JoinHandle<()>,
    terminal_reason: Arc<AtomicU8>,
    diagnostics: Arc<rvoip_core::media_diagnostics::MediaDiagnostics>,
    sequence: u16,
    timebase: timebase::MediaTimebase,
    camera_muted: bool,
    microphone_muted: bool,
    ended: bool,
}
impl NativeVideoSession {
    /// Local cancellation, remote hangup and transport closure are observable
    /// without polling incoming audio/video. This does not close the conversation.
    pub fn is_ended(&self) -> bool {
        self.ended || self.cancellation.is_cancelled() || self.connection.close_reason().is_some()
    }
    fn ended_error(&self) -> Option<ClientError> {
        if self.ended {
            return Some(ClientError::MediaEnded);
        }
        match self.terminal_reason.load(Ordering::Acquire) {
            2 => Some(ClientError::Transport),
            3 => Some(ClientError::Protocol),
            _ if self.connection.close_reason().is_some() => Some(ClientError::Transport),
            _ if self.cancellation.is_cancelled() => Some(ClientError::MediaEnded),
            _ => None,
        }
    }
    /// Mute outgoing camera frames without ending the Session or audio stream.
    pub fn set_camera_muted(&mut self, muted: bool) {
        self.camera_muted = muted;
    }
    /// Mute outgoing microphone frames independently from video.
    pub fn set_microphone_muted(&mut self, muted: bool) {
        self.microphone_muted = muted;
    }
    /// Send one encoded VP8 frame. Encoding/capture belongs to the caller.
    pub async fn send_vp8(&mut self, encoded: &[u8]) -> Result<()> {
        if let Some(error) = self.ended_error() {
            return Err(error);
        }
        if self.camera_muted {
            return Ok(());
        }
        let pt = self
            .video
            .codec()
            .payload_type
            .ok_or(ClientError::Protocol)?;
        let max_rtp = self
            .connection
            .max_datagram_size()
            .and_then(|n| n.checked_sub(8))
            .filter(|n| *n >= 512)
            .ok_or(ClientError::Transport)?
            .min(connect_protocol::video_profile::MAX_VIDEO_RTP_BYTES);
        let packets = packet::packetize_for_mtu(
            encoded,
            self.timebase.video(),
            &mut self.sequence,
            1,
            pt,
            max_rtp,
        )?;
        for packet in packets {
            self.send_frame(
                self.video.clone(),
                MediaFrame {
                    source_clock: None,
                    stream_id: self.video.id(),
                    kind: StreamKind::Video,
                    payload: packet.rtp.into(),
                    timestamp_rtp: packet.timestamp_rtp,
                    captured_at: chrono::Utc::now(),
                    payload_type: Some(pt),
                },
            )
            .await?;
        }
        Ok(())
    }
    /// Send exactly one negotiated 20 ms PCMU audio frame.
    pub async fn send_pcmu(&mut self, encoded: &[u8]) -> Result<()> {
        if let Some(error) = self.ended_error() {
            return Err(error);
        }
        if encoded.len() != 160 {
            return Err(ClientError::Protocol);
        }
        if self.microphone_muted {
            return Ok(());
        }
        let frame = MediaFrame {
            source_clock: None,
            stream_id: self.audio.id(),
            kind: StreamKind::Audio,
            payload: encoded.to_vec().into(),
            timestamp_rtp: self.timebase.audio(),
            captured_at: chrono::Utc::now(),
            payload_type: Some(0),
        };
        self.send_frame(self.audio.clone(), frame).await
    }
    async fn send_frame(
        &self,
        stream: Arc<QuicDatagramMediaStream>,
        frame: MediaFrame,
    ) -> Result<()> {
        if let Some(error) = self.ended_error() {
            return Err(error);
        }
        self.observe_media_queues();
        let tx = stream.frames_out();
        tokio::select! {
            _ = self.connection.closed() => Err(ClientError::Transport),
            _ = self.cancellation.cancelled() => Err(self.ended_error().unwrap_or(ClientError::MediaEnded)),
            result = tx.send(frame) => result.map_err(|_| self.ended_error().unwrap_or(ClientError::Transport)),
        }
    }
    /// Receive either audio or a complete RTP video packet with bounded waiting.
    /// Assemble packets with [`VideoFrameAssembler`] before independent decoding.
    pub async fn receive(&mut self, deadline: Duration) -> Result<NativeAvFrame> {
        if let Some(error) = self.ended_error() {
            return Err(error);
        }
        self.observe_media_queues();
        tokio::time::timeout(deadline,async {
            loop {
                tokio::select! {
                    _=self.connection.closed()=>return Err(ClientError::Transport),
                    _=self.cancellation.cancelled()=>return Err(self.ended_error().unwrap_or(ClientError::MediaEnded)),
                    event=self.events.recv()=>self.accept_event(event).await?,
                    frame=self.audio_in.recv()=>{
                        let f=frame.ok_or_else(||self.ended_error().unwrap_or(ClientError::Transport))?;
                        self.observe_received(&f, self.audio_in.len() + 1, self.audio_in.max_capacity());
                        return Ok(NativeAvFrame::Audio(NativeAudioFrame {payload:f.payload.to_vec(),timestamp_rtp:f.timestamp_rtp,payload_type:f.payload_type}));
                    },
                    frame=self.video_in.recv()=>{
                        let f=frame.ok_or_else(||self.ended_error().unwrap_or(ClientError::Transport))?;
                        if self.discard_stale_video(&f) {continue;}
                        self.observe_received(&f, self.video_in.len() + 1, self.video_in.max_capacity());
                        return Ok(NativeAvFrame::Video(NativeVideoPacket::parse(f.payload.to_vec())?));
                    },
                }
            }
        }).await.map_err(|_|ClientError::Timeout)?
    }
    async fn accept_event(
        &mut self,
        event: std::result::Result<UctpEnvelope, broadcast::error::RecvError>,
    ) -> Result<()> {
        let event = event.map_err(|_| ClientError::Transport)?;
        if crate::media_end::is_terminal(
            &event,
            &self.binding.conversation_id,
            &self.binding.session_id,
            &self.binding.connection_id,
        )? {
            self.stop_local();
            return Err(ClientError::MediaEnded);
        }
        if event.msg_type != MessageType::StreamOpened
            || event.sid.as_deref() != Some(&self.binding.session_id)
            || event.connid.as_deref() != Some(&self.binding.connection_id)
        {
            return Ok(());
        }
        let opened = event
            .decode_payload::<stream::StreamOpened>()
            .map_err(|_| ClientError::Protocol)?;
        let Some(id) = std::num::NonZeroU16::new(opened.stream.stream_local_id) else {
            return Err(ClientError::Protocol);
        };
        if self.router.lookup(id).is_some() {
            return Ok(());
        }
        let (stream, receiver) = signaling::bind(
            &self.router,
            &self.owner,
            &self.connection,
            &self.binding.session_id,
            &self.binding.connection_id,
            &opened,
            signaling::BindRuntime {
                cancellation: self.cancellation.clone(),
                diagnostics: self.diagnostics.clone(),
            },
        )?;
        match stream.kind() {
            StreamKind::Audio => self.audio_in = receiver,
            StreamKind::Video => self.video_in = receiver,
            _ => return Err(ClientError::Protocol),
        }
        Ok(())
    }
    /// Reliably end this Connection; conversation history remains persistent.
    pub async fn end(&mut self, client: &mut crate::ConnectClient) -> Result<()> {
        if !Arc::ptr_eq(&client.media_router, &self.router) {
            return Err(ClientError::Protocol);
        }
        if self.is_ended() {
            self.stop_local();
            return Ok(());
        }
        if !Arc::ptr_eq(&self.router, &client.media_router) {
            return Err(ClientError::Protocol);
        }
        self.stop_local();
        tokio::time::timeout(
            Duration::from_secs(5),
            client.peer.send(
                UctpEnvelope::new(
                    MessageType::ConnectionEnd,
                    serde_json::json!({"reason_code":200,"reason":"client-ended"}),
                )
                .with_sid(&self.binding.session_id)
                .with_connid(&self.binding.connection_id),
            ),
        )
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(|_| ClientError::Transport)?;
        Ok(())
    }
    fn stop_local(&mut self) {
        self.ended = true;
        self.cancellation.cancel();
        self.terminal_observer.abort();
        self.router
            .remove_connection(&rvoip_uctp::substrate::PeerMediaConnectionKey::new(
                rvoip_core::ids::SessionId::from_string(&self.binding.session_id),
                rvoip_core::ids::ConnectionId::from_string(&self.binding.connection_id),
            ));
    }
}
impl Drop for NativeVideoSession {
    fn drop(&mut self) {
        self.stop_local();
    }
}
