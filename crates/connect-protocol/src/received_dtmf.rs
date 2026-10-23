//! Actual received SIP facts. This is not a signed native observation or a grant.
use crate::{transitions::id, Error, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    #[serde(rename = "sip-info")]
    SipInfo,
    #[serde(rename = "rfc4733")]
    SipRfc4733,
}
impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SipInfo => "sip-info",
            Self::SipRfc4733 => "rfc4733",
        }
    }
    pub fn method(self) -> &'static str {
        match self {
            Self::SipInfo => "info",
            Self::SipRfc4733 => "rfc4733",
        }
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RtpObservation {
    pub ssrc: u32,
    pub payload_type: u8,
    pub clock_rate_hz: u32,
    pub rtp_timestamp: u32,
    pub extended_timestamp: u64,
    pub last_segment_timestamp: u32,
    pub duration_samples: u64,
    pub event: u8,
    pub start_observed: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceObservation {
    pub kind: SourceKind,
    pub observation_id: String,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub digits: String,
    pub duration_ms: u32,
    pub rtp: Option<RtpObservation>,
}
impl SourceObservation {
    pub fn validate(&self) -> Result<()> {
        let invalid = || Error::new("invalid-request", "Invalid received DTMF observation");
        id(&self.observation_id)?;
        if self.digits.len() != 1
            || !self
                .digits
                .bytes()
                .all(|b| b.is_ascii_digit() || b"*#ABCDabcd".contains(&b))
        {
            return Err(invalid());
        }
        match (self.kind, &self.rtp) {
            (SourceKind::SipInfo, None) if (100..=5000).contains(&self.duration_ms) => Ok(()),
            (SourceKind::SipRfc4733, Some(rtp)) => {
                let event = match self.digits.as_bytes()[0].to_ascii_uppercase() {
                    b'0'..=b'9' => self.digits.as_bytes()[0] - b'0',
                    b'*' => 10,
                    b'#' => 11,
                    b'A'..=b'D' => self.digits.as_bytes()[0].to_ascii_uppercase() - b'A' + 12,
                    _ => return Err(invalid()),
                };
                let scaled = rtp.duration_samples.checked_mul(1000).ok_or_else(invalid)?;
                if rtp.payload_type > 127
                    || rtp.clock_rate_hz == 0
                    || rtp.clock_rate_hz > 192000
                    || rtp.duration_samples > 9_007_199_254_740_991
                    || rtp.extended_timestamp > 9_007_199_254_740_991
                    || rtp.event != event
                    || scaled.div_ceil(u64::from(rtp.clock_rate_hz)) != u64::from(self.duration_ms)
                {
                    return Err(invalid());
                }
                Ok(())
            }
            _ => Err(invalid()),
        }
    }
    /// Incomplete/fractional RTP remains an observation; it cannot mint a
    /// clamped/inferred outbound tone or native duration.
    pub fn forwardable(&self) -> bool {
        self.validate().is_ok()
            && match self.kind {
                SourceKind::SipInfo => true,
                SourceKind::SipRfc4733 => {
                    (40..=6000).contains(&self.duration_ms)
                        && self.rtp.as_ref().is_some_and(|r| {
                            r.start_observed
                                && r.duration_samples * 1000 % u64::from(r.clock_rate_hz) == 0
                        })
                }
            }
    }
    pub fn same_physical_input(&self, other: &Self) -> bool {
        let mut copy = other.clone();
        copy.received_at = self.received_at;
        self == &copy
    }
}
impl std::fmt::Debug for SourceObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReceivedDtmfObservation(redacted)")
    }
}
impl std::fmt::Debug for RtpObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReceivedDtmfRtpObservation(redacted)")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rtp() -> SourceObservation {
        SourceObservation {
            kind: SourceKind::SipRfc4733,
            observation_id: "actual-packet".into(),
            received_at: chrono::Utc::now(),
            digits: "5".into(),
            duration_ms: 237,
            rtp: Some(RtpObservation {
                ssrc: 1,
                payload_type: 101,
                clock_rate_hz: 8000,
                rtp_timestamp: 3,
                extended_timestamp: 3,
                last_segment_timestamp: 3,
                duration_samples: 1896,
                event: 5,
                start_observed: true,
            }),
        }
    }
    #[test]
    fn received_dtmf_preserves_actual_provenance_and_integral_timing_without_native_signing_fields()
    {
        let r = rtp();
        r.validate().unwrap();
        assert!(r.forwardable());
        let wire = serde_json::to_value(&r).unwrap();
        assert_eq!(
            serde_json::from_value::<SourceObservation>(wire.clone()).unwrap(),
            r
        );
        for key in [
            "envelope_id",
            "signing_key_id",
            "signing_algorithm",
            "stream_revision",
            "credential_revision",
        ] {
            let mut forged = wire.clone();
            forged[key] = serde_json::json!("native");
            assert!(serde_json::from_value::<SourceObservation>(forged).is_err());
        }
        let mut other = r.clone();
        other.received_at += chrono::Duration::seconds(1);
        assert!(r.same_physical_input(&other));
        other.duration_ms += 1;
        assert!(!r.same_physical_input(&other));
    }
    #[test]
    fn received_dtmf_incomplete_fractional_and_out_of_bounds_tones_never_synthesize_effects() {
        let mut r = rtp();
        r.rtp.as_mut().unwrap().start_observed = false;
        r.validate().unwrap();
        assert!(!r.forwardable());
        let mut r = rtp();
        r.rtp.as_mut().unwrap().duration_samples += 1;
        r.duration_ms = 238; // Preserve the actual receiver's ceiling summary.
        r.validate().unwrap();
        assert!(!r.forwardable());
        let mut r = rtp();
        r.duration_ms = 6001;
        r.rtp.as_mut().unwrap().duration_samples = 48008;
        r.validate().unwrap();
        assert!(!r.forwardable());
        for mutation in 0..5 {
            let mut r = rtp();
            match mutation {
                0 => r.rtp = None,
                1 => r.kind = SourceKind::SipInfo,
                2 => r.rtp.as_mut().unwrap().clock_rate_hz = 0,
                3 => r.rtp.as_mut().unwrap().event = 7,
                _ => r.rtp.as_mut().unwrap().duration_samples = u64::MAX,
            };
            assert!(r.validate().is_err());
            assert!(!r.forwardable());
        }
    }
}
