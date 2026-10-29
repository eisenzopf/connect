//! Original-ID remote enrollment; no alternate credential or endpoint authority.
use crate::{
    credential_issuance, identity::actor_from_connection_at, Actor, AuthorityGuardSet,
    AuthorityMode, AuthorityScope, AuthorityWait, Store,
};
use connect_protocol::{credential_issuance::*, enrollment::*, provisioning::*, Error, Result};
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
mod approval;
mod claim;
mod revoke;
#[cfg(test)]
mod tests;

/// Retain current Identity fences through protected physical delivery. Facts or
/// a borrowed response alone must never be adopted as a later authority grant.
pub struct OwnedEnrollment<T> {
    value: T,
    until: i64,
    _authority: Arc<AuthorityGuardSet>,
}
impl<T> std::fmt::Debug for OwnedEnrollment<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OwnedEnrollment(redacted)")
    }
}
impl<T> OwnedEnrollment<T> {
    pub fn current(&self) -> bool {
        crate::now() < self.until
    }
    pub fn expires_at(&self) -> i64 {
        self.until
    }
    pub fn outcome(&self) -> Result<&T> {
        if !self.current() {
            return Err(Error::unauthorized());
        }
        Ok(&self.value)
    }
}
/// Current binding facts only. Transport admission must retain its own fence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnrollmentCredentialBinding {
    pub enrollment_id: String,
    pub identity_id: String,
    pub device_id: String,
    pub key_id: String,
    pub endpoint_id: String,
    pub transport: EnrollmentTransport,
    pub browser_origin: Option<String>,
}
struct Row {
    id: String,
    tenant: String,
    issuer_identity: String,
    issuer: String,
    endpoint: String,
    request: EnrollmentRequest,
    digest: String,
    challenge_hash: String,
    state: String,
    attempts: u32,
    credential: Option<String>,
    issued_at: Option<i64>,
}
fn conflict() -> Error {
    Error::new(
        "conflict",
        "Enrollment original intent or resource conflicts",
    )
}
fn capacity() -> Error {
    Error::new("capacity-exceeded", "Enrollment capacity exhausted")
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn intent(issuer: &str, request: &EnrollmentRequest) -> Result<String> {
    crate::provisioning::canonical_json(
        &serde_json::json!({"issuer_credential_id":issuer,"request":request}),
    )
}
fn issue_request(request: &EnrollmentRequest) -> CredentialIssueRequest {
    CredentialIssueRequest {
        operation_id: request.completion_operation_id.clone(),
        identity_id: request.identity_id.clone(),
        operations: request.operations.clone(),
        recipients: request.recipients.clone(),
        expires_at: request.credential_expires_at,
    }
}
fn authorize(
    c: &Connection,
    issuer: &str,
    request: &EnrollmentRequest,
) -> Result<(Actor, String, Vec<AuthorityScope>)> {
    credential_issuance::require(c, issuer, &issue_request(request), crate::now())
}
fn secret(c: &Connection, domain: &[u8], values: &[&str]) -> Result<String> {
    let key: String = c
        .query_row(
            "SELECT cursor_key FROM installation WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(domain);
    for value in values {
        mac.update(value.as_bytes());
        mac.update(b"\0");
    }
    Ok(hex::encode(mac.finalize().into_bytes()))
}
fn load(c: &Connection, id: &str) -> Result<Row> {
    let (tenant,issuer_identity,issuer,endpoint,request_json,saved_digest,hash,state,attempts,credential,issued_at,identity,device,key,approve,complete,challenge_expires,credential_expires):(String,String,String,String,String,String,String,String,u32,Option<String>,Option<i64>,String,String,String,String,String,i64,i64)=c.query_row("SELECT tenant_id,issuer_identity_id,issuer_credential_id,endpoint_id,request_json,intent_digest,challenge_hash,state,attempts,credential_id,issued_at,identity_id,device_id,key_id,approve_operation_id,complete_operation_id,challenge_expires_at,credential_expires_at FROM client_enrollments WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?,r.get(15)?,r.get(16)?,r.get(17)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let mut request: EnrollmentRequest =
        serde_json::from_str(&request_json).map_err(|_| Error::storage())?;
    request.normalize()?;
    if request.identity_id != identity
        || request.device.id != device
        || request.key_id != key
        || request.operation_id != approve
        || request.completion_operation_id != complete
        || request.challenge_expires_at != challenge_expires
        || request.credential_expires_at != credential_expires
        || digest(&intent(&issuer, &request)?) != saved_digest
    {
        return Err(Error::forbidden());
    }
    Ok(Row {
        id: id.into(),
        tenant,
        issuer_identity,
        issuer,
        endpoint,
        request,
        digest: saved_digest,
        challenge_hash: hash,
        state,
        attempts,
        credential,
        issued_at,
    })
}
fn offer(c: &Connection, row: &Row) -> Result<EnrollmentOffer> {
    let challenge = secret(
        c,
        b"connect-client-enrollment/challenge/1\0",
        &[&row.id, &row.issuer, &row.digest],
    )?;
    if digest(&challenge) != row.challenge_hash {
        return Err(Error::forbidden());
    }
    let installation_id = c
        .query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())?;
    Ok(EnrollmentOffer {
        profile: connect_protocol::enrollment::PROFILE.into(),
        enrollment_id: row.id.clone(),
        endpoint_id: row.endpoint.clone(),
        installation_id,
        issuer_identity_id: row.issuer_identity.clone(),
        request: row.request.clone(),
        challenge,
    })
}
fn check_operation(c: &Connection, row: &Row, completed: bool) -> Result<()> {
    let (kind,state,tombstone,cancelled):(String,String,bool,bool)=c.query_row("SELECT command_type,state,tombstoned,cancel_requested FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3 AND digest=?4",params![row.tenant,row.issuer_identity,row.request.completion_operation_id,row.digest],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?.ok_or_else(conflict)?;
    if tombstone {
        return Err(Error::new("gone", "Enrollment receipt expired"));
    }
    if kind != "enrollment.complete"
        || cancelled
        || state != if completed { "completed" } else { "accepted" }
    {
        return Err(conflict());
    }
    Ok(())
}
fn endpoint_command(row: &Row) -> ProvisioningCommand {
    ProvisioningCommand::Endpoint {
        operation_id: row.request.completion_operation_id.clone(),
        identity_id: row.request.identity_id.clone(),
        device: row.request.device.clone(),
        transport: row.request.transport.endpoint_transport(),
        address: row.request.endpoint_address.clone(),
        capabilities: row.request.capabilities.clone(),
        secret_ref: None,
    }
}
fn endpoint_matches(c: &Connection, row: &Row) -> Result<bool> {
    let expected = &row.request;
    let data:Option<(String,String,String,String,String,String,String)>=c.query_row("SELECT identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state FROM endpoints WHERE id=?1",[&row.endpoint],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|_|Error::storage())?;
    match data {
        None => Ok(false),
        Some((identity, tenant, device, transport, address, capabilities, state)) => {
            if identity != expected.identity_id
                || tenant != row.tenant
                || device != expected.device.id
                || transport != expected.transport.endpoint_transport().as_str()
                || address != expected.endpoint_address
                || serde_json::from_str::<Vec<String>>(&capabilities)
                    .map_err(|_| Error::storage())?
                    != expected.capabilities
                || state == "revoked"
            {
                return Err(conflict());
            }
            Ok(true)
        }
    }
}
fn current_credential(c: &Connection, row: &Row) -> Result<EnrollmentCredential> {
    check_operation(c, row, true)?;
    if row.state != "completed" {
        return Err(Error::forbidden());
    }
    let id = row.credential.as_ref().ok_or_else(Error::storage)?;
    let issued_at = row.issued_at.ok_or_else(Error::storage)?;
    let actor = actor_from_connection_at(c, id, crate::now())?;
    let key = crate::peer_credentials::peer_signing_key(
        c,
        id,
        &row.tenant,
        &row.request.key_id,
        row.request.algorithm.as_str(),
    )?;
    if actor.identity_id != row.request.identity_id
        || actor.scope.operations != row.request.operations
        || actor.scope.recipients != row.request.recipients
        || actor.expires_at != row.request.credential_expires_at
        || key.device_id != row.request.device.id
        || key.public_key
            != crate::peer_credentials::key_material(
                row.request.algorithm,
                &row.request.public_key,
            )?
        || !endpoint_matches(c, row)?
    {
        return Err(Error::forbidden());
    }
    let request = issue_request(&row.request);
    let token = credential_issuance::token(
        c,
        id,
        &row.issuer,
        &request.operation_id,
        &intent(&row.issuer, &row.request)?,
    )?;
    let valid: bool = c
        .query_row(
            "SELECT verifier=?2 AND issued_at=?3 FROM credentials WHERE id=?1",
            params![id, crate::verifier(&token), issued_at],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !valid {
        return Err(Error::forbidden());
    }
    let installation_id = c
        .query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())?;
    Ok(EnrollmentCredential {
        enrollment_id: row.id.clone(),
        endpoint_id: row.endpoint.clone(),
        device_id: row.request.device.id.clone(),
        key_id: row.request.key_id.clone(),
        transport: row.request.transport,
        browser_origin: row.request.browser_origin.clone(),
        credential: CredentialIssueOutcome {
            operation_id: request.operation_id,
            credential_id: id.clone(),
            identity_id: actor.identity_id,
            issuer_identity_id: row.issuer_identity.clone(),
            issuer_credential_id: row.issuer.clone(),
            installation_id,
            operations: actor.scope.operations,
            recipients: actor.scope.recipients,
            issued_at,
            expires_at: actor.expires_at,
            credential: token,
        },
    })
}
fn journal(
    c: &Connection,
    actor: &Actor,
    tenant: &str,
    id: &str,
    kind: &str,
    intent: &str,
    state: &str,
    result: Option<String>,
    expires: i64,
) -> Result<()> {
    c.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,intent_json,state,result_json,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,?11)",params![format!("opr_{}",uuid::Uuid::new_v4()),tenant,actor.identity_id,id,digest(intent),kind,intent,state,result,crate::now(),expires]).map_err(|_|conflict())?;
    Ok(())
}
pub(crate) fn credential_binding(
    c: &Connection,
    credential: &str,
) -> Result<Option<EnrollmentCredentialBinding>> {
    let id: Option<String> = c
        .query_row(
            "SELECT id FROM client_enrollments WHERE credential_id=?1",
            [credential],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let Some(id) = id else {
        return Ok(None);
    };
    let row = load(c, &id)?;
    current_credential(c, &row)?;
    Ok(Some(EnrollmentCredentialBinding {
        enrollment_id: id,
        identity_id: row.request.identity_id,
        device_id: row.request.device.id,
        key_id: row.request.key_id,
        endpoint_id: row.endpoint,
        transport: row.request.transport,
        browser_origin: row.request.browser_origin,
    }))
}
impl Store {
    /// Current enrollment facts, not a future IO grant. Callers retain their
    /// normal current-authority guards through effects and protected delivery.
    pub fn enrollment_credential_binding(
        &self,
        credential: &str,
    ) -> Result<Option<EnrollmentCredentialBinding>> {
        let c = self.connection()?;
        credential_binding(&c, credential)
    }
    /// Check only the immutable enrollment Origin ceiling of an authenticated
    /// ordinary or explicit Task bearer. The caller still admits its own API,
    /// Task binding, scopes and current owned authority before any effect.
    pub fn check_enrollment_http_bearer(
        &self,
        token: &str,
        actual_origin: Option<&str>,
    ) -> Result<()> {
        let credential = self.enrollment_bearer_credential(token)?;
        self.check_enrollment_http_origin(&credential, actual_origin)
    }
    /// An enrollment cannot be adopted by an ingress with no trusted physical
    /// transport/Origin context. Explicit Task scope never bypasses this ceiling.
    pub fn check_unbound_enrollment_bearer(&self, token: &str) -> Result<()> {
        let credential = self.enrollment_bearer_credential(token)?;
        if self.enrollment_credential_binding(&credential)?.is_some() {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    fn enrollment_bearer_credential(&self, token: &str) -> Result<String> {
        let credential = if token.starts_with("ctg1.") {
            let invocation = self.authenticate_local_task_grant(token)?;
            self.connection()?
                .query_row(
                    "SELECT credential_id FROM task_grants WHERE id=?1",
                    [invocation.grant_id()],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|_| Error::unauthorized())?
        } else {
            self.authenticate(token)?.credential_id
        };
        Ok(credential)
    }
    /// Finite HTTP control is independent of the approved signaling transport,
    /// but must preserve the exact enrollment Origin (including native absence).
    /// Enrollment intent is immutable; this check never grants an operation.
    pub fn check_enrollment_http_origin(
        &self,
        credential: &str,
        actual_origin: Option<&str>,
    ) -> Result<()> {
        if let Some(binding) = self.enrollment_credential_binding(credential)? {
            if binding.browser_origin.as_deref() != actual_origin {
                return Err(Error::forbidden());
            }
        }
        Ok(())
    }
}
