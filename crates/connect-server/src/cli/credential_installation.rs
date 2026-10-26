//! Durable scoped issuance plus independent native enrollment. No parent signer
//! or bearer is installed, and retries never choose new authority or logical IDs.
mod files;
#[cfg(all(test, unix))]
mod tests;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    application::{credential_issuance::*, ApplicationClient, ApplicationClientConfig},
    canonical::{CanonicalClient, CanonicalClientConfig},
};
use connect_protocol::provisioning::*;
use files::Directory;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};
const PROFILE: &str = "connect-credential-installation/1";
const PLACEHOLDER: &str = "pending-administrative-issuance";
const ASSETS: [&str; 7] = [
    "intent.json",
    "prepared.json",
    "device-signing.der",
    "certificate.der",
    "server-signing.pub",
    "issued.json",
    "client.toml",
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    profile: String,
    request: CredentialIssueRequest,
    issuer_sha256: String,
    application_endpoint: SocketAddr,
    application_timeout_ms: u64,
    native: CanonicalClientConfig,
    not_before: i64,
    public_key: String,
    assets: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Complete {
    profile: String,
    status: String,
    credential_id: String,
    assets: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    profile: String,
    intent_sha256: String,
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("Invalid protected installation JSON"))
}
fn admin(path: &Path) -> Result<ApplicationClientConfig> {
    let bytes = files::read(path, true)?;
    let config: ApplicationClientConfig =
        toml::from_str(std::str::from_utf8(&bytes).context("Invalid application configuration")?)
            .map_err(|_| anyhow::anyhow!("Invalid administrator application configuration"))?;
    config.validate()?;
    Ok(config)
}
fn operation(intent: &Intent, phase: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"connect-native-credential-enrollment/1\0");
    hash.update(intent.request.operation_id.as_bytes());
    hash.update(b"\0");
    hash.update(phase.as_bytes());
    format!("enroll_{}", hex::encode(hash.finalize()))
}
fn algorithm(name: &str) -> Result<&'static rcgen::SignatureAlgorithm> {
    match name {
        "EdDSA" => Ok(&rcgen::PKCS_ED25519),
        "ES256" => Ok(&rcgen::PKCS_ECDSA_P256_SHA256),
        _ => anyhow::bail!("Choose EdDSA or ES256 for the new Device key"),
    }
}
fn public_key(key: &[u8], name: &str) -> Result<String> {
    let key = rustls::pki_types::PrivateKeyDer::try_from(key)
        .map_err(|_| anyhow::anyhow!("Invalid saved Device key"))?;
    let pair = rcgen::KeyPair::from_der_and_sign_algo(&key, algorithm(name)?)
        .map_err(|_| anyhow::anyhow!("Invalid saved Device key"))?;
    Ok(URL_SAFE_NO_PAD.encode(pair.public_key_raw()))
}
fn validate_trust(bytes: &[u8]) -> Result<()> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= files::MAX_TRUST,
        "Invalid native trust bounds"
    );
    let certificates = if bytes.trim_ascii_start().starts_with(b"-----BEGIN") {
        CertificateDer::pem_slice_iter(bytes)
            .take(9)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| anyhow::anyhow!("Invalid native PEM trust bundle"))?
    } else {
        vec![CertificateDer::from(bytes.to_vec())]
    };
    anyhow::ensure!(
        !certificates.is_empty() && certificates.len() <= 8,
        "Invalid native trust certificate count"
    );
    let mut roots = rustls::RootCertStore::empty();
    for certificate in certificates {
        anyhow::ensure!(
            certificate.len() <= 65_536,
            "Native trust certificate exceeds64KiB"
        );
        let (remaining, _) = x509_parser::parse_x509_certificate(&certificate)
            .map_err(|_| anyhow::anyhow!("Invalid native trust certificate"))?;
        anyhow::ensure!(
            remaining.is_empty(),
            "Unexpected trailing trust certificate data"
        );
        // Keep the SDK's trust-anchor semantics. Actual TLS admission checks
        // the selected server certificate; unused roots do not select a peer.
        roots
            .add(certificate)
            .map_err(|_| anyhow::anyhow!("Invalid native trust root"))?;
    }
    Ok(())
}
/// New output directory only. Save immutable request/key/contact material before
/// any network mutation. A stopped initial preparation without intent is refused.
pub async fn issue(
    config: &Path,
    template: &Path,
    request: &Path,
    output: &Path,
    signing_algorithm: &str,
) -> Result<()> {
    let admin = admin(config)?;
    let mut request: CredentialIssueRequest = decode(&files::read(request, false)?)?;
    request.normalize()?;
    anyhow::ensure!(
        request.expires_at > chrono::Utc::now().timestamp(),
        "Credential intent is already expired"
    );
    let bytes = files::read(template, true)?;
    let mut source: CanonicalClientConfig =
        toml::from_str(std::str::from_utf8(&bytes).context("Invalid native template")?)
            .map_err(|_| anyhow::anyhow!("Invalid native template"))?;
    source.validate()?;
    // Resolve only public contact assets. Never open the template's signer.
    for path in [&mut source.trust_certificate, &mut source.server_public_key] {
        if path.is_relative() {
            *path = template.parent().unwrap_or(Path::new(".")).join(&*path);
        }
    }
    anyhow::ensure!(
        source.endpoint.port() != 0,
        "Native template must select a concrete listener"
    );
    let certificate = files::read_limit(&source.trust_certificate, false, files::MAX_TRUST)?;
    validate_trust(&certificate)?;
    let server_public = files::read(&source.server_public_key, false)?;
    anyhow::ensure!(
        (source.server_algorithm == "EdDSA" && server_public.len() == 32)
            || (source.server_algorithm == "ES256"
                && server_public.len() == 65
                && server_public[0] == 4),
        "Invalid native signing pin"
    );
    let pair = rcgen::KeyPair::generate_for(algorithm(signing_algorithm)?)?;
    let private = pair.serialize_der();
    let native = CanonicalClientConfig {
        network_profile: source.network_profile,
        endpoint: source.endpoint,
        server_name: source.server_name,
        trust_certificate: PathBuf::from("certificate.der"),
        credential: PLACEHOLDER.into(),
        identity_id: request.identity_id.clone(),
        device_id: uuid::Uuid::new_v4().to_string(),
        device_kind: "desktop".into(),
        device_platform: "connect-cli-issued".into(),
        signing_key_id: uuid::Uuid::new_v4().to_string(),
        signing_algorithm: signing_algorithm.into(),
        signing_private_key: PathBuf::from("device-signing.der"),
        server_key_id: source.server_key_id,
        server_algorithm: source.server_algorithm,
        server_public_key: PathBuf::from("server-signing.pub"),
        server_key_not_before: source.server_key_not_before,
        server_key_expires_at: source.server_key_expires_at,
        timeout_ms: source.timeout_ms,
    };
    let intent = Intent {
        profile: PROFILE.into(),
        request,
        issuer_sha256: digest(admin.credential.as_bytes()),
        application_endpoint: admin.endpoint,
        application_timeout_ms: admin.timeout_ms,
        native,
        not_before: chrono::Utc::now().timestamp() - 1,
        public_key: URL_SAFE_NO_PAD.encode(pair.public_key_raw()),
        assets: BTreeMap::from([
            ("device-signing.der".into(), digest(&private)),
            ("certificate.der".into(), digest(&certificate)),
            ("server-signing.pub".into(), digest(&server_public)),
        ]),
    };
    let directory = Directory::open(output, true)?;
    directory.publish("device-signing.der", &private)?;
    directory.publish("certificate.der", &certificate)?;
    directory.publish("server-signing.pub", &server_public)?;
    let bytes = serde_json::to_vec_pretty(&intent)?;
    directory.publish("intent.json", &bytes)?;
    directory.publish(
        "prepared.json",
        &serde_json::to_vec_pretty(&Prepared {
            profile: PROFILE.into(),
            intent_sha256: digest(&bytes),
        })?,
    )?;
    run(directory, admin).await
}
/// Resume only this saved installation, original issuer and concrete endpoint.
/// No ID, Device, key, expiry or output file is silently replaced after failure.
pub async fn resume(config: &Path, directory: &Path) -> Result<()> {
    let admin = admin(config)?;
    let directory = Directory::open(directory, false)?;
    run(directory, admin).await
}
fn validate(directory: &Directory, admin: &ApplicationClientConfig) -> Result<Intent> {
    let bytes = directory.read("intent.json")?;
    let prepared: Prepared = decode(&directory.read("prepared.json")?)?;
    anyhow::ensure!(
        prepared.profile == PROFILE && prepared.intent_sha256 == digest(&bytes),
        "Saved original intent changed or preparation is incomplete"
    );
    let intent: Intent = decode(&bytes)?;
    anyhow::ensure!(
        intent.profile == PROFILE
            && intent.issuer_sha256 == digest(admin.credential.as_bytes())
            && intent.application_endpoint == admin.endpoint,
        "Resume requires the original issuer and application endpoint"
    );
    let mut normalized = intent.request.clone();
    normalized.normalize()?;
    anyhow::ensure!(
        normalized == intent.request && intent.request.expires_at > chrono::Utc::now().timestamp(),
        "Saved credential intent is invalid or expired"
    );
    anyhow::ensure!(
        intent.native.credential == PLACEHOLDER
            && intent.native.identity_id == intent.request.identity_id
            && intent.native.endpoint.port() != 0
            && intent.native.device_kind == "desktop"
            && intent.native.device_platform == "connect-cli-issued"
            && intent.native.trust_certificate == Path::new("certificate.der")
            && intent.native.server_public_key == Path::new("server-signing.pub")
            && intent.native.signing_private_key == Path::new("device-signing.der")
            && intent.not_before > 0
            && intent.not_before <= chrono::Utc::now().timestamp()
            && intent.not_before < intent.request.expires_at
            && (1..=60_000).contains(&intent.application_timeout_ms),
        "Saved native enrollment intent is invalid"
    );
    intent.native.validate()?;
    uuid::Uuid::parse_str(&intent.native.device_id).context("Invalid saved Device ID")?;
    uuid::Uuid::parse_str(&intent.native.signing_key_id).context("Invalid saved key ID")?;
    anyhow::ensure!(
        intent.assets.len() == 3,
        "Unexpected installation asset inventory"
    );
    for name in [
        "device-signing.der",
        "certificate.der",
        "server-signing.pub",
    ] {
        anyhow::ensure!(
            intent.assets.get(name) == Some(&digest(&directory.read(name)?)),
            "Installation key or trust asset changed; preserved without replacement"
        );
    }
    anyhow::ensure!(
        public_key(
            &directory.read("device-signing.der")?,
            &intent.native.signing_algorithm
        )? == intent.public_key,
        "Saved Device public/private key differs"
    );
    validate_trust(&directory.read("certificate.der")?)?;
    Ok(intent)
}
fn configuration(intent: &Intent, issued: &CredentialIssueOutcome) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut native = intent.native.clone();
    native.credential = issued.credential.clone();
    let application = ApplicationClientConfig {
        endpoint: intent.application_endpoint,
        credential: issued.credential.clone(),
        expires_at: chrono::DateTime::from_timestamp(issued.expires_at, 0)
            .context("Invalid child expiry")?,
        timeout_ms: intent.application_timeout_ms,
    };
    Ok((
        toml::to_string_pretty(&native)?.into_bytes(),
        toml::to_string_pretty(&application)?.into_bytes(),
    ))
}
fn validate_issued(intent: &Intent, issued: &CredentialIssueOutcome) -> Result<()> {
    anyhow::ensure!(
        issued.operation_id == intent.request.operation_id
            && issued.identity_id == intent.request.identity_id
            && issued.operations == intent.request.operations
            && issued.recipients == intent.request.recipients
            && issued.expires_at == intent.request.expires_at
            && issued.issued_at > 0
            && issued.issued_at <= chrono::Utc::now().timestamp()
            && issued.issued_at < issued.expires_at
            && issued.credential.len() == 64
            && issued
                .credential
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "Saved issuance differs from original intent"
    );
    for id in [
        &issued.credential_id,
        &issued.issuer_identity_id,
        &issued.issuer_credential_id,
        &issued.installation_id,
    ] {
        uuid::Uuid::parse_str(id).context("Invalid saved issuance binding")?;
    }
    Ok(())
}
fn optional_outputs(
    directory: &Directory,
    intent: &Intent,
    issued: &CredentialIssueOutcome,
) -> Result<()> {
    let (native, application) = configuration(intent, issued)?;
    for (name, expected) in [("client.toml", native), ("application.toml", application)] {
        if let Some(actual) = directory.optional(name)? {
            anyhow::ensure!(
                actual == expected,
                "Installed credential configuration changed; preserved without replacement"
            );
        }
    }
    Ok(())
}
fn complete(directory: &Directory, issued: &CredentialIssueOutcome) -> Result<Complete> {
    let mut assets = BTreeMap::new();
    for name in ASSETS.into_iter().chain(["application.toml"]) {
        assets.insert(name.into(), digest(&directory.read(name)?));
    }
    Ok(Complete {
        profile: PROFILE.into(),
        status: "native-authenticated".into(),
        credential_id: issued.credential_id.clone(),
        assets,
    })
}
async fn run(directory: Directory, admin: ApplicationClientConfig) -> Result<()> {
    let intent = validate(&directory, &admin)?;
    let cached = directory
        .optional("issued.json")?
        .map(|b| decode::<CredentialIssueOutcome>(&b))
        .transpose()?;
    if let Some(issued) = &cached {
        validate_issued(&intent, issued)?;
        optional_outputs(&directory, &intent, issued)?;
    } else {
        anyhow::ensure!(
            directory.optional("client.toml")?.is_none()
                && directory.optional("application.toml")?.is_none()
                && directory.optional("install.json")?.is_none(),
            "Incomplete issuance has unexpected completed assets"
        );
    }
    if let Some(marker) = directory.optional("install.json")? {
        let expected = complete(
            &directory,
            cached.as_ref().context("Missing original issuance")?,
        )?;
        anyhow::ensure!(
            decode::<Complete>(&marker)? == expected,
            "Completion marker or assets changed; preserved without replacement"
        );
    }
    let mut client = ApplicationClient::open(admin.clone()).await?;
    let issued = client
        .issue_scoped_credential(intent.request.clone())
        .await
        .context("Issuance not confirmed; resume this directory with the original issuer")?;
    if let Some(cached) = cached {
        anyhow::ensure!(
            issued == cached,
            "Server issuance binding differs from saved original"
        );
    }
    directory.publish("issued.json", &serde_json::to_vec_pretty(&issued)?)?;
    let endpoint = ProvisioningCommand::Endpoint {
        operation_id: operation(&intent, "device"),
        identity_id: intent.request.identity_id.clone(),
        device: DeviceSpec {
            id: intent.native.device_id.clone(),
            kind: intent.native.device_kind.clone(),
            platform: intent.native.device_platform.clone(),
        },
        transport: EndpointTransport::Quic,
        address: format!(
            "quic://{}/{}",
            intent.native.endpoint, intent.native.device_id
        ),
        capabilities: vec!["data".into()],
        secret_ref: None,
    };
    let enrolled = client
        .provision(endpoint)
        .await
        .context("Device enrollment not confirmed; resume with the same saved intent")?;
    anyhow::ensure!(
        matches!(enrolled,ProvisioningOutcome::Endpoint {ref verification_state,ref capabilities,..} if verification_state!="revoked" && capabilities==&vec!["data".to_string()]),
        "Saved Device enrollment is no longer current"
    );
    let key = ProvisioningCommand::SigningKey {
        operation_id: operation(&intent, "key"),
        identity_id: intent.request.identity_id.clone(),
        credential_id: issued.credential_id.clone(),
        device_id: intent.native.device_id.clone(),
        key_id: intent.native.signing_key_id.clone(),
        algorithm: if intent.native.signing_algorithm == "ES256" {
            SigningAlgorithm::ES256
        } else {
            SigningAlgorithm::EdDSA
        },
        public_key: intent.public_key.clone(),
        not_before: intent.not_before,
        expires_at: intent.request.expires_at,
        expected_revision: None,
    };
    anyhow::ensure!(
        matches!(
            client
                .provision(key)
                .await
                .context("Signing enrollment not confirmed; resume with the same saved intent")?,
            ProvisioningOutcome::SigningKey { revoked: false, .. }
        ),
        "Saved signing key is no longer current"
    );
    let (native, application) = configuration(&intent, &issued)?;
    directory.publish("client.toml", &native)?;
    directory.publish("application.toml", &application)?;
    let mut config = intent.native.clone();
    directory.check_path()?;
    config.credential = issued.credential.clone();
    for path in [
        &mut config.trust_certificate,
        &mut config.server_public_key,
        &mut config.signing_private_key,
    ] {
        *path = directory.path.join(&*path);
    }
    let native = CanonicalClient::open(config)
        .await
        .context("Native admission not confirmed; preserve the saved installation for recovery")?;
    native
        .shutdown()
        .await
        .context("Native shutdown not confirmed; preserve the saved installation")?;
    let rechecked = validate(&directory, &admin)?;
    anyhow::ensure!(
        serde_json::to_value(&rechecked)? == serde_json::to_value(&intent)?,
        "Saved intent changed during enrollment"
    );
    optional_outputs(&directory, &intent, &issued)?;
    directory.check_path()?;
    anyhow::ensure!(
        decode::<CredentialIssueOutcome>(&directory.read("issued.json")?)? == issued,
        "Saved issuance changed during enrollment"
    );
    directory.publish(
        "install.json",
        &serde_json::to_vec_pretty(&complete(&directory, &issued)?)?,
    )?;
    println!(
        "{}",
        serde_json::json!({"status":"credential-installed-or-reconciled","operation_id":issued.operation_id,"identity_id":issued.identity_id,"credential_id":issued.credential_id,"directory":directory.path})
    );
    Ok(())
}
