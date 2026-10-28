//! Explicit operator-owned TLS rotation shared by cloned listener configs.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Mutex, RwLock},
};

#[derive(Debug)]
struct Identity {
    resolver: Arc<dyn rustls::server::ResolvesServerCert>,
    digest: [u8; 32],
    names: BTreeSet<String>,
    revision: u64,
}
#[derive(Debug)]
struct Resolver(RwLock<Identity>);
impl rustls::server::ResolvesServerCert for Resolver {
    fn resolve(
        &self,
        hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        // No file IO or reload serialization is held by a handshake.
        let resolver = self.0.read().ok()?.resolver.clone();
        resolver.resolve(hello)
    }
}

/// Observation of one trusted file reload, without any certificate/key bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TlsReload {
    pub revision: u64,
    pub changed: bool,
}

/// Owns the original configured paths. A failed reload preserves the last
/// complete validated identity. Clones of `config()` share the same resolver;
/// existing TLS sessions retain their negotiated traffic keys.
pub struct ReloadableServerTls {
    certificate: PathBuf,
    private_key: PathBuf,
    config: Arc<rustls::ServerConfig>,
    resolver: Arc<Resolver>,
    reload: Mutex<()>,
}
fn identity(
    certificate: &Path,
    private_key: &Path,
) -> Result<(rustls::ServerConfig, [u8; 32], BTreeSet<String>)> {
    let (config, chain) = super::load_config(certificate, private_key)?;
    let mut digest = Sha256::new();
    for certificate in &chain {
        digest.update((certificate.len() as u64).to_be_bytes());
        digest.update(certificate.as_ref());
    }
    let (_, leaf) = x509_parser::parse_x509_certificate(chain[0].as_ref())
        .map_err(|_| anyhow::anyhow!("Invalid TLS identity"))?;
    let san = leaf
        .subject_alternative_name()
        .map_err(|_| anyhow::anyhow!("Invalid TLS identity names"))?
        .context("TLS identity requires subject alternative names")?;
    let mut names = BTreeSet::new();
    for name in &san.value.general_names {
        use x509_parser::extensions::GeneralName;
        match name {
            GeneralName::DNSName(value) => {
                names.insert(format!("dns:{}", value.to_ascii_lowercase()));
            }
            GeneralName::IPAddress(value) => {
                names.insert(format!("ip:{}", hex::encode(value)));
            }
            _ => (),
        }
    }
    anyhow::ensure!(!names.is_empty(), "TLS identity requires DNS or IP names");
    Ok((config, digest.finalize().into(), names))
}
impl ReloadableServerTls {
    pub fn load(certificate: &Path, private_key: &Path) -> Result<Self> {
        let (mut config, digest, names) = identity(certificate, private_key)?;
        let resolver = Arc::new(Resolver(RwLock::new(Identity {
            resolver: config.cert_resolver.clone(),
            digest,
            names,
            revision: 1,
        })));
        config.cert_resolver = resolver.clone();
        Ok(Self {
            certificate: certificate.to_owned(),
            private_key: private_key.to_owned(),
            config: Arc::new(config),
            resolver,
            reload: Mutex::new(()),
        })
    }
    pub fn config(&self) -> Arc<rustls::ServerConfig> {
        self.config.clone()
    }
    /// Synchronous bounded file IO. Async owners must call this off executor.
    /// Serialize before loading so concurrent callers cannot install an older
    /// captured pair after a newer successful reload. Publish under a short lock.
    pub fn reload(&self) -> Result<TlsReload> {
        let _reload = self
            .reload
            .lock()
            .map_err(|_| anyhow::anyhow!("TLS reload custody unavailable"))?;
        let (config, digest, names) = identity(&self.certificate, &self.private_key)?;
        let mut current = self
            .resolver
            .0
            .write()
            .map_err(|_| anyhow::anyhow!("TLS identity custody unavailable"))?;
        // A hot certificate update cannot silently remove an existing served
        // DNS/IP identity. Change deployment contacts through explicit restart.
        anyhow::ensure!(
            current.names.is_subset(&names),
            "TLS rotation cannot remove configured identity names"
        );
        if current.digest == digest {
            return Ok(TlsReload {
                revision: current.revision,
                changed: false,
            });
        }
        let revision = current
            .revision
            .checked_add(1)
            .context("TLS identity revision exhausted")?;
        *current = Identity {
            resolver: config.cert_resolver,
            digest,
            names,
            revision,
        };
        Ok(TlsReload {
            revision,
            changed: true,
        })
    }
}

#[cfg(test)]
mod tests;
