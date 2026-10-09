use anyhow::{bail, Context, Result};
use connect_protocol::Limits;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Complete local server configuration.
pub struct ServerConfig {
    /// Native UCTP/QUIC listener.
    pub bind: SocketAddr,
    /// Optional loopback WebSocket control-parity listener.
    pub websocket_bind: Option<SocketAddr>,
    /// SQLite state file.
    pub database: PathBuf,
    /// DER certificate presented by the native listener.
    pub certificate: PathBuf,
    /// DER private key for the native listener.
    pub private_key: PathBuf,
    /// Optional SIP/RTP listener and its explicit durable route.
    #[serde(default)]
    pub sip: Option<SipMediaConfig>,
    /// Optional authenticated browser WebRTC listener and durable route.
    #[serde(default)]
    pub webrtc: Option<WebRtcMediaConfig>,
    /// Bounded application and transport limits.
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Administrator-selected authority for an unauthenticated media address.
pub struct InboundMediaRoute {
    /// Credential revalidated for each admitted leg.
    pub credential_id: String,
    /// Existing durable conversation that owns every created Session.
    pub conversation_id: String,
    /// Existing assistant identity with one enabled voice route.
    pub assistant_identity_id: String,
    /// Optional accepted Session to join instead of creating a new routed
    /// assistant Session. This is used for explicit cross-transport pairing.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// SIP signaling and RTP listener configuration.
pub struct SipMediaConfig {
    /// Local SIP UDP bind address. Port zero is rejected.
    pub bind: SocketAddr,
    /// Concrete local interface used for RTP allocation. Required when the
    /// signaling listener binds a wildcard address.
    #[serde(default)]
    pub media_bind_ip: Option<IpAddr>,
    /// Optional public SIP address advertised to peers.
    pub advertised: Option<SocketAddr>,
    /// Optional public media address used in SDP.
    pub media_public: Option<SocketAddr>,
    /// Optional outbound carrier trunk. The password is read from the named
    /// environment variable when the server starts and is never serialized.
    #[serde(default)]
    pub outbound: Option<SipOutboundConfig>,
    /// Explicit durable authority for inbound dialogs.
    pub route: InboundMediaRoute,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Operator-owned settings for outbound SIP carrier access.
pub struct SipOutboundConfig {
    /// Optional registrar for receiving calls on this credential trunk.
    /// rvoip refreshes the accepted registration and unregisters on shutdown.
    #[serde(default)]
    pub registrar_uri: Option<String>,
    /// Loose-route proxy used for every originated dialog.
    pub proxy_uri: String,
    /// SIP identity used in From and P-Asserted-Identity headers.
    pub identity_uri: String,
    /// Digest authentication username supplied by the carrier.
    pub auth_username: String,
    /// Name of the environment variable containing the Digest password.
    pub auth_password_env: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Authenticated WebRTC WebSocket signaling configuration.
pub struct WebRtcMediaConfig {
    /// Loopback signaling listener.
    pub bind: SocketAddr,
    /// Explicit durable authority for authenticated inbound peers.
    pub route: InboundMediaRoute,
}
impl ServerConfig {
    /// Load, validate, and resolve relative paths against the config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).context("Read server configuration")?;
        let mut config: Self =
            toml::from_str(&text).map_err(|_| anyhow::anyhow!("Invalid server configuration"))?;
        config.validate()?;
        let parent = path.parent().unwrap_or(Path::new("."));
        for field in [
            &mut config.database,
            &mut config.certificate,
            &mut config.private_key,
        ] {
            if field.is_relative() {
                *field = parent.join(&*field);
            }
        }
        Ok(config)
    }
    /// Validate the bounded local profile without opening listeners or files.
    pub fn validate(&self) -> Result<()> {
        if !self.bind.ip().is_loopback()
            || self
                .websocket_bind
                .is_some_and(|addr| !addr.ip().is_loopback())
            || self
                .webrtc
                .as_ref()
                .is_some_and(|value| !value.bind.ip().is_loopback())
        {
            bail!("The local profile requires loopback listener addresses")
        }
        if let Some(sip) = &self.sip {
            if sip.bind.port() == 0
                || (sip.bind.ip().is_unspecified()
                    && (sip
                        .advertised
                        .is_none_or(|value| value.ip().is_unspecified())
                        || sip.media_bind_ip.is_none_or(|value| value.is_unspecified())))
                || sip
                    .media_bind_ip
                    .is_some_and(|value| value.is_unspecified())
                || sip
                    .advertised
                    .is_some_and(|value| value.ip().is_unspecified())
                || sip
                    .media_public
                    .is_some_and(|value| value.ip().is_unspecified())
            {
                bail!("SIP bind and advertised addresses are invalid")
            }
            if let Some(outbound) = &sip.outbound {
                outbound.validate()?;
            }
        }
        for route in [
            self.sip.as_ref().map(|value| &value.route),
            self.webrtc.as_ref().map(|value| &value.route),
        ]
        .into_iter()
        .flatten()
        {
            if route.credential_id.is_empty()
                || route.conversation_id.is_empty()
                || route.assistant_identity_id.is_empty()
                || route.session_id.as_ref().is_some_and(String::is_empty)
            {
                bail!("Inbound media routes require explicit durable authority")
            }
        }
        let l = &self.limits;
        if l.message_bytes == 0
            || l.message_bytes > 64 * 1024
            || l.page_size == 0
            || l.page_size > 1000
            || l.participants == 0
            || l.participants > 128
            || l.journal_events == 0
            || l.operation_receipts == 0
            || l.message_storage_bytes == 0
            || l.wait_ms > 5000
            || l.request_timeout_ms == 0
            || l.request_timeout_ms > 60_000
            || l.connections == 0
            || l.connections > 1024
            || l.sessions == 0
            || l.artifact_file_bytes == 0
            || l.artifact_file_bytes > 64 * 1024 * 1024
            || l.artifact_bytes < l.artifact_file_bytes
            || l.artifact_bytes > 4 * 1024 * 1024 * 1024
            || l.artifact_chunk_bytes == 0
            || l.artifact_chunk_bytes > 1024 * 1024
        {
            bail!("Local limits are invalid or exceed the bounded profile")
        }
        Ok(())
    }
}

impl SipOutboundConfig {
    fn validate(&self) -> Result<()> {
        if !valid_sip_uri(&self.proxy_uri)
            || !valid_sip_uri(&self.identity_uri)
            || self
                .registrar_uri
                .as_ref()
                .is_some_and(|uri| !valid_sip_uri(uri))
            || self.auth_username.is_empty()
            || self.auth_username.len() > 256
            || !self
                .auth_username
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-._~".contains(character))
            || !valid_environment_name(&self.auth_password_env)
        {
            bail!("Outbound SIP trunk configuration is invalid")
        }
        Ok(())
    }
}

fn valid_sip_uri(value: &str) -> bool {
    value.len() <= 2048
        && (value.starts_with("sip:") || value.starts_with("sips:"))
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn valid_environment_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::{
        valid_environment_name, valid_sip_uri, InboundMediaRoute, ServerConfig, SipMediaConfig,
    };
    use connect_protocol::Limits;

    #[test]
    fn outbound_sip_fields_reject_ambiguous_input() {
        assert!(valid_sip_uri("sip:sip.telnyx.com;lr"));
        assert!(valid_sip_uri("sips:carrier.example:5061;lr"));
        assert!(!valid_sip_uri("https://carrier.example"));
        assert!(!valid_sip_uri("sip:carrier.example\nInjected: value"));

        assert!(valid_environment_name("CONNECT_SIP_PASSWORD"));
        assert!(!valid_environment_name("1PASSWORD"));
        assert!(!valid_environment_name("PASSWORD-NAME"));
    }

    #[test]
    fn wildcard_sip_requires_a_concrete_media_bind_interface() {
        let mut config = ServerConfig {
            bind: "127.0.0.1:4433".parse().expect("native address"),
            websocket_bind: None,
            database: "state.sqlite3".into(),
            certificate: "certificate.der".into(),
            private_key: "private-key.der".into(),
            sip: Some(SipMediaConfig {
                bind: "0.0.0.0:5060".parse().expect("SIP address"),
                media_bind_ip: None,
                advertised: Some("203.0.113.8:5060".parse().expect("advertised address")),
                media_public: Some("203.0.113.8:0".parse().expect("media address")),
                outbound: None,
                route: InboundMediaRoute {
                    credential_id: "credential".into(),
                    conversation_id: "conversation".into(),
                    assistant_identity_id: "assistant".into(),
                    session_id: None,
                },
            }),
            webrtc: None,
            limits: Limits::default(),
        };
        assert!(config.validate().is_err());

        config.sip.as_mut().expect("SIP config").media_bind_ip =
            Some("192.168.1.104".parse().expect("media bind address"));
        config.validate().expect("explicit media bind is valid");
    }
}
