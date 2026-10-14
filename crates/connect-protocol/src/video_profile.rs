//! Named, bounded Gate 5 VP8 interoperability profile. These limits describe
//! encoded media and do not imply that an assistant can interpret video.
use serde::{Deserialize, Serialize};

/// Versioned native/browser encoded-video profile.
pub const VIDEO_PROFILE: &str = "connect-video-vp8/1";
/// Maximum complete RTP packet, excluding the eight-byte UCTP header.
pub const MAX_VIDEO_RTP_BYTES: usize = 1180;
/// Maximum assembled encoded VP8 frame.
pub const MAX_VIDEO_FRAME_BYTES: usize = 256 * 1024;
/// Time allowed for incomplete frame assembly before discarding it.
pub const VIDEO_ASSEMBLY_MS: u64 = 200;
/// Maximum packet queue length at each video stream boundary.
pub const VIDEO_QUEUE_PACKETS: usize = 64;
/// Maximum locally queued video age before forwarding to the SDK caller.
pub const MAX_VIDEO_QUEUE_AGE_MS: i64 = 200;
/// VP8 RTP timestamp clock rate.
pub const VIDEO_CLOCK_HZ: u32 = 90_000;

/// Public capability limits for the named two-party VP8 profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoProfile {
    pub profile: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub max_fps: u32,
    pub target_bitrate_kbps: u32,
    pub max_rtp_bytes: usize,
    pub max_frame_bytes: usize,
    pub assembly_timeout_ms: u64,
    pub queue_packets: usize,
}
impl Default for VideoProfile {
    fn default() -> Self {
        Self {
            profile: VIDEO_PROFILE.into(),
            codec: "vp8".into(),
            width: 640,
            height: 360,
            max_fps: 30,
            target_bitrate_kbps: 1000,
            max_rtp_bytes: MAX_VIDEO_RTP_BYTES,
            max_frame_bytes: MAX_VIDEO_FRAME_BYTES,
            assembly_timeout_ms: VIDEO_ASSEMBLY_MS,
            queue_packets: VIDEO_QUEUE_PACKETS,
        }
    }
}
