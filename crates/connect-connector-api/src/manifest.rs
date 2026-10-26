//! Versioned, immutable declarations for explicit trusted composition.
//!
//! A manifest is documentation and dispatch validation, never authorization.
//! Registration contains references to secrets; resolved values stay in adapters.
use crate::{AddressKind, Descriptor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MANIFEST_VERSION: u16 = 2;
pub use crate::media::{
    ContextHandling, LiveBinding, LiveCodecs, LiveControl, MediaSupport, RecordHandling,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u16,
    pub name: String,
    pub address: AddressKind,
    pub messaging: bool,
    pub operations: BTreeSet<ConnectorOperation>,
    pub context: bool,
    pub sms: bool,
    /// Explicitly separates unsupported media, a lifecycle fixture, and an
    /// installed live host/transport contract. No variant grants authority.
    pub media: MediaSupport,
    pub limits: Limits,
    pub configuration: Vec<ConfigField>,
}
/// Operations are declared independently from codecs. A telephone control
/// adapter can expose start/status/end while media uses a separate rvoip binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConnectorOperation {
    MessageSend,
    MessageReceive,
    MessageStatus,
    ContextExport,
    VoiceStart,
    VoiceDiscover,
    VoiceStatus,
    VoiceReconcile,
    VoiceEnd,
    MediaExchange,
    LiveMedia,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioFormat {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_ms: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub message_bytes: usize,
    pub context_bytes: usize,
    pub event_bytes: usize,
    pub teardown_ms: u32,
    pub media_queue_frames: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigField {
    pub name: String,
    pub required: bool,
    pub secret: bool,
}
/// Safe readiness categories. Provider bodies, credentials and URLs never belong here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Readiness {
    Ready,
    MissingConfiguration,
    MissingSecret,
    Unavailable,
    ReconciliationRequired,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("incompatible connector manifest version")]
    Version,
    #[error("invalid or inconsistent connector manifest")]
    Manifest,
    #[error("missing or invalid connector configuration reference")]
    Configuration,
    #[error("connector registration conflicts with immutable contract")]
    Conflict,
    #[error("route address does not match connector contract")]
    Address,
    #[error("connector media is unsupported")]
    UnsupportedMedia,
    #[error("connector media runtime does not satisfy its contract")]
    MediaRuntime,
    #[error("current connection media negotiation is required")]
    MediaNegotiation,
}
fn name_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}
impl Manifest {
    pub fn messaging(descriptor: Descriptor) -> Self {
        Self {
            version: MANIFEST_VERSION,
            name: descriptor.name.into(),
            address: descriptor.address,
            messaging: true,
            operations: if descriptor.conversational {
                [
                    ConnectorOperation::MessageSend,
                    ConnectorOperation::ContextExport,
                ]
                .into()
            } else {
                [ConnectorOperation::MessageSend].into()
            },
            context: descriptor.conversational,
            sms: descriptor.sms,
            media: MediaSupport::Unsupported,
            limits: Limits {
                message_bytes: 40_000,
                context_bytes: 128_000,
                event_bytes: 16_384,
                teardown_ms: 5_000,
                media_queue_frames: 8,
            },
            configuration: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.version != MANIFEST_VERSION {
            return Err(ContractError::Version);
        }
        let l = &self.limits;
        let mut names = BTreeSet::new();
        use ConnectorOperation::*;
        let has = |op| self.operations.contains(&op);
        if !name_valid(&self.name)
            || self.operations.is_empty()
            || self.messaging != (has(MessageSend) || has(MessageReceive))
            || self.context != has(ContextExport)
            || (has(MessageStatus) && !self.messaging)
            || (has(VoiceStart) && (!has(VoiceStatus) || !has(VoiceEnd)))
            || (has(VoiceReconcile) && (!has(VoiceStatus) || !has(VoiceEnd)))
            || (has(MediaExchange) != matches!(&self.media, MediaSupport::LifecycleFixture { .. }))
            || (has(MediaExchange) && !has(VoiceStart))
            || (has(LiveMedia) != matches!(&self.media, MediaSupport::Live { .. }))
            || (self.context && !self.messaging && !has(VoiceStart) && !has(LiveMedia))
            || matches!(&self.media,MediaSupport::Live{context,..} if self.context != (*context==ContextHandling::HostSelected))
            || (self.sms
                && (!self.messaging
                    || self.context
                    || self.address != AddressKind::E164Pair
                    || self.media != MediaSupport::Unsupported
                    || has(VoiceStart)
                    || has(VoiceDiscover)
                    || has(VoiceStatus)
                    || has(VoiceReconcile)
                    || has(VoiceEnd)
                    || has(MediaExchange)
                    || has(LiveMedia)))
            || !(1..=1_048_576).contains(&l.message_bytes)
            || l.context_bytes > 1_048_576
            || (self.context && l.context_bytes == 0)
            || !(1..=65_536).contains(&l.event_bytes)
            || !(1..=5_000).contains(&l.teardown_ms)
            || !(1..=64).contains(&l.media_queue_frames)
            || self.configuration.len() > 32
            || self
                .configuration
                .iter()
                .any(|f| !name_valid(&f.name) || !names.insert(&f.name))
        {
            return Err(ContractError::Manifest);
        }
        self.media.validate()?;
        Ok(())
    }
    pub fn validate_media_runtime(
        &self,
        runtime: &crate::media::MediaRuntime,
    ) -> Result<(), ContractError> {
        self.validate()?;
        self.media.require_runtime(runtime)
    }
    /// Configuration receives opaque references, never a raw secret value.
    /// Resolution and secure storage are the host composition's responsibility.
    /// An explicitly mounted message account must bind its one required secret
    /// field to the selected protected host reference. Metadata grants no IO.
    pub fn validate_account_reference(
        &self,
        values: &BTreeMap<String, String>,
        reference: &str,
    ) -> Result<(), ContractError> {
        self.validate_configuration(values)?;
        let mut required = self.configuration.iter().filter(|f| f.required && f.secret);
        let field = required.next().ok_or(ContractError::Configuration)?;
        if required.next().is_some()
            || values.get(&field.name) != Some(&format!("secret:{reference}"))
        {
            return Err(ContractError::Configuration);
        }
        Ok(())
    }
    pub fn validate_configuration(
        &self,
        values: &BTreeMap<String, String>,
    ) -> Result<(), ContractError> {
        self.validate()?;
        for field in &self.configuration {
            let value = values.get(&field.name);
            if field.required && value.is_none_or(|v| v.is_empty()) {
                return Err(ContractError::Configuration);
            }
            if let Some(value) = value {
                if value.is_empty()
                    || value.len() > 256
                    || value.chars().any(char::is_control)
                    || (field.secret && !valid_secret_reference(value))
                {
                    return Err(ContractError::Configuration);
                }
            }
        }
        if values
            .keys()
            .any(|key| !self.configuration.iter().any(|f| &f.name == key))
        {
            return Err(ContractError::Configuration);
        }
        Ok(())
    }
    /// Enforce the descriptor used by durable dispatch; metadata cannot weaken SMS or context policy.
    pub fn validate_descriptor(&self, descriptor: Descriptor) -> Result<(), ContractError> {
        self.validate()?;
        if self.name != descriptor.name
            || self.address != descriptor.address
            || self.context != descriptor.conversational
            || self.sms != descriptor.sms
            || !self.messaging
        {
            return Err(ContractError::Manifest);
        }
        Ok(())
    }
    pub fn validate_address(
        &self,
        target: &str,
        source: Option<&str>,
    ) -> Result<(), ContractError> {
        let e164 = |s: &str| {
            s.starts_with('+')
                && (8..=16).contains(&s.len())
                && s.as_bytes()[1] != b'0'
                && s[1..].bytes().all(|b| b.is_ascii_digit())
        };
        let uuid = |s: &str| {
            s.len() == 36
                && s.bytes().enumerate().all(|(i, b)| {
                    if matches!(i, 8 | 13 | 18 | 23) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
        };
        let valid = match self.address {
            AddressKind::Opaque => {
                !target.is_empty()
                    && target.len() <= 256
                    && !target.chars().any(char::is_control)
                    && source.is_none_or(|s| {
                        !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
                    })
            }
            AddressKind::Uuid => uuid(target) && source.is_none(),
            AddressKind::E164Pair => e164(target) && source.is_some_and(e164),
        };
        if valid {
            Ok(())
        } else {
            Err(ContractError::Address)
        }
    }
}
fn valid_secret_reference(value: &str) -> bool {
    let Some((scheme, key)) = value.split_once(':') else {
        return false;
    };
    matches!(scheme, "env" | "secret")
        && !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
/// Process-local registry. Persist `snapshot()` in host release/configuration state
/// and use `restore()` before adding adapters on restart to detect contract drift.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Registry(BTreeMap<String, Manifest>);
impl Registry {
    pub fn register(&mut self, manifest: Manifest) -> Result<(), ContractError> {
        manifest.validate()?;
        if self.0.contains_key(&manifest.name) {
            return Err(ContractError::Conflict);
        }
        self.0.insert(manifest.name.clone(), manifest);
        Ok(())
    }
    pub fn restore(snapshot: &str) -> Result<Self, ContractError> {
        let registry: Self = serde_json::from_str(snapshot).map_err(|_| ContractError::Manifest)?;
        for (key, m) in &registry.0 {
            m.validate()?;
            if key != &m.name {
                return Err(ContractError::Manifest);
            }
        }
        Ok(registry)
    }
    pub fn snapshot(&self) -> Result<String, ContractError> {
        serde_json::to_string(self).map_err(|_| ContractError::Manifest)
    }
    pub fn require_compatible(&self, current: &Manifest) -> Result<(), ContractError> {
        current.validate()?;
        if self.0.get(&current.name) != Some(current) {
            return Err(ContractError::Conflict);
        }
        Ok(())
    }
}
