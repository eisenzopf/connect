//! Optional carrier registration using rvoip's owned refresh/shutdown lifecycle.

use crate::config::SipOutboundConfig;
use anyhow::{bail, Context, Result};
use rvoip_sip::{RegistrationStatus, UnifiedCoordinator};
use std::{sync::Arc, time::Duration};

/// Startup is ready only after the registrar accepts the binding. Raw carrier
/// errors can contain credentials and URIs, so expose only a stable class.
pub(crate) async fn register(
    coordinator: &Arc<UnifiedCoordinator>,
    trunk: &SipOutboundConfig,
) -> Result<()> {
    let Some(registrar) = &trunk.registrar_uri else {
        return Ok(());
    };
    let password = std::env::var(&trunk.auth_password_env)
        .context("SIP registration password is unavailable")?;
    register_with_password(coordinator, trunk, registrar, password).await
}

async fn register_with_password(
    coordinator: &Arc<UnifiedCoordinator>,
    trunk: &SipOutboundConfig,
    registrar: &str,
    password: String,
) -> Result<()> {
    let (scheme, authority) = registrar
        .split_once(':')
        .context("SIP registrar URI is invalid")?;
    tokio::time::timeout(Duration::from_secs(10), async {
        let handle = coordinator
            .register(registrar, &trunk.auth_username, password)
            .with_from_uri(format!("{scheme}:{}@{authority}", trunk.auth_username))
            .with_expires(300)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("SIP registration request failed"))?;
        loop {
            let info = coordinator
                .registration_info(&handle)
                .await
                .map_err(|_| anyhow::anyhow!("SIP registration state unavailable"))?;
            match info.status {
                RegistrationStatus::Registered => {
                    tracing::info!("SIP trunk registration accepted");
                    return Ok(());
                }
                RegistrationStatus::Failed | RegistrationStatus::Unregistered => {
                    bail!("SIP trunk registration rejected");
                }
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    })
    .await
    .context("SIP trunk registration timed out")?
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // Isolated SIP fixture assertions deliberately panic.
mod tests {
    use super::*;
    use rvoip_sip::Config;
    use tokio::net::UdpSocket;

    fn header<'a>(request: &'a str, name: &str) -> &'a str {
        request
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case(name).then_some(value.trim())
            })
            .unwrap()
    }

    async fn registrar(socket: UdpSocket, reject: bool) -> Vec<String> {
        let mut requests = Vec::new();
        let mut bytes = [0; 8192];
        loop {
            let (length, peer) = socket.recv_from(&mut bytes).await.unwrap();
            let request = String::from_utf8(bytes[..length].to_vec()).unwrap();
            let challenge = requests.is_empty() && !reject;
            let unregister = header(&request, "Expires") == "0";
            let status = if reject {
                "403 Forbidden"
            } else if challenge {
                "401 Unauthorized"
            } else {
                "200 OK"
            };
            let extra = if challenge {
                "WWW-Authenticate: Digest realm=\"local-carrier\", nonce=\"qualification-nonce\", algorithm=MD5, qop=\"auth\"\r\n".to_string()
            } else {
                format!(
                    "Contact: {}\r\nExpires: {}\r\n",
                    header(&request, "Contact"),
                    header(&request, "Expires")
                )
            };
            let response = format!(
                "SIP/2.0 {status}\r\nVia: {}\r\nFrom: {}\r\nTo: {};tag=registrar\r\nCall-ID: {}\r\nCSeq: {}\r\n{extra}Content-Length: 0\r\n\r\n",
                header(&request, "Via"), header(&request, "From"), header(&request, "To"), header(&request, "Call-ID"), header(&request, "CSeq"),
            );
            socket.send_to(response.as_bytes(), peer).await.unwrap();
            requests.push(request);
            if unregister || reject {
                return requests;
            }
        }
    }

    async fn fixture(
        reject: bool,
    ) -> (
        Arc<UnifiedCoordinator>,
        SipOutboundConfig,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let carrier = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let registrar_uri = format!("sip:{}", carrier.local_addr().unwrap());
        let port = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut config = Config::local("local-user", port);
        config.contact_uri = Some(format!("sip:local-user@127.0.0.1:{port}"));
        let coordinator = UnifiedCoordinator::new(config).await.unwrap();
        let trunk = SipOutboundConfig {
            registrar_uri: Some(registrar_uri.clone()),
            proxy_uri: format!("{registrar_uri};lr"),
            identity_uri: "sip:+14155550101@local.invalid".into(),
            auth_username: "local-user".into(),
            auth_password_env: "UNUSED_LOCAL_TEST_PASSWORD".into(),
        };
        (coordinator, trunk, tokio::spawn(registrar(carrier, reject)))
    }

    #[tokio::test]
    async fn carrier_registration_authenticates_and_shutdown_removes_binding() {
        let (coordinator, trunk, carrier) = fixture(false).await;
        register_with_password(
            &coordinator,
            &trunk,
            trunk.registrar_uri.as_deref().unwrap(),
            "local-password".into(),
        )
        .await
        .unwrap();
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(5)))
            .await
            .unwrap();
        let requests = tokio::time::timeout(Duration::from_secs(5), carrier)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(requests.len(), 3);
        assert!(header(&requests[1], "Authorization").contains("qualification-nonce"));
        assert!(!requests[1].contains("local-password"));
        assert_eq!(header(&requests[2], "Expires"), "0");
        assert_eq!(
            header(&requests[0], "Call-ID"),
            header(&requests[2], "Call-ID")
        );
    }

    #[tokio::test]
    async fn rejected_registration_does_not_report_ready() {
        let (coordinator, trunk, carrier) = fixture(true).await;
        let error = register_with_password(
            &coordinator,
            &trunk,
            trunk.registrar_uri.as_deref().unwrap(),
            "local-password".into(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("rejected"));
        assert!(!error.to_string().contains("local-password"));
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(5)))
            .await
            .unwrap();
        assert_eq!(carrier.await.unwrap().len(), 1);
    }
}
