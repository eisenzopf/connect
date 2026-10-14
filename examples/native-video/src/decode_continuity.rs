//! Keep dependent VP8 interframes out of the decoder after a reference gap.
use connect_client::VideoSequenceRange;

#[derive(Default)]
pub struct DecodeContinuity {
    next: Option<(u32, u16)>,
}
impl DecodeContinuity {
    pub fn accept(&mut self, keyframe: bool, range: VideoSequenceRange) -> bool {
        if keyframe || self.next == Some((range.ssrc, range.first)) {
            self.next = Some((range.ssrc, range.last.wrapping_add(1)));
            true
        } else {
            self.next = None;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn range(ssrc: u32, first: u16, last: u16) -> VideoSequenceRange {
        VideoSequenceRange { ssrc, first, last }
    }
    #[test]
    fn startup_and_missing_reference_require_a_new_keyframe() {
        let mut guard = DecodeContinuity::default();
        assert!(!guard.accept(false, range(1, 0, 4)));
        assert!(guard.accept(true, range(1, 5, 10)));
        assert!(guard.accept(false, range(1, 11, 12)));
        assert!(!guard.accept(false, range(1, 15, 18)));
        assert!(!guard.accept(false, range(1, 19, 20)));
        assert!(guard.accept(true, range(1, 21, 30)));
        assert!(guard.accept(false, range(1, 31, 32)));
    }
    #[test]
    fn sequence_wrap_is_contiguous_but_source_change_is_not() {
        let mut guard = DecodeContinuity::default();
        assert!(guard.accept(true, range(1, 65530, u16::MAX)));
        assert!(guard.accept(false, range(1, 0, 4)));
        assert!(!guard.accept(false, range(2, 5, 6)));
        assert!(guard.accept(true, range(2, 7, 12)));
    }
}
