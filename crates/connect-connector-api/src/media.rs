//! Media declarations describe a host installation, never authority or readiness.
use crate::manifest::{AudioFormat, ContractError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MediaSupport {
    Unsupported,
    /// Bounded decoded PCM exercise; this does not install a network adapter.
    LifecycleFixture {
        audio: AudioFormat,
    },
    Live {
        binding: LiveBinding,
        codecs: LiveCodecs,
        controls: BTreeSet<LiveControl>,
        context: ContextHandling,
        records: RecordHandling,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LiveBinding {
    /// Exact rvoip Transport serialization, checked against the supplied adapter.
    Transport { transport: String },
    /// A separately enrolled native Endpoint and owned public SDK media host.
    SignedNativeEndpoint,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LiveCodecs {
    /// No static codec promise. The current negotiated Connection is required.
    Negotiated,
    Advertised {
        audio: Vec<AudioCodec>,
        video: Vec<VideoCodec>,
        data: Vec<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioCodec {
    pub name: String,
    pub clock_rate_hz: u32,
    pub channels: u8,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoCodec {
    pub name: String,
    pub clock_rate_hz: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveControl {
    End,
    InboundAdmission,
    StagedOriginate,
    DeviceReplacement,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextHandling {
    Unsupported,
    /// Explicit bounded historical sources selected by current host policy.
    HostSelected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordHandling {
    Unsupported,
    /// The host captures selected streams under current content/audience policy.
    HostAuthorizedStreams,
}

/// Trusted composition's actual adapter/hook observation. Not a wire admission.
/// The host must derive this from its installed implementation, not client JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRuntime {
    pub binding: LiveBinding,
    pub codecs: LiveCodecs,
    pub authoritative_liveness: bool,
    pub atomic_inbound_handoff: bool,
    pub terminal_fallback: bool,
    pub staged_outbound_activation: bool,
    pub device_replacement: bool,
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
}
impl LiveCodecs {
    pub fn validate(&self) -> Result<(), ContractError> {
        let Self::Advertised { audio, video, data } = self else {
            return Ok(());
        };
        if audio.len() > 16
            || video.len() > 16
            || data.len() > 16
            || audio.len() + video.len() + data.len() == 0
            || audio.iter().any(|a| {
                !token(&a.name)
                    || !(1..=192_000).contains(&a.clock_rate_hz)
                    || !(1..=8).contains(&a.channels)
            })
            || video
                .iter()
                .any(|v| !token(&v.name) || v.clock_rate_hz != 90_000)
            || data.iter().any(|v| !token(v))
            || audio.iter().collect::<BTreeSet<_>>().len() != audio.len()
            || video.iter().collect::<BTreeSet<_>>().len() != video.len()
            || data.iter().collect::<BTreeSet<_>>().len() != data.len()
        {
            return Err(ContractError::Manifest);
        }
        Ok(())
    }
    /// A claim is a subset of the actual runtime; negotiated-only data cannot
    /// satisfy a static codec advertisement. Packetization is negotiated later.
    pub fn supported_by(&self, runtime: &Self) -> bool {
        match (self, runtime) {
            (Self::Negotiated, _) => true,
            (
                Self::Advertised { audio, video, data },
                Self::Advertised {
                    audio: a,
                    video: v,
                    data: d,
                },
            ) => {
                audio.iter().all(|x| a.contains(x))
                    && video.iter().all(|x| v.contains(x))
                    && data.iter().all(|x| d.contains(x))
            }
            _ => false,
        }
    }
}
impl MediaSupport {
    pub fn fixture_audio(&self) -> Option<&AudioFormat> {
        match self {
            Self::LifecycleFixture { audio } => Some(audio),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<(), ContractError> {
        match self {
            Self::Unsupported => Ok(()),
            Self::LifecycleFixture { audio } => {
                if audio.codec != "pcm-s16le"
                    || !matches!(audio.sample_rate, 8000 | 16000 | 24000 | 48000)
                    || audio.channels != 1
                    || audio.frame_ms != 20
                {
                    return Err(ContractError::Manifest);
                }
                Ok(())
            }
            Self::Live {
                binding,
                codecs,
                controls,
                ..
            } => {
                if matches!(binding,LiveBinding::Transport{transport} if !token(transport))
                    || !controls.contains(&LiveControl::End)
                {
                    return Err(ContractError::Manifest);
                }
                codecs.validate()
            }
        }
    }
    pub fn require_runtime(&self, runtime: &MediaRuntime) -> Result<(), ContractError> {
        self.validate()?;
        runtime.codecs.validate()?;
        let Self::Live {
            binding,
            codecs,
            controls,
            ..
        } = self
        else {
            return Err(ContractError::UnsupportedMedia);
        };
        if binding != &runtime.binding
            || !runtime.authoritative_liveness
            || !runtime.terminal_fallback
            || !codecs.supported_by(&runtime.codecs)
            || (controls.contains(&LiveControl::InboundAdmission)
                && !runtime.atomic_inbound_handoff)
            || (controls.contains(&LiveControl::StagedOriginate)
                && !runtime.staged_outbound_activation)
            || (controls.contains(&LiveControl::DeviceReplacement)
                && (!runtime.device_replacement || !runtime.staged_outbound_activation))
        {
            return Err(ContractError::MediaRuntime);
        }
        Ok(())
    }
    /// Shape/capability check only. Successful negotiation and current host
    /// authorization are separate requirements for any packet or control effect.
    pub fn require_codecs(&self, requested: &LiveCodecs) -> Result<(), ContractError> {
        requested.validate()?;
        let Self::Live { codecs, .. } = self else {
            return Err(ContractError::UnsupportedMedia);
        };
        if matches!(codecs, LiveCodecs::Negotiated) || matches!(requested, LiveCodecs::Negotiated) {
            return Err(ContractError::MediaNegotiation);
        }
        if !requested.supported_by(codecs) {
            return Err(ContractError::UnsupportedMedia);
        }
        Ok(())
    }
}
