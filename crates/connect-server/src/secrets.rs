//! Server-only secret references. Missing/locked storage never falls back to plaintext.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Serialized references contain identifiers only. Values are never serialized.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "storage", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretReference {
    Keychain {
        service: String,
        account: String,
        #[serde(default)]
        keychain: Option<PathBuf>,
    },
    Environment {
        variable: String,
    },
}
/// Explicit configuration for server/provider secrets, separate from client credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretReferences {
    pub version: u32,
    pub references: BTreeMap<String, SecretReference>,
}
/// Minimal replacement boundary for native storage and deterministic fixture adapters.
pub trait SecretStore {
    fn read(&self, reference: &SecretReference) -> Result<SecretValue>;
}
/// Explicit operator rotation/removal. Environment references are read-only.
pub trait MutableSecretStore: SecretStore {
    fn replace(&self, reference: &SecretReference, value: &SecretValue) -> Result<()>;
    fn remove(&self, reference: &SecretReference) -> Result<()>;
}
/// Deliberate access to plaintext; debug/log output is redacted.
pub struct SecretValue(String);
impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl SecretValue {
    pub fn new(value: String) -> Result<Self> {
        anyhow::ensure!(
            !value.trim().is_empty() && value.len() <= 65536,
            "Secret is empty or exceeds limit"
        );
        Ok(Self(value))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
pub struct NativeSecretStore;
impl NativeSecretStore {
    /// Create an installation-owned immutable secret without replacing an existing
    /// entry. Reconciliation/read-back belongs to the authenticated vault adapter.
    pub fn insert_new(&self, reference: &SecretReference, value: &SecretValue) -> Result<()> {
        reference.validate()?;
        #[cfg(target_os = "macos")]
        if let SecretReference::Keychain {
            service,
            account,
            keychain,
        } = reference
        {
            use security_framework::os::macos::keychain::SecKeychain;
            let _lock = KEYCHAIN_ACCESS
                .lock()
                .map_err(|_| anyhow::anyhow!("Keychain adapter unavailable"))?;
            let _interaction = SecKeychain::disable_user_interaction()
                .map_err(|_| anyhow::anyhow!("Cannot disable interactive Keychain prompts"))?;
            let chain = if let Some(path) = keychain {
                SecKeychain::open(path)
            } else {
                SecKeychain::default()
            }
            .map_err(|_| anyhow::anyhow!("Selected Keychain is unavailable"))?;
            chain.add_generic_password(service,account,value.expose().as_bytes()).map_err(|_|anyhow::anyhow!("Keychain insertion missing, denied, locked or already exists; no existing entry was replaced"))?;
            return Ok(());
        }
        let _ = value;
        anyhow::bail!("Immutable setup secrets require the explicitly selected macOS Keychain")
    }
}
impl MutableSecretStore for NativeSecretStore {
    fn replace(&self, reference: &SecretReference, value: &SecretValue) -> Result<()> {
        reference.validate()?;
        match reference {
            SecretReference::Keychain {
                service,
                account,
                keychain,
            } => keychain_mutation(service, account, keychain.as_deref(), Some(value)),
            _ => anyhow::bail!("Environment references cannot be rotated by Connect"),
        }
    }
    fn remove(&self, reference: &SecretReference) -> Result<()> {
        reference.validate()?;
        match reference {
            SecretReference::Keychain {
                service,
                account,
                keychain,
            } => keychain_mutation(service, account, keychain.as_deref(), None),
            _ => anyhow::bail!("Environment references cannot be removed by Connect"),
        }
    }
}
#[cfg(target_os = "macos")]
static KEYCHAIN_ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(target_os = "macos")]
fn keychain_mutation(
    service: &str,
    account: &str,
    path: Option<&Path>,
    value: Option<&SecretValue>,
) -> Result<()> {
    use security_framework::os::macos::keychain::SecKeychain;
    let _lock = KEYCHAIN_ACCESS
        .lock()
        .map_err(|_| anyhow::anyhow!("Keychain adapter unavailable"))?;
    let _interaction = SecKeychain::disable_user_interaction()
        .map_err(|_| anyhow::anyhow!("Cannot disable interactive Keychain prompts"))?;
    let chain = if let Some(path) = path {
        SecKeychain::open(path)
    } else {
        SecKeychain::default()
    }
    .map_err(|_| anyhow::anyhow!("Selected Keychain is unavailable"))?;
    if let Some(value) = value {
        chain
            .set_generic_password(service, account, value.expose().as_bytes())
            .map_err(|_| {
                anyhow::anyhow!("Keychain rotation denied or locked; no fallback was written")
            })?;
    } else {
        let (_, item) = chain
            .find_generic_password(service, account)
            .map_err(|_| anyhow::anyhow!("Keychain entry missing or inaccessible"))?;
        item.delete();
        anyhow::ensure!(
            chain
                .find_generic_password(service, account)
                .is_err_and(|e| e.code() == -25300),
            "Keychain removal not confirmed; inspect the selected entry"
        );
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn keychain_mutation(
    _service: &str,
    _account: &str,
    _path: Option<&Path>,
    _value: Option<&SecretValue>,
) -> Result<()> {
    anyhow::bail!("Keychain mutations require macOS")
}
fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
impl SecretReferences {
    /// Protected reference files cannot be symlinks or accessible to other users.
    pub fn load(path: &Path) -> Result<Self> {
        let meta = std::fs::symlink_metadata(path).context("Read secret reference file")?;
        anyhow::ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "Secret references require a regular protected file"
        );
        anyhow::ensure!(meta.len() <= 65536, "Secret reference file exceeds limit");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            anyhow::ensure!(
                meta.permissions().mode() & 0o077 == 0,
                "Secret references require mode 0600"
            );
        }
        let config: Self = toml::from_str(&std::fs::read_to_string(path)?)
            .map_err(|_| anyhow::anyhow!("Invalid secret reference configuration"))?;
        anyhow::ensure!(
            config.version == 1 && config.references.len() <= 32,
            "Unsupported or oversized secret reference configuration"
        );
        for (name, reference) in &config.references {
            anyhow::ensure!(valid_name(name), "Invalid secret name");
            reference.validate()?;
        }
        Ok(config)
    }
}
impl SecretReference {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Environment { variable }=>anyhow::ensure!(valid_name(variable),"Invalid secret environment reference"),
            Self::Keychain {service,account,keychain}=>anyhow::ensure!(service.starts_with("Connect/") && service.len()<=256 && !account.is_empty() && account.len()<=256 && !service.contains('\0') && !account.contains('\0') && keychain.as_ref().is_none_or(|p|p.is_absolute()),"Keychain references require a Connect/ service, account and absolute optional keychain path"),
        }
        Ok(())
    }
}
impl SecretStore for NativeSecretStore {
    fn read(&self, reference: &SecretReference) -> Result<SecretValue> {
        reference.validate()?;
        match reference {
            SecretReference::Environment { variable } => {
                SecretValue::new(std::env::var(variable).map_err(|_| {
                    anyhow::anyhow!("Missing explicitly selected secret environment variable")
                })?)
            }
            SecretReference::Keychain {
                service,
                account,
                keychain,
            } => read_keychain(service, account, keychain.as_deref()),
        }
    }
}
#[cfg(target_os = "macos")]
fn read_keychain(service: &str, account: &str, path: Option<&Path>) -> Result<SecretValue> {
    use security_framework::os::macos::{keychain::SecKeychain, passwords::find_generic_password};
    let _lock = KEYCHAIN_ACCESS
        .lock()
        .map_err(|_| anyhow::anyhow!("Keychain adapter unavailable"))?;
    let _interaction = SecKeychain::disable_user_interaction()
        .map_err(|_| anyhow::anyhow!("Cannot disable interactive Keychain prompts"))?;
    let chain = path
        .map(SecKeychain::open)
        .transpose()
        .map_err(|_| anyhow::anyhow!("Keychain missing or inaccessible; unlock it explicitly"))?;
    let (value,_)=find_generic_password(chain.as_ref().map(std::slice::from_ref),service,account).map_err(|_|anyhow::anyhow!("Keychain secret missing, locked or denied; unlock or install the selected entry explicitly"))?;
    SecretValue::new(
        String::from_utf8(value.to_vec())
            .map_err(|_| anyhow::anyhow!("Secret encoding is invalid"))?,
    )
}
#[cfg(not(target_os = "macos"))]
fn read_keychain(_service: &str, _account: &str, _path: Option<&Path>) -> Result<SecretValue> {
    anyhow::bail!(
        "Keychain requires macOS; select an explicit environment reference for development/CI"
    )
}
/// Resolve one server secret. With a references file configured, absent references
/// mean absent capability; ambient provider variables never substitute for it.
pub fn optional(name: &str) -> Result<Option<SecretValue>> {
    let references = std::env::var_os("CONNECT_SECRET_REFERENCES")
        .map(|path| SecretReferences::load(Path::new(&path)))
        .transpose()?;
    optional_from_sources(
        name,
        references.as_ref(),
        |reference| NativeSecretStore.read(reference),
        |name| std::env::var(name).ok().map(SecretValue::new).transpose(),
    )
}
fn optional_from_sources(
    name: &str,
    references: Option<&SecretReferences>,
    read_reference: impl FnOnce(&SecretReference) -> Result<SecretValue>,
    read_environment: impl FnOnce(&str) -> Result<Option<SecretValue>>,
) -> Result<Option<SecretValue>> {
    let value = if let Some(refs) = references {
        refs.references.get(name).map(read_reference).transpose()
    } else {
        read_environment(name)
    }?;
    value
        .map(|value| {
            // Legacy provider constructors trim API keys. Preserve that boundary for
            // protected references too, without changing SIP or authority passwords.
            if matches!(name, "TELNYX_API_KEY" | "VAPI_PRIVATE_KEY") {
                SecretValue::new(value.expose().trim().to_owned())
            } else {
                Ok(value)
            }
        })
        .transpose()
}
pub fn resolve(name: &str) -> Result<SecretValue> {
    optional(name)?.context("Required server secret is not configured")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_api_key_whitespace_preserves_legacy_headers_and_password_bytes() {
        for name in [
            "TELNYX_API_KEY",
            "VAPI_PRIVATE_KEY",
            "CONNECT_SIP_PASSWORD",
            "CONNECT_SMS_AUTHORITY_TOKEN",
        ] {
            let value = optional_from_sources(
                name,
                None,
                |_| anyhow::bail!("No reference selected"),
                |_| Ok(Some(SecretValue::new("  disposable-fixture  ".into())?)),
            )
            .expect("environment key")
            .expect("selected value");
            assert_eq!(
                value.expose(),
                if matches!(name, "TELNYX_API_KEY" | "VAPI_PRIVATE_KEY") {
                    "disposable-fixture"
                } else {
                    "  disposable-fixture  "
                }
            );
        }
    }
    #[test]
    fn explicit_missing_or_locked_provider_reference_cannot_read_ambient_key() {
        let mut references = SecretReferences {
            version: 1,
            references: BTreeMap::new(),
        };
        let absent = optional_from_sources(
            "TELNYX_API_KEY",
            Some(&references),
            |_| anyhow::bail!("No reference selected"),
            |_| anyhow::bail!("Ambient lookup is forbidden"),
        )
        .expect("configured absence");
        assert!(absent.is_none());
        references.references.insert(
            "TELNYX_API_KEY".into(),
            SecretReference::Keychain {
                service: "Connect/fixture".into(),
                account: "fixture".into(),
                keychain: None,
            },
        );
        let ambient_read = std::cell::Cell::new(false);
        let locked = optional_from_sources(
            "TELNYX_API_KEY",
            Some(&references),
            |_| anyhow::bail!("Disposable vault locked"),
            |_| {
                ambient_read.set(true);
                anyhow::bail!("Ambient lookup is forbidden")
            },
        )
        .expect_err("selected locked reference must fail");
        assert_eq!(locked.to_string(), "Disposable vault locked");
        assert!(!ambient_read.get());
    }
    #[test]
    fn references_and_debug_never_contain_secret_values() {
        let value = SecretValue::new("private-fixture-token".into()).expect("secret");
        assert_eq!(format!("{value:?}"), "[REDACTED]");
        assert!(NativeSecretStore
            .read(&SecretReference::Environment {
                variable: "PATH".into()
            })
            .is_ok());
        assert!(NativeSecretStore
            .read(&SecretReference::Environment {
                variable: "invalid-value!".into()
            })
            .is_err());
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("references.toml");
        crate::cli::init::private_write(&path,b"version=1\n[references.VAPI_PRIVATE_KEY]\nstorage='keychain'\nservice='Connect/provider'\naccount='vapi'\n").expect("write");
        assert!(SecretReferences::load(&path).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
            assert!(SecretReferences::load(&path).is_err());
        }
    }
}
