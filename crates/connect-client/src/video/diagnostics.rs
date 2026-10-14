//! Sample local channel occupancy without retaining media or peer identifiers.
use super::NativeVideoSession;
use rvoip_core::{
    media_diagnostics::{MediaDiagnosticsSnapshot, QueueStage},
    stream::{MediaFrame, MediaStream},
};

impl NativeVideoSession {
    /// Numeric observations for this handle's native media streams.
    ///
    /// Channel depths are sampled, not exact peaks; outgoing occupancy includes
    /// reserved sender capacity. The QUIC byte buffer belongs to the shared
    /// physical connection. Quinn does not expose its queued age or evictions.
    /// This snapshot contains no credentials, addresses, IDs or media bytes and
    /// does not assert delivery, independent decoding or physical render time.
    pub fn media_diagnostics(&self) -> MediaDiagnosticsSnapshot {
        self.observe_media_queues();
        self.diagnostics.snapshot()
    }

    pub(super) fn observe_media_queues(&self) {
        for (stage, rx) in [
            (QueueStage::QuicAudioIn, &self.audio_in),
            (QueueStage::QuicVideoIn, &self.video_in),
        ] {
            self.diagnostics
                .observe_queue(stage, rx.max_capacity(), rx.len());
        }
        for stream in [&self.audio, &self.video] {
            let tx = stream.frames_out();
            if let Some((_, output, _)) = QueueStage::quic(stream.kind()) {
                self.diagnostics.observe_queue(
                    output,
                    tx.max_capacity(),
                    tx.max_capacity().saturating_sub(tx.capacity()),
                );
            }
        }
    }

    pub(super) fn observe_received(&self, frame: &MediaFrame, depth: usize, capacity: usize) {
        if let Some((input, _, _)) = QueueStage::quic(frame.kind) {
            // The receiver has removed one frame; include it in this poll sample.
            self.diagnostics.observe_queue(input, capacity, depth);
            self.diagnostics.forwarded_age(
                input,
                (chrono::Utc::now() - frame.captured_at).num_milliseconds(),
            );
        }
    }

    pub(super) fn discard_stale_video(&self, frame: &MediaFrame) -> bool {
        if (chrono::Utc::now() - frame.captured_at).num_milliseconds()
            > connect_protocol::video_profile::MAX_VIDEO_QUEUE_AGE_MS
        {
            self.diagnostics.stale_discard(QueueStage::QuicVideoIn);
            true
        } else {
            false
        }
    }
}
