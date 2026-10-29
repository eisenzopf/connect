use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::{
    rand::SystemRandom,
    signature::{self, KeyPair},
};

/// Client-owned key material. Explicit save is private/exclusive; Debug is redacted.
pub struct NativeEnrollmentKey {
    algorithm: SigningAlgorithm,
    pkcs8: Vec<u8>,
    public: String,
}
impl std::fmt::Debug for NativeEnrollmentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeEnrollmentKey(redacted)")
    }
}
impl NativeEnrollmentKey {
    pub fn generate(algorithm: SigningAlgorithm) -> Result<Self> {
        let rng = SystemRandom::new();
        let key = match algorithm {
            SigningAlgorithm::EdDSA => signature::Ed25519KeyPair::generate_pkcs8(&rng),
            SigningAlgorithm::ES256 => signature::EcdsaKeyPair::generate_pkcs8(
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                &rng,
            ),
        }
        .map_err(|_| ClientError::Configuration)?;
        Self::from_pkcs8(algorithm, key.as_ref().to_vec())
    }
    fn from_pkcs8(algorithm: SigningAlgorithm, pkcs8: Vec<u8>) -> Result<Self> {
        let public = match algorithm {
            SigningAlgorithm::EdDSA => signature::Ed25519KeyPair::from_pkcs8(&pkcs8)
                .map_err(|_| ClientError::Configuration)?
                .public_key()
                .as_ref()
                .to_vec(),
            SigningAlgorithm::ES256 => signature::EcdsaKeyPair::from_pkcs8(
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                &pkcs8,
                &SystemRandom::new(),
            )
            .map_err(|_| ClientError::Configuration)?
            .public_key()
            .as_ref()
            .to_vec(),
        };
        Ok(Self {
            algorithm,
            pkcs8,
            public: URL_SAFE_NO_PAD.encode(public),
        })
    }
    pub fn load(path: &Path, algorithm: SigningAlgorithm) -> Result<Self> {
        Self::from_pkcs8(algorithm, state::read_private(path, 16_384)?)
    }
    /// Persist before requesting approval so an application restart retains the
    /// same public key and original intent. Never replaces another file.
    pub fn save(&self, path: &Path) -> Result<()> {
        state::write_private(path, &self.pkcs8)
    }
    pub fn algorithm(&self) -> SigningAlgorithm {
        self.algorithm
    }
    pub fn public_key(&self) -> &str {
        &self.public
    }
    pub(super) fn sign(&self, message: &[u8]) -> Result<String> {
        let bytes = match self.algorithm {
            SigningAlgorithm::EdDSA => signature::Ed25519KeyPair::from_pkcs8(&self.pkcs8)
                .map_err(|_| ClientError::Configuration)?
                .sign(message)
                .as_ref()
                .to_vec(),
            SigningAlgorithm::ES256 => signature::EcdsaKeyPair::from_pkcs8(
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                &self.pkcs8,
                &SystemRandom::new(),
            )
            .map_err(|_| ClientError::Configuration)?
            .sign(&SystemRandom::new(), message)
            .map_err(|_| ClientError::Configuration)?
            .as_ref()
            .to_vec(),
        };
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    }
}
