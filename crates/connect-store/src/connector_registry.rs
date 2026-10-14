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
    /// Persist the full extension contract before worker recovery. An identical
    /// restart is allowed; capability/configuration drift requires an explicit
    /// migration/requalification rather than silently changing durable routes.
    pub fn register_connector_manifest(
        &self,
        descriptor: Descriptor,
        manifest: &connect_connector_api::manifest::Manifest,
    ) -> Result<()> {
        manifest
            .validate_descriptor(descriptor)
            .map_err(|_| Error::new("invalid-request", "Invalid connector manifest"))?;
        self.persist_extension_manifest(manifest, Some(descriptor))
    }
    /// Persist non-messaging extension contracts without creating message routes.
    pub fn register_extension_manifest(
        &self,
        manifest: &connect_connector_api::manifest::Manifest,
    ) -> Result<()> {
        manifest
            .validate()
            .map_err(|_| Error::new("invalid-request", "Invalid connector manifest"))?;
        self.persist_extension_manifest(manifest, None)
    }
    fn persist_extension_manifest(
        &self,
        manifest: &connect_connector_api::manifest::Manifest,
        descriptor: Option<Descriptor>,
    ) -> Result<()> {
        let encoded = serde_json::to_string(manifest).map_err(|_| Error::storage())?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        use rusqlite::OptionalExtension;
        let previous: Option<String> = tx
            .query_row(
                "SELECT manifest FROM connector_manifests WHERE name=?1",
                [&manifest.name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if previous.as_ref().is_some_and(|prior| prior != &encoded) {
            return Err(Error::new(
                "conflict",
                "Connector manifest differs from persisted contract",
            ));
        }
        if let Some(descriptor) = descriptor {
            self.register_connector(descriptor)?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO connector_manifests(name,manifest) VALUES(?1,?2)",
            rusqlite::params![manifest.name, encoded],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())
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
    /// Read declared operation support at trusted composition boundaries. This
    /// is capability validation, never principal authorization. Legacy drivers
    /// without a persisted manifest retain their existing dispatch behavior.
    pub fn require_connector_operation(
        &self,
        name: &str,
        operation: connect_connector_api::manifest::ConnectorOperation,
    ) -> Result<()> {
        let connection = self.connection()?;
        require_operation(&connection, name, operation)
    }
}

pub(crate) fn require_operation(
    connection: &rusqlite::Connection,
    name: &str,
    operation: connect_connector_api::manifest::ConnectorOperation,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let encoded: Option<String> = connection
        .query_row(
            "SELECT manifest FROM connector_manifests WHERE name=?1",
            [name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if let Some(encoded) = encoded {
        let manifest: connect_connector_api::manifest::Manifest =
            serde_json::from_str(&encoded).map_err(|_| Error::storage())?;
        manifest.validate().map_err(|_| Error::storage())?;
        if !manifest.operations.contains(&operation) {
            return Err(Error::new(
                "unsupported-connector-operation",
                "Connector operation is not declared",
            ));
        }
    }
    Ok(())
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
