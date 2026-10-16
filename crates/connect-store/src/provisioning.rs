//! Current-authority administrative provisioning, serialized with durable receipts.
use crate::{identity::actor_from_connection, now, Actor, Store};
use connect_protocol::{provisioning::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

fn conflict() -> Error {
    Error::new(
        "conflict",
        "Provisioning intent or ownership conflicts with current state",
    )
}
// Canonicalize explicitly so another dependency enabling preserve_order on
// serde_json cannot change operation identity. This typed input has no floats.
fn canonical_json(value: &serde_json::Value) -> Result<String> {
    use serde_json::Value;
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let mut fields = Vec::with_capacity(keys.len());
            for key in keys {
                fields.push(format!(
                    "{}:{}",
                    serde_json::to_string(key).map_err(|_| Error::storage())?,
                    canonical_json(&object[key])?
                ));
            }
            Ok(format!("{{{}}}", fields.join(",")))
        }
        Value::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Result<Vec<_>>>()?
                .join(",")
        )),
        _ => serde_json::to_string(value).map_err(|_| Error::storage()),
    }
}
fn storage(error: rusqlite::Error) -> Error {
    match error {
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            conflict()
        }
        _ => Error::storage(),
    }
}
fn identity_tenant(c: &Connection, identity: &str, active: bool) -> Result<String> {
    c.query_row(
        "SELECT tenant_id FROM identities WHERE id=?1 AND (?2=0 OR revoked=0)",
        params![identity, active],
        |r| r.get(0),
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}
fn require_identity(
    c: &Connection,
    actor: &Actor,
    tenant: &str,
    id: &str,
    active: bool,
) -> Result<()> {
    if !actor.permits_recipient(id) || identity_tenant(c, id, active)? != tenant {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn endpoint(c: &Connection, id: &str) -> Result<ProvisioningOutcome> {
    let row: (String,String,String,String,String,u64) = c.query_row("SELECT identity_id,device_id,transport,capabilities_json,verification_state,revision FROM endpoints WHERE id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
        .optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    Ok(ProvisioningOutcome::Endpoint {
        endpoint_id: id.into(),
        identity_id: row.0,
        device_id: row.1,
        transport: serde_json::from_value(serde_json::Value::String(row.2))
            .map_err(|_| Error::storage())?,
        capabilities: serde_json::from_str(&row.3).map_err(|_| Error::storage())?,
        verification_state: row.4,
        revision: row.5,
    })
}
fn identity(c: &Connection, id: &str) -> Result<ProvisioningOutcome> {
    let (alias, kind, revision, revoked): (String, String, u64, bool) = c
        .query_row(
            "SELECT alias,kind,revision,revoked FROM identities WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    Ok(ProvisioningOutcome::Identity {
        identity_id: id.into(),
        alias,
        identity_kind: serde_json::from_value(serde_json::Value::String(kind))
            .map_err(|_| Error::storage())?,
        revision,
        revoked,
    })
}
fn authorize_outcome(
    c: &Connection,
    actor: &Actor,
    tenant: &str,
    value: ProvisioningOutcome,
) -> Result<ProvisioningOutcome> {
    let id = match &value {
        ProvisioningOutcome::Identity { identity_id, .. }
        | ProvisioningOutcome::Endpoint { identity_id, .. }
        | ProvisioningOutcome::Participant { identity_id, .. } => identity_id,
    };
    require_identity(c, actor, tenant, id, false)?;
    match value {
        ProvisioningOutcome::Identity { identity_id, .. } => identity(c, &identity_id),
        ProvisioningOutcome::Endpoint { endpoint_id, .. } => endpoint(c, &endpoint_id),
        value @ ProvisioningOutcome::Participant { .. } => Ok(value),
    }
}
impl Store {
    /// No network calls or plaintext secrets. Same current principal/operation
    /// and canonical intent returns the existing resource without duplicating it.
    /// Administration never implicitly grants the operator content access.
    pub fn execute_provisioning(
        &self,
        credential_id: &str,
        mut command: ProvisioningCommand,
    ) -> Result<ProvisioningOutcome> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential_id)?;
        actor.require("admin")?;
        let tenant = identity_tenant(&tx, &actor.identity_id, true)?;
        command.normalize()?;
        // This typed contract contains strings, arrays and safe integers only.
        let mut intent = serde_json::to_value(&command).map_err(|_| Error::storage())?;
        intent
            .as_object_mut()
            .ok_or_else(Error::storage)?
            .remove("operation_id");
        let intent = canonical_json(&intent)?;
        let digest = hex::encode(Sha256::digest(intent.as_bytes()));
        let saved: Option<(String,String,Option<String>,bool)> = tx.query_row("SELECT digest,command_type,result_json,tombstoned FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,command.operation_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
            .optional().map_err(|_|Error::storage())?;
        if let Some((old_digest, kind, result, tombstoned)) = saved {
            if old_digest != digest || kind != command.command_type() {
                return Err(conflict());
            }
            if tombstoned {
                return Err(Error::new("gone", "Provisioning receipt has expired"));
            }
            let result = result.ok_or_else(conflict)?;
            let value = serde_json::from_str(&result).map_err(|_| Error::storage())?;
            return authorize_outcome(&tx, &actor, &tenant, value);
        }
        let count:u64=tx.query_row("SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM requests)+(SELECT COUNT(*) FROM task_requests)",[],|r|r.get(0)).map_err(|_|Error::storage())?;
        if count >= self.limits.operation_receipts {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable request receipt capacity exceeded",
            ));
        }
        let outcome = apply(&tx, &actor, &tenant, &command)?;
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,intent_json,state,result_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'completed',?8,?9,?9)",params![format!("opr_{}",uuid::Uuid::new_v4()),tenant,actor.identity_id,command.operation_id(),digest,command.command_type(),intent,serde_json::to_string(&outcome).map_err(|_|Error::storage())?,now()]).map_err(storage)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(outcome)
    }
}
fn apply(
    c: &Connection,
    actor: &Actor,
    tenant: &str,
    command: &ProvisioningCommand,
) -> Result<ProvisioningOutcome> {
    match command {
        ProvisioningCommand::Identity {
            alias,
            kind,
            display_name,
            ..
        } => {
            // The selected ID is generated inside this transaction, not supplied
            // by an untrusted client or silently adopted from an existing alias.
            let id = format!("id_{}", uuid::Uuid::new_v4());
            c.execute("INSERT INTO identities(id,alias,capabilities,tenant_id,kind,display_name) VALUES(?1,?2,'[\"messaging\"]',?3,?4,?5)",params![id,alias,tenant,kind.as_str(),display_name]).map_err(storage)?;
            // Scoped administrators cannot manufacture a recipient outside their
            // permitted recipient set; new-identity provisioning needs wildcard.
            if !actor.permits_recipient(&id) {
                return Err(Error::forbidden());
            }
            identity(c, &id)
        }
        ProvisioningCommand::Participant {
            conversation_id,
            identity_id,
            role,
            ..
        } => {
            require_identity(c, actor, tenant, identity_id, true)?;
            let permitted:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND tenant_id=?2 AND closed=0)",params![conversation_id,tenant],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !permitted {
                return Err(Error::forbidden());
            }
            let id = format!("part_{}", uuid::Uuid::new_v4());
            c.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,tenant_id,kind,role) SELECT ?1,id,?2,tenant_id,kind,?3 FROM identities WHERE id=?4",params![conversation_id,id,role,identity_id]).map_err(storage)?;
            Ok(ProvisioningOutcome::Participant {
                participant_id: id,
                conversation_id: conversation_id.clone(),
                identity_id: identity_id.clone(),
                membership_epoch: 1,
            })
        }
        ProvisioningCommand::Endpoint {
            identity_id,
            device,
            transport,
            address,
            capabilities,
            secret_ref,
            ..
        } => {
            require_identity(c, actor, tenant, identity_id, true)?;
            let previous: Option<(String, String, String, bool)> = c
                .query_row(
                    "SELECT identity_id,kind,platform,revoked FROM devices WHERE id=?1",
                    [&device.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
                .map_err(|_| Error::storage())?;
            if let Some((owner, kind, platform, revoked)) = previous {
                if owner != *identity_id
                    || kind != device.kind
                    || platform != device.platform
                    || revoked
                {
                    return Err(conflict());
                }
            } else {
                c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES(?1,?2,?3,?4,?5)",params![device.id,identity_id,tenant,device.kind,device.platform]).map_err(storage)?;
            }
            let id = format!("ep_{}", uuid::Uuid::new_v4());
            c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,secret_ref,route_owner_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,identity_id,tenant,device.id,transport.as_str(),address,serde_json::to_string(capabilities).map_err(|_|Error::storage())?,secret_ref,actor.identity_id]).map_err(storage)?;
            endpoint(c, &id)
        }
        ProvisioningCommand::VerifyEndpoint {
            endpoint_id,
            expected_revision,
            evidence_ref,
            ..
        } => {
            authorize_endpoint(c, actor, tenant, endpoint_id)?;
            let changed=c.execute("UPDATE endpoints SET verification_state='verified',verification_evidence=?2,verified_at=?3,revision=revision+1 WHERE id=?1 AND revision=?4 AND verification_state!='revoked' AND EXISTS(SELECT 1 FROM identities i WHERE i.id=endpoints.identity_id AND i.revoked=0) AND EXISTS(SELECT 1 FROM devices d WHERE d.id=endpoints.device_id AND d.revoked=0)",params![endpoint_id,evidence_ref,now(),expected_revision]).map_err(storage)?;
            if changed != 1 {
                return Err(conflict());
            }
            endpoint(c, endpoint_id)
        }
        ProvisioningCommand::RevokeEndpoint {
            endpoint_id,
            expected_revision,
            ..
        } => {
            authorize_endpoint(c, actor, tenant, endpoint_id)?;
            let changed=c.execute("UPDATE endpoints SET verification_state='revoked',secret_ref=NULL,revision=revision+1 WHERE id=?1 AND revision=?2",params![endpoint_id,expected_revision]).map_err(storage)?;
            if changed != 1 {
                return Err(conflict());
            }
            endpoint(c, endpoint_id)
        }
        ProvisioningCommand::RevokeIdentity {
            identity_id,
            expected_revision,
            ..
        } => {
            require_identity(c, actor, tenant, identity_id, false)?;
            if c.execute(
                "UPDATE identities SET revoked=1,revision=revision+1 WHERE id=?1 AND revision=?2",
                params![identity_id, expected_revision],
            )
            .map_err(storage)?
                != 1
            {
                return Err(conflict());
            }
            c.execute(
                "UPDATE credentials SET revoked=1 WHERE identity_id=?1",
                [identity_id],
            )
            .map_err(storage)?;
            c.execute(
                "UPDATE devices SET revoked=1 WHERE identity_id=?1",
                [identity_id],
            )
            .map_err(storage)?;
            c.execute("UPDATE endpoints SET verification_state='revoked',secret_ref=NULL,revision=revision+1 WHERE identity_id=?1 AND verification_state!='revoked'",[identity_id]).map_err(storage)?;
            identity(c, identity_id)
        }
    }
}
fn authorize_endpoint(c: &Connection, actor: &Actor, tenant: &str, id: &str) -> Result<()> {
    let owner: Option<String> = c
        .query_row(
            "SELECT identity_id FROM endpoints WHERE id=?1 AND tenant_id=?2",
            params![id, tenant],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    require_identity(
        c,
        actor,
        tenant,
        &owner.ok_or_else(Error::forbidden)?,
        false,
    )
}
