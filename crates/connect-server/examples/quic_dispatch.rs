//! Separate-process, authenticated native QUIC application dispatch experiment.
use anyhow::{Context, Result};
use async_trait::async_trait;
use connect_protocol::{PROFILE, REQUIRED_SCOPE};
use rvoip_auth_core::{BearerAuthError, BearerValidator};
use rvoip_core::{AuthenticatedPrincipal, IdentityAssurance};
use rvoip_quic::{UctpQuicAdapter, UctpQuicClient, UctpQuicConfig};
use rvoip_uctp::{
    application::{ApplicationContext, ApplicationError, ApplicationHandler},
    envelope::UctpEnvelope,
    substrate,
    types::MessageType,
};
use serde_json::json;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

struct Validator;
#[async_trait]
impl BearerValidator for Validator {
    async fn validate(
        &self,
        token: &str,
    ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
        Ok(self.validate_principal(token).await?.assurance)
    }
    async fn validate_principal(
        &self,
        token: &str,
    ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
        // Deliberately public experiment fixture. Production uses hashed scoped credentials.
        if token != "dispatch-fixture-only" {
            return Err(BearerAuthError::Invalid("invalid".into()));
        }
        let mut p = AuthenticatedPrincipal::from_assurance(IdentityAssurance::TaskScoped {
            identity: rvoip_core::ids::IdentityId::from_string("probe-client"),
            task_id: "dispatch-probe".into(),
            scopes: vec![REQUIRED_SCOPE.into()],
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        });
        p.subject = "probe-client".into();
        p.tenant = Some("dispatch-fixture".into());
        Ok(p)
    }
}
struct Handler(AtomicUsize);
#[async_trait]
impl ApplicationHandler for Handler {
    fn profile(&self) -> &'static str {
        PROFILE
    }
    fn required_scope(&self) -> &'static str {
        REQUIRED_SCOPE
    }
    async fn handle(
        &self,
        ctx: ApplicationContext,
        _request: UctpEnvelope,
    ) -> std::result::Result<UctpEnvelope, ApplicationError> {
        let count = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(UctpEnvelope::new(
            MessageType::Unknown("connect.result".into()),
            json!({"actor":ctx.principal.subject,"executions":count}),
        ))
    }
}
async fn receive(rx: &mut tokio::sync::mpsc::Receiver<UctpEnvelope>) -> Result<UctpEnvelope> {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await?
        .context("connection closed")
}
#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1).context("mode: server|client")?;
    let dir = Path::new(
        args.get(2)
            .context("explicit disposable state directory required")?,
    );
    if mode == "server" {
        let (cert, key) = substrate::self_signed_for_dev(&["localhost".into()])?;
        let mut tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)?;
        tls.alpn_protocols = vec![rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES.to_vec()];
        let ep = Arc::new(substrate::make_server_endpoint(
            "127.0.0.1:0".parse()?,
            Arc::new(tls),
            quinn::TransportConfig::default(),
        )?);
        let mut routes =
            substrate::dispatch_by_alpn(ep.clone(), &[rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES])?;
        let handler = Arc::new(Handler(AtomicUsize::new(0)));
        let adapter = UctpQuicAdapter::new(
            UctpQuicConfig::new(
                ep.clone(),
                routes
                    .take(rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES)
                    .context("route")?,
                Arc::new(Validator),
            )
            .with_application_handler(handler.clone()),
        )
        .await?;
        std::fs::write(dir.join("cert.der"), cert.as_ref())?;
        std::fs::write(dir.join("address"), ep.local_addr()?.to_string())?;
        println!(
            "ready transport=quic alpn={} profile={PROFILE}",
            rvoip_uctp::UCTP_RAW_QUIC_ALPN
        );
        tokio::signal::ctrl_c().await?;
        adapter.begin_drain();
        ep.close(0u32.into(), b"shutdown");
        ep.wait_idle().await;
        println!("executions={}", handler.0.load(Ordering::SeqCst));
    } else {
        let cert = rustls::pki_types::CertificateDer::from(std::fs::read(dir.join("cert.der"))?);
        let tls = Arc::new(substrate::dev_client_config_trusting(&cert)?);
        let ep = substrate::make_client_endpoint("127.0.0.1:0".parse()?, tls.clone())?;
        let client = UctpQuicClient::connect(
            &ep,
            std::fs::read_to_string(dir.join("address"))?.parse()?,
            "localhost",
            tls,
        )
        .await?;
        let mut rx = client.take_inbound().context("inbound")?;
        // Unauthenticated application calls must fail before reaching the handler.
        client
            .send(UctpEnvelope::new(
                MessageType::Unknown("connect.request".into()),
                json!({"profile":PROFILE}),
            ))
            .await?;
        assert_eq!(receive(&mut rx).await?.payload["code"], 401);
        let hello = UctpEnvelope::new(
            MessageType::AuthHello,
            json!({"device":{"id":"probe","kind":"desktop","platform":"macos","sdk_version":"probe/1"},"auth_methods":["bearer"],"capabilities":{}}),
        );
        client.send(hello).await?;
        let challenge = receive(&mut rx).await?;
        assert_eq!(challenge.msg_type, MessageType::AuthChallenge);
        assert_eq!(
            challenge.payload["server_capabilities"]["application_profiles"][0],
            PROFILE
        );
        client
            .send(UctpEnvelope::new(
                MessageType::AuthResponse,
                json!({"method":"bearer","credential":"dispatch-fixture-only"}),
            ))
            .await?;
        assert_eq!(receive(&mut rx).await?.msg_type, MessageType::AuthSession);
        let request = UctpEnvelope::new(
            MessageType::Unknown("connect.request".into()),
            json!({"profile":PROFILE,"op":"probe"}),
        );
        let id = request.id.clone();
        client.send(request.clone()).await?;
        let reply = receive(&mut rx).await?;
        assert_eq!(reply.in_reply_to.as_ref(), Some(&id));
        assert_eq!(reply.payload["actor"], "probe-client");
        assert_eq!(reply.payload["executions"], 1);
        client.send(request).await?;
        let replay = receive(&mut rx).await?;
        assert_eq!(replay.payload["code"], 409);
        client
            .send(UctpEnvelope::new(
                MessageType::Unknown("connect.request".into()),
                json!({"profile":"connect-control/999"}),
            ))
            .await?;
        assert_eq!(receive(&mut rx).await?.payload["code"], 501);
        client.connection.close(0u32.into(), b"done");
        ep.close(0u32.into(), b"done");
        ep.wait_idle().await;
        println!("PASS native QUIC; verified TLS; actor binding; correlation; auth/profile rejection; single handler execution");
    }
    Ok(())
}
