//! Explicit startup-owned public assistants and the same discovery bindings.
use crate::{
    assistant_vapi::{
        AssistantProviderTiming, AssistantVapiProfile, PublicVapiCoordinator, PublicVapiTarget,
    },
    canonical::CanonicalServerConfig,
    private_vapi::ProviderSecrets,
    secrets::{NativeSecretStore, SecretReferences, SecretStore},
};
use anyhow::{Context, Result};
use connect_client::canonical::CanonicalClientConfig;
use connect_protocol::{
    events::ObservationCapabilities,
    provisioning::EndpointTransport,
    routing::{ReachabilityPolicy, RouteMedium},
};
use connect_service::{Service, SessionInitializer};
use connect_store::{
    AssistantEndpointHost, DiscoveryRuntime, EndpointProfile, EndpointRuntime, Store,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalPublicVapiConfig {
    /// A protected references file. No ambient-name fallback is used.
    pub secret_references: PathBuf,
    pub targets: Vec<CanonicalPublicVapiTarget>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalPublicVapiTarget {
    pub native_peer: PathBuf,
    pub provider_endpoint: String,
    pub secret_ref: String,
    pub api_base: String,
    pub websocket_origin: String,
    #[serde(default)]
    pub loopback_fixture: bool,
    pub model: String,
    pub application_prompt: String,
    pub context_timing: ContextTiming,
    #[serde(default)]
    pub reachability: ReachabilityPolicy,
    /// Client-side proposals/observations only, published by this native peer.
    pub observations: Option<ObservationCapabilities>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextTiming {
    AfterCreation,
    Initialization,
}
impl CanonicalPublicVapiConfig {
    pub(crate) fn resolve_paths(&mut self, base: &Path) {
        if self.secret_references.is_relative() {
            self.secret_references = base.join(&self.secret_references);
        }
        for t in &mut self.targets {
            if t.native_peer.is_relative() {
                t.native_peer = base.join(&t.native_peer);
            }
        }
    }
    pub(crate) fn validate(&self, capacity: usize) -> Result<()> {
        anyhow::ensure!(
            !self.targets.is_empty() && self.targets.len() <= capacity.min(512),
            "Invalid public assistant target capacity"
        );
        let mut ids = BTreeSet::new();
        for t in &self.targets {
            connect_protocol::transitions::id(&t.provider_endpoint)
                .map_err(|_| anyhow::anyhow!("Invalid public assistant Endpoint reference"))?;
            connect_protocol::transitions::id(&t.secret_ref)
                .map_err(|_| anyhow::anyhow!("Invalid public assistant secret reference"))?;
            anyhow::ensure!(
                ids.insert(&t.provider_endpoint),
                "Duplicate public assistant Endpoint"
            );
            t.profile(None)?
                .validate()
                .map_err(|_| anyhow::anyhow!("Invalid public assistant transport/model profile"))?;
            if let Some(caps) = &t.observations {
                crate::agent_observations::AgentObservationConfig::new(
                    "configuration-validation".into(),
                    caps.clone(),
                )
                .map_err(|_| {
                    anyhow::anyhow!("Invalid public assistant observation capabilities")
                })?;
            }
        }
        Ok(())
    }
}
impl CanonicalPublicVapiTarget {
    fn profile(
        &self,
        observations: Option<crate::agent_observations::AgentObservationConfig>,
    ) -> Result<AssistantVapiProfile> {
        Ok(AssistantVapiProfile {
            api_base: self
                .api_base
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid public assistant API origin"))?,
            websocket_origin: self
                .websocket_origin
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid public assistant WebSocket origin"))?,
            loopback_fixture: self.loopback_fixture,
            model: self.model.clone(),
            application_prompt: self.application_prompt.clone(),
            context_timing: match self.context_timing {
                ContextTiming::AfterCreation => AssistantProviderTiming::AfterCreation,
                ContextTiming::Initialization => AssistantProviderTiming::Initialization,
            },
            observations,
        })
    }
}
struct Target {
    peer: CanonicalClientConfig,
    public_key: Vec<u8>,
    config: CanonicalPublicVapiTarget,
}
pub(crate) struct Loaded {
    targets: Vec<Target>,
    secrets: ProviderSecrets,
}
pub(crate) struct Prepared {
    targets: Vec<PublicVapiTarget>,
    secrets: ProviderSecrets,
    pub runtime: DiscoveryRuntime,
}
/// Also cancels when construction fails after installation but before ownership
/// reaches RunningCanonicalServer. Jobs retain and drain their captured effects.
pub(crate) struct Hosting(pub Arc<PublicVapiCoordinator>);
impl Drop for Hosting {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn file(path: &Path, limit: u64, private: bool) -> Result<Vec<u8>> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path).context("Read configured assistant asset")?;
    anyhow::ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() > 0 && meta.len() <= limit,
        "Invalid configured assistant asset"
    );
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "Configured assistant credentials/keys require protected files"
        );
    }
    let _ = private;
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut data)?;
    anyhow::ensure!(
        !data.is_empty() && data.len() as u64 <= limit,
        "Invalid configured assistant asset size"
    );
    Ok(data)
}
fn public_key(bytes: &[u8]) -> Result<Vec<u8>> {
    let pair = rcgen::KeyPair::try_from(bytes)
        .map_err(|_| anyhow::anyhow!("Invalid configured assistant signing key"))?;
    Ok(pair.public_key_raw().to_vec())
}
impl Loaded {
    /// Read/validate files and selected secrets before installation recovery or
    /// opening listeners. Only references survive; rotation/withdrawal is read
    /// again by actual provider jobs, without an ambient credential fallback.
    pub fn load(server: &CanonicalServerConfig) -> Result<Option<Self>> {
        let Some(config) = &server.public_vapi else {
            return Ok(None);
        };
        config.validate(server.limits.connections)?;
        let references = SecretReferences::load(&config.secret_references)
            .map_err(|_| anyhow::anyhow!("Public assistant secret references are unavailable"))?;
        let server_public = public_key(&file(&server.signing_private_key, 16384, false)?)?;
        let certificate = file(&server.certificate, 65536, false)?;
        let mut selected = BTreeMap::new();
        let mut targets = Vec::new();
        let mut identities = BTreeSet::new();
        for t in &config.targets {
            file(&t.native_peer, 65536, true)?;
            let peer = CanonicalClientConfig::load(&t.native_peer).map_err(|_| {
                anyhow::anyhow!("Invalid public assistant native peer configuration")
            })?;
            anyhow::ensure!(
                peer.timeout_ms <= 5000 && identities.insert(peer.identity_id.clone()),
                "Invalid or duplicate public assistant peer"
            );
            let private = file(&peer.signing_private_key, 16384, true)?;
            let public = public_key(&private)?;
            let valid = match peer.signing_algorithm.as_str() {
                "EdDSA" => {
                    rvoip_auth_core::sig9421::Sig9421Signer::ed25519(&peer.signing_key_id, &private)
                }
                "ES256" => {
                    rvoip_auth_core::sig9421::Sig9421Signer::es256(&peer.signing_key_id, &private)
                }
                _ => unreachable!("validated peer algorithm"),
            };
            valid.map_err(|_| anyhow::anyhow!("Invalid public assistant native signing key"))?;
            anyhow::ensure!(
                peer.server_key_id == server.signing_key_id
                    && peer.server_algorithm == server.signing_algorithm
                    && peer.server_key_not_before == server.signing_key_not_before
                    && peer.server_key_expires_at == server.signing_key_expires_at
                    && file(&peer.server_public_key, 128, false)? == server_public
                    && file(&peer.trust_certificate, 65536, false)? == certificate,
                "Public assistant peer does not pin this server"
            );
            if !selected.contains_key(&t.secret_ref) {
                let reference = references.references.get(&t.secret_ref).ok_or_else(|| {
                    anyhow::anyhow!("Selected public assistant secret reference is missing")
                })?;
                NativeSecretStore.read(reference).map_err(|_| {
                    anyhow::anyhow!("Selected public assistant secret is unavailable")
                })?;
                selected.insert(t.secret_ref.clone(), reference.clone());
            }
            targets.push(Target {
                peer,
                public_key: public,
                config: t.clone(),
            });
        }
        let secrets: ProviderSecrets = Arc::new(move |name| {
            let reference = selected.get(name).ok_or_else(|| {
                connect_protocol::Error::new(
                    "provider-secret-unavailable",
                    "Provider secret is not configured",
                )
            })?;
            NativeSecretStore.read(reference).map_err(|_| {
                connect_protocol::Error::new(
                    "provider-secret-unavailable",
                    "Selected provider secret is unavailable",
                )
            })
        });
        Ok(Some(Self { targets, secrets }))
    }
    pub fn prepare(self, store: &Store) -> Result<Prepared> {
        let mut bindings = Vec::new();
        let mut refs = BTreeSet::new();
        let mut targets = Vec::new();
        for t in self.targets {
            let actor = store
                .authenticate(&t.peer.credential)
                .map_err(|_| anyhow::anyhow!("Configured assistant credential is not current"))?;
            let metadata = store.credential_metadata(&actor.credential_id)?;
            let key = store.resolve_peer_signing_key(
                &actor.credential_id,
                &metadata.tenant_id,
                &t.peer.signing_key_id,
                &t.peer.signing_algorithm,
            )?;
            anyhow::ensure!(
                actor.identity_id == t.peer.identity_id
                    && key.device_id == t.peer.device_id
                    && key.public_key == t.public_key,
                "Configured assistant key is not bound to its current credential/Device"
            );
            let host = AssistantEndpointHost {
                endpoint_id: t.config.provider_endpoint.clone(),
                identity_id: t.peer.identity_id.clone(),
                credential_id: actor.credential_id.clone(),
                native_device_id: t.peer.device_id.clone(),
                native_device_kind: t.peer.device_kind.clone(),
                native_device_platform: t.peer.device_platform.clone(),
                signing_key_id: t.peer.signing_key_id.clone(),
                signing_algorithm: t.peer.signing_algorithm.clone(),
                secret_ref: t.config.secret_ref.clone(),
            };
            store.validate_assistant_host(&host).map_err(|_| {
                anyhow::anyhow!("Configured assistant Endpoint/key/scope binding is not current")
            })?;
            let observations = if let Some(caps) = &t.config.observations {
                actor.require("session.observation.publish")?;
                Some(crate::agent_observations::AgentObservationConfig::new(
                    actor.credential_id,
                    caps.clone(),
                )?)
            } else {
                None
            };
            let profile = t.config.profile(observations)?;
            refs.insert(t.config.secret_ref.clone());
            bindings.push(host);
            targets.push(PublicVapiTarget {
                native_peer: t.peer,
                provider_endpoint: t.config.provider_endpoint,
                profile,
                routing: Default::default(),
                reachability: t.config.reachability,
            });
        }
        let mut runtime = native_runtime()?;
        runtime.endpoints = EndpointRuntime::new(vec![
            native_profile(),
            EndpointProfile {
                transport: EndpointTransport::VapiVoice,
                media: vec![RouteMedium::Audio],
            },
        ])?
        .with_secret_refs(
            refs.into_iter()
                .map(|r| (EndpointTransport::VapiVoice, r))
                .collect(),
        )?
        .with_assistant_hosts(bindings)?;
        runtime.vapi_audio = true;
        for target in &mut targets {
            target.routing = runtime.clone();
        }
        Ok(Prepared {
            targets,
            secrets: self.secrets,
            runtime,
        })
    }
}
fn native_profile() -> EndpointProfile {
    EndpointProfile {
        transport: EndpointTransport::Quic,
        media: vec![
            RouteMedium::Audio,
            RouteMedium::Video,
            RouteMedium::Screen,
            RouteMedium::Data,
        ],
    }
}
pub(crate) fn native_runtime() -> Result<DiscoveryRuntime> {
    Ok(DiscoveryRuntime {
        endpoints: EndpointRuntime::new(vec![native_profile()])?,
        native_audio: true,
        native_video: true,
        ..Default::default()
    })
}
impl Prepared {
    pub fn install(mut self, service: &Arc<Service>, address: SocketAddr) -> Result<Hosting> {
        // Every configured native profile is local to this exact listener. A
        // changed ephemeral bind never selects a different host or credential.
        for target in &mut self.targets {
            target.native_peer.endpoint = address;
        }
        let host = PublicVapiCoordinator::install(service, self.targets, self.secrets)?;
        service.readiness.set(
            "public-assistant-host",
            connect_protocol::ReadinessState::Ready,
            "configured",
        );
        Ok(Hosting(host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> CanonicalPublicVapiTarget {
        CanonicalPublicVapiTarget {
            native_peer: "native.toml".into(),
            provider_endpoint: "provider_endpoint".into(),
            secret_ref: "provider_secret".into(),
            api_base: "https://api.vapi.ai/".into(),
            websocket_origin: "wss://designated.vapi.ai/".into(),
            loopback_fixture: false,
            model: "designated-model".into(),
            application_prompt: "Use approved public history.".into(),
            context_timing: ContextTiming::AfterCreation,
            reachability: ReachabilityPolicy::RequireFresh,
            observations: None,
        }
    }
    #[test]
    fn strict_host_configuration_preserves_explicit_paths_policy_and_rejects_unknown_fields() {
        let mut config = CanonicalPublicVapiConfig {
            secret_references: "secrets.toml".into(),
            targets: vec![target()],
        };
        config.validate(4).expect("bounded profile");
        let temp = tempfile::tempdir().expect("fixture");
        config.resolve_paths(temp.path());
        assert_eq!(config.secret_references, temp.path().join("secrets.toml"));
        assert_eq!(
            config.targets[0].native_peer,
            temp.path().join("native.toml")
        );
        let mut value = serde_json::to_value(&config).expect("config JSON");
        value["targets"][0]["credential"] = serde_json::json!("private-canary");
        assert!(serde_json::from_value::<CanonicalPublicVapiConfig>(value).is_err());
        config.targets.push(config.targets[0].clone());
        assert!(config.validate(4).is_err());
    }
    #[test]
    fn configured_assistant_assets_require_private_regular_files_without_echoing_contents() {
        let temp = tempfile::tempdir().expect("fixture");
        let path = temp.path().join("native.toml");
        let canary = "private-credential-canary";
        std::fs::write(&path, canary).expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
        }
        let error = file(&path, 65536, true).expect_err("unprotected credentials");
        assert!(!format!("{error:#}").contains(canary));
        assert!(file(&path, 1, false).is_err());
        #[cfg(unix)]
        {
            let link = temp.path().join("symlink");
            std::os::unix::fs::symlink(&path, &link).expect("symlink");
            assert!(file(&link, 65536, false).is_err());
        }
    }
    #[test]
    fn public_profile_never_enables_insecure_nonfixture_transport_or_duplicate_proposal_tools() {
        let mut t = target();
        t.api_base = "http://127.0.0.1:1234/".into();
        let mut c = CanonicalPublicVapiConfig {
            secret_references: "secrets.toml".into(),
            targets: vec![t],
        };
        assert!(c.validate(4).is_err());
        c.targets[0] = target();
        c.targets[0].observations = Some(ObservationCapabilities {
            transcript: false,
            speech: false,
            actions: vec!["proposal".into(), "proposal".into()],
        });
        assert!(c.validate(4).is_err());
    }
}
