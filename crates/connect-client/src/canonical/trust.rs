//! Explicit bounded trust roots for native TLS and its pinned record origin.
use super::*;
use rustls::pki_types::{pem::PemObject, CertificateDer};
pub(super) const MAX_TRUST_BYTES: usize = 256 * 1024;
const MAX_CERTIFICATES: usize = 8;
pub(super) fn certificates(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>> {
    if bytes.is_empty() || bytes.len() > MAX_TRUST_BYTES {
        return Err(ClientError::Configuration);
    }
    let certificates = if bytes.trim_ascii_start().starts_with(b"-----BEGIN") {
        CertificateDer::pem_slice_iter(bytes)
            .take(MAX_CERTIFICATES + 1)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| ClientError::Configuration)?
    } else {
        vec![CertificateDer::from(bytes.to_vec())]
    };
    if certificates.is_empty()
        || certificates.len() > MAX_CERTIFICATES
        || certificates.iter().any(|c| c.len() > 65_536)
    {
        return Err(ClientError::Configuration);
    }
    Ok(certificates)
}
pub(super) fn client_config(bytes: &[u8]) -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in certificates(bytes)? {
        roots
            .add(certificate)
            .map_err(|_| ClientError::Configuration)?;
    }
    Ok(rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_der_and_pem_roots_never_fall_back_to_unconfigured_system_trust() {
        let cert =
            rcgen::generate_simple_self_signed(vec!["remote.example.invalid".into()]).unwrap();
        assert!(client_config(cert.cert.der().as_ref()).is_ok());
        assert!(client_config(cert.cert.pem().as_bytes()).is_ok());
        assert!(
            client_config(format!("{}{}", cert.cert.pem(), cert.cert.pem()).as_bytes()).is_ok()
        );
        for invalid in [
            Vec::new(),
            b"not a certificate".to_vec(),
            vec![0; MAX_TRUST_BYTES + 1],
            cert.cert.pem().repeat(MAX_CERTIFICATES + 1).into_bytes(),
        ] {
            assert!(client_config(&invalid).is_err());
        }
    }
}
