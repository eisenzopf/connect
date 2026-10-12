//! Local queue limits for Gate 3's negotiated 20 ms audio frames.
//!
//! These limits affect buffering, not the UCTP wire protocol. A stalled
//! consumer must lose obsolete media rather than accumulate seconds of speech.

use std::{num::NonZeroUsize, time::Duration};

/// Duration of the native PCMU and negotiated browser Opus audio frame.
pub const AUDIO_FRAME_DURATION: Duration = Duration::from_millis(20);

/// Ten 20 ms frames give each local audio queue a 200 ms ceiling.
pub const AUDIO_QUEUE_CAPACITY: NonZeroUsize = NonZeroUsize::MIN.saturating_add(9);
