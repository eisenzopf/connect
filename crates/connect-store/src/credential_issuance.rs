//! Journal-owned administrative issuance; no bearer appears in a stored receipt.
use crate::{
    identity::actor_from_connection_at, Actor, AuthorityGuardSet, AuthorityMode, AuthorityScope,
    AuthorityWait, Store,
};
use connect_protocol::{credential_issuance::*, Error, Result};
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

#[cfg(test)]
mod tests;

/// Current read fences exclude supported issuer/target/recipient revocation.
/// The protected HTTP transport retains this owner until physical response IO
/// ends. Trusted callers must not treat a borrowed result as a later grant.
pub struct IssuedScopedCredential {
    outcome: CredentialIssueOutcome,
    until: i64,
    _authority: Arc<AuthorityGuardSet>,
}
impl std::fmt::Debug for IssuedScopedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IssuedScopedCredential(redacted)")
    }
}
impl IssuedScopedCredential {
    pub fn current(&self) -> bool {
        crate::now() < self.until
    }
    pub fn expires_at(&self) -> i64 {
        self.until
    }
    pub fn outcome(&self) -> Result<&CredentialIssueOutcome> {
        if !self.current() {
            return Err(Error::unauthorized());
        }
        Ok(&self.outcome)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    credential_id: String,
    issuer_credential_id: String,
    issued_at: i64,
}
fn conflict() -> Error {
    Error::new(
        "conflict",
        "Credential issuance has different original authority or intent",
    )
}
pub(crate) fn token(
    c: &Connection,
    id: &str,
    issuer: &str,
    operation: &str,
    intent: &str,
) -> Result<String> {
    let secret: String = c
        .query_row(
            "SELECT cursor_key FROM installation WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(b"connect-administrative-credential/1\0");
    for value in [id, issuer, operation, intent] {
        mac.update(value.as_bytes());
        mac.update(b"\0");
    }
    Ok(hex::encode(mac.finalize().into_bytes()))
}
pub(crate) fn require(
    c: &Connection,
    issuer: &str,
    request: &CredentialIssueRequest,
    now: i64,
) -> Result<(Actor, String, Vec<AuthorityScope>)> {
    let actor = actor_from_connection_at(c, issuer, now)?;
    actor.require("admin")?;
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1 AND revoked=0",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    let mut scopes = vec![AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?];
    for identity in std::iter::once(&request.identity_id).chain(request.recipients.iter()) {
        crate::provisioning::require_identity(c, &actor, &tenant, identity, true)?;
        scopes.push(AuthorityScope::identity(identity, AuthorityMode::Read)?);
    }
    if request.expires_at <= now
        || request.expires_at > now.saturating_add(365 * 86400)
        || request.expires_at > actor.expires_at
        || request.operations.iter().any(|op| !actor.permits(op))
    {
        return Err(Error::forbidden());
    }
    Ok((actor, tenant, scopes))
}
/// Shared insertion after the caller rechecks current issuer and held scopes.
pub(crate) fn insert(
    c: &Connection,
    id: &str,
    request: &CredentialIssueRequest,
    credential: &str,
    now: i64,
) -> Result<()> {
    c.execute("INSERT INTO credentials(id,identity_id,verifier,operations,recipients,expires_at,issued_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,request.identity_id,crate::verifier(credential),serde_json::to_string(&request.operations).map_err(|_|Error::storage())?,serde_json::to_string(&request.recipients).map_err(|_|Error::storage())?,request.expires_at,now]).map_err(|_|Error::storage())?;
    Ok(())
}
impl Store {
    /// Current administrator only. Scope/expiry/issuer bind the original journal
    /// operation. Reordered set inputs normalize; changed authority conflicts.
    /// Revoked/expired issuers or children cannot recover a fresh credential.
    pub fn issue_scoped_credential(
        &self,
        issuer: &str,
        mut request: CredentialIssueRequest,
    ) -> Result<IssuedScopedCredential> {
        request.normalize()?;
        let authority =
            self.stable_authority(&AuthorityWait::new(Duration::from_secs(2)), || {
                let c = self.connection()?;
                Ok(require(&c, issuer, &request, crate::now())?.2)
            })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let now = crate::now();
        let (actor, tenant, scopes) = require(&tx, issuer, &request, now)?;
        if !self.authority_covers(&authority, &scopes) {
            return Err(Error::forbidden());
        }
        crate::operations::reject_control_id(&tx, &actor, &request.operation_id)?;
        let intent = crate::provisioning::canonical_json(&serde_json::json!({
            "identity_id":request.identity_id,"operations":request.operations,"recipients":request.recipients,
            "expires_at":request.expires_at,"issuer_credential_id":issuer}))?;
        let digest = hex::encode(Sha256::digest(intent.as_bytes()));
        let saved:Option<(String,String,String,Option<String>,bool)>=tx.query_row(
            "SELECT digest,command_type,state,result_json,tombstoned FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",
            params![tenant,actor.identity_id,request.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|Error::storage())?;
        let receipt = if let Some((old_digest, kind, state, result, tombstoned)) = saved {
            if old_digest != digest || kind != "credential.issue" {
                return Err(conflict());
            }
            if tombstoned {
                return Err(Error::new(
                    "gone",
                    "Credential issuance receipt has expired",
                ));
            }
            if state != "completed" {
                return Err(Error::new(
                    "reconciliation-required",
                    "Original issuance has no completed receipt",
                ));
            }
            let saved: Receipt = serde_json::from_str(&result.ok_or_else(conflict)?)
                .map_err(|_| Error::storage())?;
            if saved.issuer_credential_id != issuer {
                return Err(conflict());
            }
            saved
        } else {
            let count: u64 = tx
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if count >= self.limits.operation_receipts {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Credential issuance receipt capacity exhausted",
                ));
            }
            let receipt = Receipt {
                credential_id: uuid::Uuid::new_v4().to_string(),
                issuer_credential_id: issuer.into(),
                issued_at: now,
            };
            let credential = token(
                &tx,
                &receipt.credential_id,
                issuer,
                &request.operation_id,
                &intent,
            )?;
            insert(&tx, &receipt.credential_id, &request, &credential, now)?;
            tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,intent_json,state,result_json,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,'credential.issue',?6,'completed',?7,?8,?8,?9)",params![format!("opr_{}",uuid::Uuid::new_v4()),tenant,actor.identity_id,request.operation_id,digest,intent,serde_json::to_string(&receipt).map_err(|_|Error::storage())?,now,request.expires_at]).map_err(|_|Error::storage())?;
            receipt
        };
        let child = actor_from_connection_at(&tx, &receipt.credential_id, now)?;
        let (issued_at, verifier): (i64, String) = tx
            .query_row(
                "SELECT issued_at,verifier FROM credentials WHERE id=?1",
                [&receipt.credential_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| Error::storage())?;
        let credential = token(
            &tx,
            &receipt.credential_id,
            issuer,
            &request.operation_id,
            &intent,
        )?;
        if child.identity_id != request.identity_id
            || child.scope.operations != request.operations
            || child.scope.recipients != request.recipients
            || child.expires_at != request.expires_at
            || issued_at != receipt.issued_at
            || issued_at <= 0
            || issued_at > now
            || issued_at >= child.expires_at
            || verifier != crate::verifier(&credential)
        {
            return Err(Error::forbidden());
        }
        let installation_id: String = tx
            .query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
                r.get(0)
            })
            .map_err(|_| Error::storage())?;
        let until = actor.expires_at.min(child.expires_at);
        if until <= crate::now() {
            return Err(Error::unauthorized());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(IssuedScopedCredential {
            outcome: CredentialIssueOutcome {
                operation_id: request.operation_id,
                credential_id: receipt.credential_id,
                identity_id: request.identity_id,
                issuer_identity_id: actor.identity_id,
                issuer_credential_id: issuer.into(),
                installation_id,
                operations: request.operations,
                recipients: request.recipients,
                issued_at,
                expires_at: request.expires_at,
                credential,
            },
            until,
            _authority: authority,
        })
    }
}
