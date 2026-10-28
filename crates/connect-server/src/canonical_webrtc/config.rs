//! Explicit canonical WebRTC composition, separate from WSS control.
use connect_protocol::native_network::NativeNetworkProfile;
use rvoip_core::capability::CodecInfo;
use rvoip_webrtc::WebRtcConfig;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CanonicalWebRtcProfile {
    Opus,
    OpusVp8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalWebRtcConfig {
    pub profile: CanonicalWebRtcProfile,
    pub max_peers: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addressing: Option<super::CanonicalWebRtcAddressing>,
}
impl CanonicalWebRtcConfig {
    pub(crate) fn validate(
        &self,
        network: NativeNetworkProfile,
        websocket_configured: bool,
        connection_limit: usize,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            websocket_configured,
            "Canonical WebRTC requires configured signed WSS control"
        );
        anyhow::ensure!(
            network == NativeNetworkProfile::Local || self.addressing.is_some(),
            "Remote WebRTC requires explicit media addressing"
        );
        anyhow::ensure!(
            (1..=512).contains(&self.max_peers) && self.max_peers <= connection_limit,
            "Canonical WebRTC max_peers must be 1..=512 and within the connection limit"
        );
        if let Some(addressing) = &self.addressing {
            addressing.validate(network, self.max_peers)?;
        }
        Ok(())
    }
    pub(crate) fn host(
        &self,
        network: NativeNetworkProfile,
    ) -> connect_protocol::Result<Arc<super::SignalingHost>> {
        self.validate(network, true, self.max_peers).map_err(|_| {
            connect_protocol::Error::new("invalid-request", "Invalid WebRTC configuration")
        })?;
        let mut config = match network {
            NativeNetworkProfile::Local => WebRtcConfig::loopback(),
            NativeNetworkProfile::Remote => WebRtcConfig::default(),
        };
        // Never inherit the upstream default public STUN service.
        config.ice_servers.clear();
        if let Some(addressing) = &self.addressing {
            addressing
                .apply(network, self.max_peers, &mut config, |name| {
                    crate::secrets::resolve(name).map(|value| value.expose().to_owned())
                })
                .map_err(|_| {
                    connect_protocol::Error::new(
                        "not-configured",
                        "WebRTC addressing or TURN credentials unavailable",
                    )
                })?;
        }
        config.capabilities.audio_codecs = vec![CodecInfo::from_name_with_defaults("opus")];
        config.capabilities.video_codecs = match self.profile {
            CanonicalWebRtcProfile::Opus => vec![],
            CanonicalWebRtcProfile::OpusVp8 => vec![CodecInfo::from_name_with_defaults("vp8")],
        };
        config.require_vp8_audio_video_profile = self.profile == CanonicalWebRtcProfile::OpusVp8;
        config.opus_settings.max_average_bitrate_bps = 32000;
        super::SignalingHost::new(config, self.max_peers)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_local_profiles_build_bounded_hosts() {
        for profile in [
            CanonicalWebRtcProfile::Opus,
            CanonicalWebRtcProfile::OpusVp8,
        ] {
            let config = CanonicalWebRtcConfig {
                profile,
                max_peers: 2,
                addressing: None,
            };
            config
                .validate(NativeNetworkProfile::Local, true, 8)
                .unwrap();
            assert_eq!(
                config
                    .host(NativeNetworkProfile::Local)
                    .unwrap()
                    .available_slots(),
                2
            );
        }
    }
    #[test]
    fn unsupported_network_control_and_capacity_refuse() {
        let mut config = CanonicalWebRtcConfig {
            profile: CanonicalWebRtcProfile::OpusVp8,
            max_peers: 2,
            addressing: None,
        };
        assert!(config
            .validate(NativeNetworkProfile::Remote, true, 8)
            .is_err());
        assert!(config
            .validate(NativeNetworkProfile::Local, false, 8)
            .is_err());
        for capacity in [0, 9, 513] {
            config.max_peers = capacity;
            assert!(config
                .validate(NativeNetworkProfile::Local, true, 8)
                .is_err());
        }
    }
    #[test]
    fn strict_configuration_roundtrips_and_refuses_unknown_fields_profiles() {
        let config: CanonicalWebRtcConfig =
            toml::from_str("profile = 'opus-vp8'\nmax_peers = 2\n").unwrap();
        let saved = toml::to_string(&config).unwrap();
        let restored: CanonicalWebRtcConfig = toml::from_str(&saved).unwrap();
        assert_eq!(restored.profile, config.profile);
        assert_eq!(restored.max_peers, 2);
        assert!(
            toml::from_str::<CanonicalWebRtcConfig>("profile = 'h264'\nmax_peers = 2\n").is_err()
        );
        assert!(toml::from_str::<CanonicalWebRtcConfig>(
            "profile = 'opus'\nmax_peers = 2\nunknown = true\n"
        )
        .is_err());
    }

    #[test]
    fn remote_host_requires_explicit_bounded_addressing_and_roundtrips_without_secrets() {
        let config = CanonicalWebRtcConfig {
            profile: CanonicalWebRtcProfile::OpusVp8,
            max_peers: 2,
            addressing: Some(super::super::CanonicalWebRtcAddressing {
                bind_ip: "0.0.0.0".parse().unwrap(),
                port_start: 41000,
                port_end: 41001,
                public_ip: Some("203.0.113.44".parse().unwrap()),
                ice_policy: super::super::CanonicalIcePolicy::All,
                ice_services: Vec::new(),
            }),
        };
        config
            .validate(NativeNetworkProfile::Remote, true, 4)
            .unwrap();
        assert_eq!(
            config
                .host(NativeNetworkProfile::Remote)
                .unwrap()
                .available_slots(),
            2
        );
        assert!(config
            .validate(NativeNetworkProfile::Local, true, 4)
            .is_err());
        assert!(config
            .validate(NativeNetworkProfile::Remote, false, 4)
            .is_err());
        let saved = toml::to_string(&config).unwrap();
        let restored: CanonicalWebRtcConfig = toml::from_str(&saved).unwrap();
        restored
            .validate(NativeNetworkProfile::Remote, true, 4)
            .unwrap();
        assert_eq!(restored.addressing.unwrap().port_end, 41001);
    }
}
