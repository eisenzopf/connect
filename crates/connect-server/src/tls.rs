use anyhow::{Context, Result};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::{path::Path, sync::Arc};
mod rotation;
pub use rotation::{ReloadableServerTls, TlsReload};

fn bounded(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let metadata = std::fs::metadata(path).context("Read configured TLS asset metadata")?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() > 0 && metadata.len() <= maximum as u64,
        "Configured TLS asset must be a bounded regular file"
    );
    let file = std::fs::File::open(path).context("Read configured TLS asset")?;
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .context("Read configured TLS asset")?;
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= maximum,
        "Configured TLS asset exceeds its bounds"
    );
    Ok(bytes)
}
fn certificates(bytes: Vec<u8>) -> Result<Vec<CertificateDer<'static>>> {
    let chain = if bytes.trim_ascii_start().starts_with(b"-----BEGIN") {
        CertificateDer::pem_slice_iter(&bytes)
            .take(9)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| anyhow::anyhow!("Invalid TLS certificate chain"))?
    } else {
        vec![CertificateDer::from(bytes)]
    };
    anyhow::ensure!(
        !chain.is_empty() && chain.len() <= 8,
        "Invalid TLS certificate chain bounds"
    );
    for cert in &chain {
        anyhow::ensure!(cert.len() <= 65_536, "TLS certificate exceeds its bounds");
        let (remaining, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
            .map_err(|_| anyhow::anyhow!("Invalid TLS certificate DER"))?;
        anyhow::ensure!(
            remaining.is_empty(),
            "Unexpected trailing TLS certificate data"
        );
        anyhow::ensure!(parsed.validity().is_valid(), "Server certificate is expired or not yet valid; refresh server TLS and configured client trust");
    }
    Ok(chain)
}
/// Check configured public contacts with the TLS library's DNS/IP matcher.
/// Certificate/key validity and matching are independently checked at load.
pub(crate) fn validate_name(certificate: &Path, name: &str) -> Result<()> {
    let chain = certificates(bounded(certificate, 256 * 1024)?)?;
    let leaf = rustls::server::ParsedCertificate::try_from(&chain[0])?;
    rustls::client::verify_server_name(&leaf, &rustls::pki_types::ServerName::try_from(name)?)
        .context("Configured HTTPS name is not covered by its TLS certificate")
}
pub fn server_config(cert: &Path, key: &Path) -> Result<Arc<rustls::ServerConfig>> {
    Ok(Arc::new(load_config(cert, key)?.0))
}
fn load_config(
    cert: &Path,
    key: &Path,
) -> Result<(rustls::ServerConfig, Vec<CertificateDer<'static>>)> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let chain = certificates(bounded(cert, 256 * 1024)?)?;
    let key = bounded(key, 16_384)?;
    let key = if key.trim_ascii_start().starts_with(b"-----BEGIN") {
        PrivateKeyDer::from_pem_slice(&key)
            .map_err(|_| anyhow::anyhow!("Invalid TLS private key"))?
    } else {
        PrivateKeyDer::try_from(key).map_err(|_| anyhow::anyhow!("Invalid TLS private key"))?
    };
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain.clone(), key)
        .context("Certificate and key must match")?;
    config.alpn_protocols = vec![rvoip_uctp::UCTP_RAW_QUIC_ALPN_BYTES.to_vec()];
    Ok((config, chain))
}

/// Local certificate with an explicit one-year lifetime and explicit localhost/loopback identities.
pub fn generate_local_certificate() -> Result<rcgen::CertifiedKey<rcgen::KeyPair>> {
    use chrono::Datelike;
    let today = chrono::Utc::now().date_naive();
    let before = today - chrono::Duration::days(1);
    let after = today + chrono::Duration::days(365);
    let mut params =
        rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into(), "::1".into()])?;
    params.not_before =
        rcgen::date_time_ymd(before.year(), before.month() as u8, before.day() as u8);
    params.not_after = rcgen::date_time_ymd(after.year(), after.month() as u8, after.day() as u8);
    let signing_key = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&signing_key)?;
    Ok(rcgen::CertifiedKey { cert, signing_key })
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_local_certificate_verifies_loopback_names_and_refuses_other_hosts() {
        use rustls::client::danger::ServerCertVerifier;
        use std::sync::Arc;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let generated = super::generate_local_certificate().unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(generated.cert.der().clone()).unwrap();
        let verifier = rustls::client::WebPkiServerVerifier::builder(Arc::new(roots))
            .build()
            .unwrap();
        for name in ["localhost", "127.0.0.1", "::1"] {
            verifier
                .verify_server_cert(
                    generated.cert.der(),
                    &[],
                    &rustls::pki_types::ServerName::try_from(name).unwrap(),
                    &[],
                    rustls::pki_types::UnixTime::now(),
                )
                .unwrap();
        }
        for name in ["foreign.fixture.invalid", "127.0.0.2", "::2"] {
            assert!(verifier
                .verify_server_cert(
                    generated.cert.der(),
                    &[],
                    &rustls::pki_types::ServerName::try_from(name).unwrap(),
                    &[],
                    rustls::pki_types::UnixTime::now()
                )
                .is_err());
        }
    }
}
