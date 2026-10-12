//! Composition and bounded shutdown for optional SIP and WebRTC listeners.

use crate::config::{ServerConfig, SipOutboundConfig};
use anyhow::{Context, Result};
use connect_service::StoreValidator;
use connect_store::Store;
use rvoip_core::{adapter::ConnectionAdapter, Orchestrator};
use rvoip_sip::{
    api::unified::Config as SipConfig, types::Credentials, SipAdapter, UnifiedCoordinator,
};
use rvoip_webrtc::{WebRtcServer, WebRtcServerBuilder};
use std::{net::SocketAddr, sync::Arc, time::Duration};

pub(crate) struct MediaTransports {
    sip: Option<Arc<UnifiedCoordinator>>,
    registration: Option<crate::registration_monitor::Monitor>,
    webrtc: Option<WebRtcServer>,
    pub(crate) sip_address: Option<SocketAddr>,
    pub(crate) webrtc_address: Option<SocketAddr>,
}

impl MediaTransports {
    pub(crate) async fn start(
        config: &ServerConfig,
        orchestrator: &Arc<Orchestrator>,
        store: Arc<Store>,
        readiness: Arc<connect_service::readiness::OperationalReadiness>,
    ) -> Result<Self> {
        let (sip, sip_address, registration) =
            start_sip(config, orchestrator, readiness.clone()).await?;
        let (webrtc, webrtc_address) = start_webrtc(config, orchestrator, store).await?;
        Ok(Self {
            sip,
            registration,
            webrtc,
            sip_address,
            webrtc_address,
        })
    }

    pub(crate) async fn shutdown(&mut self, failures: &mut Vec<String>) {
        if let Some(monitor) = self.registration.take() {
            if let Err(error) = monitor.shutdown().await {
                failures.push(error.to_string());
            }
        }
        if let Some(server) = self.webrtc.take() {
            server.shutdown_with_deadline(Duration::from_secs(5)).await;
        }
        if let Some(coordinator) = self.sip.take() {
            if let Err(error) = coordinator
                .shutdown_gracefully(Some(Duration::from_secs(5)))
                .await
            {
                failures.push(error.to_string());
            }
        }
    }
}

async fn start_sip(
    config: &ServerConfig,
    orchestrator: &Arc<Orchestrator>,
    readiness: Arc<connect_service::readiness::OperationalReadiness>,
) -> Result<(
    Option<Arc<UnifiedCoordinator>>,
    Option<SocketAddr>,
    Option<crate::registration_monitor::Monitor>,
)> {
    let Some(sip) = &config.sip else {
        return Ok((None, None, None));
    };
    let mut listener = SipConfig::local("connect", sip.bind.port());
    listener.bind_addr = sip.bind;
    listener.sip_port = sip.bind.port();
    listener.local_ip = sip.media_bind_ip.unwrap_or(sip.bind.ip());
    listener.local_uri = format!("sip:connect@{}:{}", listener.local_ip, sip.bind.port());
    listener.sip_advertised_addr = sip.advertised;
    if let Some(outbound) = &sip.outbound {
        apply_outbound_trunk(&mut listener, outbound, |name| std::env::var(name))?;
    }
    if let Some(address) = sip.media_public {
        listener = listener.with_media_public_addr(address);
    }
    let coordinator = UnifiedCoordinator::new(listener)
        .await
        .context("Cannot bind configured SIP listener")?;
    let registration = if let Some(outbound) = &sip.outbound {
        match crate::sip_registration::register(&coordinator, outbound).await {
            Ok(Some(handle)) => Some(crate::registration_monitor::Monitor::start(
                coordinator.clone(),
                handle,
                readiness.clone(),
            )),
            Ok(None) => {
                readiness.set(
                    "sip-registration",
                    connect_protocol::ReadinessState::Uncertain,
                    "registration-not-configured",
                );
                None
            }
            Err(error) => {
                let _ = coordinator
                    .shutdown_gracefully(Some(Duration::from_secs(5)))
                    .await;
                return Err(error);
            }
        }
    } else {
        None
    };
    let adapter = SipAdapter::new_with_inbound_context_policy_and_queue_capacity(
        coordinator.clone(),
        Default::default(),
        connect_protocol::media_profile::AUDIO_QUEUE_CAPACITY,
    )
    .await
    .context("Cannot create SIP media adapter")?;
    orchestrator.register(adapter as Arc<dyn ConnectionAdapter>)?;
    Ok((Some(coordinator), Some(sip.bind), registration))
}

fn apply_outbound_trunk(
    listener: &mut SipConfig,
    outbound: &SipOutboundConfig,
    read_secret: impl FnOnce(&str) -> std::result::Result<String, std::env::VarError>,
) -> Result<()> {
    let password = read_secret(&outbound.auth_password_env).with_context(|| {
        format!(
            "Outbound SIP password environment variable {} is unavailable",
            outbound.auth_password_env
        )
    })?;
    if password.is_empty() {
        anyhow::bail!(
            "Outbound SIP password environment variable {} is empty",
            outbound.auth_password_env
        );
    }
    listener.local_uri = outbound.identity_uri.clone();
    listener.pai_uri = Some(outbound.identity_uri.clone());
    listener.outbound_proxy_uri = Some(outbound.proxy_uri.clone());
    listener.credentials = Some(Credentials::new(outbound.auth_username.clone(), password));
    let contact = listener.sip_advertised_addr.unwrap_or(listener.bind_addr);
    listener.contact_uri = Some(format!("sip:{}@{contact}", outbound.auth_username));
    Ok(())
}

async fn start_webrtc(
    config: &ServerConfig,
    orchestrator: &Arc<Orchestrator>,
    store: Arc<Store>,
) -> Result<(Option<WebRtcServer>, Option<SocketAddr>)> {
    let Some(webrtc) = &config.webrtc else {
        return Ok((None, None));
    };
    let auth = rvoip_webrtc::signaling::auth::AuthCoreHook::new(StoreValidator::for_credential(
        store,
        webrtc.route.credential_id.clone(),
    ))
    .with_realm("connect");
    let server = WebRtcServerBuilder::new(crate::media_profile::webrtc_config())
        .with_ws(webrtc.bind.to_string())
        .with_ws_auth(Arc::new(auth))
        .build()
        .await
        .context("Cannot bind configured WebRTC signaling listener")?;
    let address = server
        .ws_addr()
        .context("WebRTC signaling listener did not report its address")?;
    orchestrator.register(server.adapter() as Arc<dyn ConnectionAdapter>)?;
    Ok((Some(server), Some(address)))
}

#[cfg(test)]
mod tests {
    use super::{apply_outbound_trunk, SipConfig};
    use crate::config::SipOutboundConfig;

    fn trunk() -> SipOutboundConfig {
        SipOutboundConfig {
            registrar_uri: None,
            proxy_uri: "sip:sip.telnyx.com;lr".into(),
            identity_uri: "sip:+12702628454@sip.telnyx.com".into(),
            auth_username: "connect-user".into(),
            auth_password_env: "CONNECT_SIP_PASSWORD".into(),
        }
    }

    #[test]
    fn outbound_trunk_uses_environment_secret_without_logging_it() {
        let mut listener = SipConfig::local("connect", 5060);
        apply_outbound_trunk(&mut listener, &trunk(), |_| Ok("private-password".into()))
            .expect("valid trunk should be applied");

        assert_eq!(listener.local_uri, "sip:+12702628454@sip.telnyx.com");
        assert_eq!(
            listener.contact_uri.as_deref(),
            Some("sip:connect-user@127.0.0.1:5060")
        );
        assert_eq!(
            listener.pai_uri.as_deref(),
            Some(listener.local_uri.as_str())
        );
        assert_eq!(
            listener.outbound_proxy_uri.as_deref(),
            Some("sip:sip.telnyx.com;lr")
        );
        let debug = format!(
            "{:?}",
            listener
                .credentials
                .expect("valid trunk should install credentials")
        );
        assert!(!debug.contains("private-password"));
    }

    #[test]
    fn outbound_trunk_fails_closed_without_a_password() {
        let mut listener = SipConfig::local("connect", 5060);
        let missing = apply_outbound_trunk(&mut listener, &trunk(), |_| {
            Err(std::env::VarError::NotPresent)
        })
        .expect_err("missing secret should fail closed");
        assert!(missing.to_string().contains("CONNECT_SIP_PASSWORD"));

        let empty = apply_outbound_trunk(&mut listener, &trunk(), |_| Ok(String::new()))
            .expect_err("empty secret should fail closed");
        assert!(empty.to_string().contains("empty"));
    }
}
