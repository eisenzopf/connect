//! Trusted composition descriptors; the store applies generic durable policy.
use connect_connector_api::{AddressKind, Descriptor};
use connect_protocol::{Error, Result};
use std::{collections::BTreeMap, sync::RwLock};

pub(crate) struct Registry(RwLock<BTreeMap<&'static str, Descriptor>>);
impl Default for Registry {
    fn default() -> Self {
        Self(RwLock::new(
            [connect_connector_api::TELNYX, connect_connector_api::VAPI]
                .into_iter()
                .map(|d| (d.name, d))
                .collect(),
        ))
    }
}
impl crate::Store {
    /// Register a connector contract from trusted Rust composition, never from
    /// an application request. Re-registration must preserve its exact policy.
    pub fn register_connector(&self, descriptor: Descriptor) -> Result<()> {
        if descriptor.name.is_empty()
            || descriptor.name.len() > 64
            || !descriptor
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'-')
            || (descriptor.sms && descriptor.address != AddressKind::E164Pair)
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid connector descriptor",
            ));
        }
        let mut registry = self.connectors.0.write().map_err(|_| Error::storage())?;
        if registry
            .get(descriptor.name)
            .is_some_and(|prior| prior != &descriptor)
        {
            return Err(Error::new(
                "conflict",
                "Connector contract is already registered",
            ));
        }
        registry.insert(descriptor.name, descriptor);
        Ok(())
    }
    pub(crate) fn connector(&self, name: &str) -> Result<Descriptor> {
        self.connectors
            .0
            .read()
            .map_err(|_| Error::storage())?
            .get(name)
            .copied()
            .ok_or_else(|| Error::new("unavailable", "Connector contract is not registered"))
    }
}

pub(crate) fn validate(descriptor: Descriptor, target: &str, source: Option<&str>) -> Result<()> {
    let e164 = |s: &str| {
        s.starts_with('+')
            && (8..=16).contains(&s.len())
            && s[1..].bytes().all(|b| b.is_ascii_digit())
    };
    let valid = match descriptor.address {
        AddressKind::Opaque => {
            !target.is_empty()
                && !target.chars().any(char::is_control)
                && source.is_none_or(|s| !s.chars().any(char::is_control))
        }
        AddressKind::Uuid => uuid::Uuid::parse_str(target).is_ok() && source.is_none(),
        AddressKind::E164Pair => e164(target) && source.is_some_and(e164),
    };
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            "invalid-request",
            "Route address does not match connector contract",
        ))
    }
}
