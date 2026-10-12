use anyhow::{Context, Result};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::{path::Path, sync::Arc};

pub fn server_config(cert: &Path, key: &Path) -> Result<Arc<rustls::ServerConfig>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let bytes = std::fs::read(cert).context("Read server certificate")?;
    let (_, parsed) = x509_parser::parse_x509_certificate(&bytes)
        .map_err(|_| anyhow::anyhow!("Server certificate is invalid DER"))?;
    anyhow::ensure!(parsed.validity().is_valid(), "Server certificate is expired or not yet valid; stop the server and run rotate-tls, then refresh client trust");
    let cert = CertificateDer::from(bytes);
    let key = PrivatePkcs8KeyDer::from(std::fs::read(key).context("Read private key")?);
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key.into())
        .context("Certificate and key must match")?;
    config.alpn_protocols = vec![rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES.to_vec()];
    Ok(Arc::new(config))
}

/// Local certificate with an explicit one-year lifetime and localhost identity.
pub fn generate_local_certificate() -> Result<rcgen::CertifiedKey<rcgen::KeyPair>> {
    use chrono::Datelike;
    let today = chrono::Utc::now().date_naive();
    let before = today - chrono::Duration::days(1);
    let after = today + chrono::Duration::days(365);
    let mut params = rcgen::CertificateParams::new(vec!["localhost".into()])?;
    params.not_before =
        rcgen::date_time_ymd(before.year(), before.month() as u8, before.day() as u8);
    params.not_after = rcgen::date_time_ymd(after.year(), after.month() as u8, after.day() as u8);
    let signing_key = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&signing_key)?;
    Ok(rcgen::CertifiedKey { cert, signing_key })
}
