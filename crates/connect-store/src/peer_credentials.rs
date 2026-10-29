//! Durable issuer metadata and administrator-bound verification keys.
use crate::{identity::actor_from_connection, now, Actor, Store};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::{provisioning::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub struct CredentialMetadata {
    pub actor: Actor,
    pub tenant_id: String,
    pub installation_id: String,
    pub issued_at: i64,
    pub user_authorization: Option<crate::UserAuthorization>,
}
impl std::fmt::Debug for CredentialMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialMetadata(redacted)")
    }
}
pub struct PeerSigningKey {
    pub identity_id: String,
    pub device_id: String,
    pub algorithm: SigningAlgorithm,
    pub public_key: Vec<u8>,
    pub not_before: i64,
    pub expires_at: i64,
}
impl std::fmt::Debug for PeerSigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PeerSigningKey(redacted)")
    }
}
impl Store {
    /// Identity-level cryptographic ownership only. The authenticated peer policy
    /// must separately authorize the exact credential-to-key binding.
    pub fn resolve_identity_signing_key(
        &self,
        identity: &str,
        installation: &str,
        key_id: &str,
        algorithm: &str,
    ) -> Result<PeerSigningKey> {
        let c = self.connection()?;
        c.query_row("SELECT k.identity_id,k.device_id,k.algorithm,k.public_key,k.not_before,k.expires_at FROM peer_signing_keys k JOIN credential_signing_keys b ON b.key_id=k.key_id JOIN credentials cr ON cr.id=b.credential_id AND cr.identity_id=k.identity_id JOIN identities i ON i.id=k.identity_id AND i.tenant_id=k.tenant_id JOIN devices d ON d.id=k.device_id AND d.identity_id=k.identity_id AND d.tenant_id=k.tenant_id CROSS JOIN installation n WHERE k.identity_id=?1 AND n.id=?2 AND n.singleton=1 AND k.key_id=?3 AND k.algorithm=?4 AND i.revoked=0 AND d.revoked=0 AND k.revoked=0 AND k.not_before<=?5 AND k.expires_at>?5 AND cr.revoked=0 AND cr.expires_at>?5 AND cr.issued_at>0 AND cr.issued_at<=?5 LIMIT 1",params![identity,installation,key_id,algorithm,now()],|r|Ok(PeerSigningKey {
            identity_id:r.get(0)?,device_id:r.get(1)?,algorithm:decode_algorithm(r.get(2)?)?,public_key:r.get(3)?,not_before:r.get(4)?,expires_at:r.get(5)?,
        })).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
    }
    /// Current issuer facts, never inferred from the time a token is validated.
    pub fn credential_metadata(&self, credential_id: &str) -> Result<CredentialMetadata> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        credential_metadata(&tx, credential_id)
    }

    /// One current lookup. The caller must retain a separate owned authority
    /// fence over verification/execution/delivery; this value is not that proof.
    pub fn resolve_peer_signing_key(
        &self,
        credential_id: &str,
        tenant: &str,
        key_id: &str,
        algorithm: &str,
    ) -> Result<PeerSigningKey> {
        let c = self.connection()?;
        peer_signing_key(&c, credential_id, tenant, key_id, algorithm)
    }
}

pub(crate) fn peer_signing_key(
    c: &Connection,
    credential_id: &str,
    tenant: &str,
    key_id: &str,
    algorithm: &str,
) -> Result<PeerSigningKey> {
    let different: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM client_enrollments WHERE credential_id=?1 AND key_id!=?2)",
            params![credential_id, key_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if different {
        return Err(Error::forbidden());
    }
    c.query_row("SELECT k.identity_id,k.device_id,k.algorithm,k.public_key,k.not_before,k.expires_at FROM peer_signing_keys k JOIN credential_signing_keys b ON b.key_id=k.key_id JOIN credentials cr ON cr.id=b.credential_id AND cr.identity_id=k.identity_id JOIN identities i ON i.id=k.identity_id AND i.tenant_id=k.tenant_id JOIN devices d ON d.id=k.device_id AND d.identity_id=k.identity_id AND d.tenant_id=k.tenant_id WHERE b.credential_id=?1 AND k.tenant_id=?2 AND k.key_id=?3 AND k.algorithm=?4 AND i.revoked=0 AND d.revoked=0 AND k.revoked=0 AND k.not_before<=?5 AND k.expires_at>?5 AND cr.revoked=0 AND cr.expires_at>?5 AND cr.issued_at>0 AND cr.issued_at<=?5",params![credential_id,tenant,key_id,algorithm,now()],|r|Ok(PeerSigningKey {
            identity_id:r.get(0)?,device_id:r.get(1)?,algorithm:decode_algorithm(r.get(2)?)?,public_key:r.get(3)?,not_before:r.get(4)?,expires_at:r.get(5)?,
        })).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}

pub(crate) fn credential_metadata(
    c: &Connection,
    credential_id: &str,
) -> Result<CredentialMetadata> {
    let actor = actor_from_connection(c, credential_id)?;
    crate::enrollment::credential_binding(c, credential_id)?;
    let (issued_at,tenant_id,installation_id):(i64,String,String)=c.prepare_cached("SELECT c.issued_at,i.tenant_id,n.id FROM credentials c JOIN identities i ON i.id=c.identity_id CROSS JOIN installation n WHERE c.id=?1 AND n.singleton=1").map_err(|_|Error::storage())?.query_row([credential_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    if issued_at <= 0 || issued_at > now() || issued_at >= actor.expires_at {
        return Err(Error::unauthorized());
    }
    Ok(CredentialMetadata {
        user_authorization: crate::user_delegation::revalidate(c, &actor, now())?,
        actor,
        tenant_id,
        installation_id,
        issued_at,
    })
}

pub(crate) fn key_outcome(c: &Connection, id: &str) -> Result<ProvisioningOutcome> {
    c.query_row("SELECT identity_id,device_id,algorithm,not_before,expires_at,revision,revoked FROM peer_signing_keys WHERE key_id=?1",[id],|r|Ok(ProvisioningOutcome::SigningKey {
        key_id:id.into(),identity_id:r.get(0)?,device_id:r.get(1)?,algorithm:decode_algorithm(r.get(2)?)?,not_before:r.get(3)?,expires_at:r.get(4)?,revision:r.get(5)?,revoked:r.get(6)?,
    })).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
pub(crate) fn device_outcome(c: &Connection, id: &str) -> Result<ProvisioningOutcome> {
    c.query_row(
        "SELECT identity_id,revision,revoked FROM devices WHERE id=?1",
        [id],
        |r| {
            Ok(ProvisioningOutcome::Device {
                device_id: id.into(),
                identity_id: r.get(0)?,
                revision: r.get(1)?,
                revoked: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}
pub(crate) fn apply(
    c: &Connection,
    actor: &Actor,
    tenant: &str,
    command: &ProvisioningCommand,
) -> Result<ProvisioningOutcome> {
    let conflict = || {
        Error::new(
            "conflict",
            "Signing key intent or revision conflicts with current state",
        )
    };
    match command {
        ProvisioningCommand::SigningKey {
            identity_id,
            credential_id,
            device_id,
            key_id,
            algorithm,
            public_key,
            not_before,
            expires_at,
            expected_revision,
            ..
        } => {
            super::provisioning::require_identity(c, actor, tenant, identity_id, true)?;
            let target = actor_from_connection(c, credential_id)?;
            let issued: i64 = c
                .query_row(
                    "SELECT issued_at FROM credentials WHERE id=?1",
                    [credential_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if issued <= 0 || issued > now() || issued >= target.expires_at {
                return Err(Error::unauthorized());
            }
            let current_device:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND identity_id=?2 AND tenant_id=?3 AND revoked=0)",params![device_id,identity_id,tenant],|r|r.get(0)).map_err(|_|Error::storage())?;
            if target.identity_id != *identity_id || !current_device {
                return Err(Error::forbidden());
            }
            if *expires_at <= now() || *expires_at > now() + 365 * 86400 {
                return Err(Error::new(
                    "invalid-request",
                    "Signing key expiry must be within one year",
                ));
            }
            let bytes = key_material(*algorithm, public_key)?;
            let existing:Option<(String,String,String,String,Vec<u8>,i64,i64,u64,bool)>=c.query_row("SELECT identity_id,tenant_id,device_id,algorithm,public_key,not_before,expires_at,revision,revoked FROM peer_signing_keys WHERE key_id=?1",[key_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|_|Error::storage())?;
            if let Some(row) = existing {
                if row.0 != *identity_id
                    || row.1 != tenant
                    || row.2 != *device_id
                    || row.3 != algorithm.as_str()
                    || row.4 != bytes
                    || row.5 != *not_before
                    || row.6 != *expires_at
                    || row.8
                    || Some(row.7) != *expected_revision
                {
                    return Err(conflict());
                }
            } else {
                if expected_revision.is_some() {
                    return Err(conflict());
                }
                let count: u64 = c
                    .query_row(
                        "SELECT COUNT(*) FROM peer_signing_keys WHERE device_id=?1",
                        [device_id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                if count >= 64 {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Signing key history capacity exceeded",
                    ));
                }
                c.execute("INSERT INTO peer_signing_keys(key_id,identity_id,tenant_id,device_id,algorithm,public_key,not_before,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![key_id,identity_id,tenant,device_id,algorithm.as_str(),bytes,not_before,expires_at]).map_err(|_|Error::storage())?;
            }
            let bound:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM credential_signing_keys WHERE credential_id=?1 AND key_id=?2)",params![credential_id,key_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !bound {
                let count: u64 = c
                    .query_row(
                        "SELECT COUNT(*) FROM credential_signing_keys WHERE key_id=?1",
                        [key_id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                if count >= 128 {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Signing credential history capacity exceeded",
                    ));
                }
                c.execute(
                    "INSERT INTO credential_signing_keys VALUES(?1,?2)",
                    params![credential_id, key_id],
                )
                .map_err(|_| Error::storage())?;
                if expected_revision.is_some() {
                    c.execute(
                        "UPDATE peer_signing_keys SET revision=revision+1 WHERE key_id=?1",
                        [key_id],
                    )
                    .map_err(|_| Error::storage())?;
                }
            }
            key_outcome(c, key_id)
        }
        ProvisioningCommand::RevokeSigningKey {
            key_id,
            expected_revision,
            ..
        } => {
            let outcome = key_outcome(c, key_id)?;
            let ProvisioningOutcome::SigningKey { identity_id, .. } = outcome else {
                return Err(Error::storage());
            };
            super::provisioning::require_identity(c, actor, tenant, &identity_id, false)?;
            if c.execute("UPDATE peer_signing_keys SET revoked=1,revision=revision+1 WHERE key_id=?1 AND revision=?2 AND revoked=0",params![key_id,expected_revision]).map_err(|_|Error::storage())?!=1 {return Err(conflict());}
            key_outcome(c, key_id)
        }
        ProvisioningCommand::RevokeDevice {
            device_id,
            expected_revision,
            ..
        } => {
            let ProvisioningOutcome::Device { identity_id, .. } = device_outcome(c, device_id)?
            else {
                return Err(Error::storage());
            };
            super::provisioning::require_identity(c, actor, tenant, &identity_id, false)?;
            if c.execute("UPDATE devices SET revoked=1,revision=revision+1 WHERE id=?1 AND revision=?2 AND revoked=0",params![device_id,expected_revision]).map_err(|_|Error::storage())?!=1 {return Err(conflict());}
            c.execute("UPDATE endpoints SET verification_state='revoked',secret_ref=NULL,revision=revision+1 WHERE device_id=?1 AND verification_state!='revoked'",[device_id]).map_err(|_|Error::storage())?;
            device_outcome(c, device_id)
        }
        _ => Err(Error::new(
            "invalid-request",
            "Not a peer credential command",
        )),
    }
}

fn decode_algorithm(value: String) -> rusqlite::Result<SigningAlgorithm> {
    match value.as_str() {
        "EdDSA" => Ok(SigningAlgorithm::EdDSA),
        "ES256" => Ok(SigningAlgorithm::ES256),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub(crate) fn key_material(algorithm: SigningAlgorithm, public_key: &str) -> Result<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD
        .decode(public_key)
        .map_err(|_| Error::new("invalid-request", "Invalid public key"))?;
    let valid = match algorithm {
        SigningAlgorithm::EdDSA => bytes
            .as_slice()
            .try_into()
            .ok()
            .and_then(|bytes| ed25519_dalek::VerifyingKey::from_bytes(bytes).ok())
            .is_some_and(|key| !key.is_weak()),
        SigningAlgorithm::ES256 => {
            bytes.len() == 65
                && bytes.first() == Some(&4)
                && p256::PublicKey::from_sec1_bytes(&bytes).is_ok()
        }
    };
    if !valid {
        return Err(Error::new("invalid-request", "Invalid public key"));
    }
    Ok(bytes)
}
