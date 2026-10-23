use crate::{now, random_secret, verifier, Store};
use connect_protocol::{Error, Identity, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

// Messaging, voice/session control and task ownership need more than 32
// explicit permissions together. Keep issuance bounded without requiring '*'.
const MAX_CREDENTIAL_OPERATIONS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub operations: Vec<String>,
    pub recipients: Vec<String>,
}
impl Scope {
    pub fn operator() -> Self {
        Self {
            operations: vec!["*".into()],
            recipients: vec!["*".into()],
        }
    }
    /// Explicit voice grant; existing messaging-only credentials remain unchanged.
    pub fn communications(recipients: Vec<String>) -> Self {
        let mut scope = Self::messaging(recipients);
        scope.operations.extend(
            [
                "call_start",
                "call_status",
                "call_end",
                "session_invite",
                "session_accept",
                "session_reject",
                "connection.offer",
                "connection.update",
                "capability.advertise",
                "session.invite",
                "session.accept",
                "session.update",
                "session.end",
                "session.reject",
                "session.cancel",
                "session_cancel",
                "session_end",
                "session_get",
                "sip_dial",
                "sip_status",
                "sip_end",
            ]
            .map(str::to_owned),
        );
        scope
    }
    /// Ordinary participant/agent authority; no task grant or role promotion.
    pub fn collaboration(recipients: Vec<String>) -> Self {
        let mut scope = Self::communications(recipients);
        scope.operations.extend(
            [
                "conversation.get",
                "conversation.list",
                "conversation.subscribe",
                "session.observation",
                "record_get",
                "conversation.participant.add",
                "conversation.participant.update",
                "conversation.participant.remove",
                "conversation.history.share",
            ]
            .map(str::to_owned),
        );
        scope
    }
    pub fn messaging(recipients: Vec<String>) -> Self {
        Self {
            operations: [
                "resolve",
                "create",
                "conversation.list",
                "send",
                "history",
                "inbox",
                "request_status",
                "operation.get",
                "close",
                "health",
                "provider_status",
                "context_view",
                "availability_set",
                "availability_get",
                "artifact_begin",
                "artifact_append",
                "artifact_finish",
                "artifact_get",
                "artifact_delete",
            ]
            .iter()
            .map(|v| v.to_string())
            .collect(),
            recipients,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Actor {
    pub credential_id: String,
    pub identity_id: String,
    pub scope: Scope,
    pub expires_at: i64,
}
impl Actor {
    pub fn permits(&self, op: &str) -> bool {
        self.scope.operations.iter().any(|v| v == "*" || v == op)
    }
    pub fn permits_recipient(&self, id: &str) -> bool {
        id == self.identity_id || self.scope.recipients.iter().any(|v| v == "*" || v == id)
    }
    pub fn require(&self, op: &str) -> Result<()> {
        if self.permits(op) {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
}
/// Credential plaintext is returned only at explicit issuance and never Debug-printed.
pub struct IssuedCredential {
    pub id: String,
    pub token: String,
}
impl Store {
    pub fn create_identity(&self, alias: &str) -> Result<Identity> {
        let connection = self.connection()?;
        create_identity_on_connection(&connection, alias)
    }
    /// Protected local provisioning with one atomic creation receipt. Current
    /// administrator authority is rechecked before replay; an existing alias
    /// without this exact actor/request intent is never silently adopted.
    pub fn create_identity_as(
        &self,
        actor: &Actor,
        request_id: &str,
        alias: &str,
    ) -> Result<Identity> {
        use connect_protocol::provisioning::{
            IdentityKind, ProvisioningCommand, ProvisioningOutcome,
        };
        match self.execute_provisioning(
            &actor.credential_id,
            ProvisioningCommand::Identity {
                operation_id: request_id.into(),
                alias: alias.into(),
                kind: IdentityKind::Human,
                display_name: None,
            },
        )? {
            ProvisioningOutcome::Identity { identity_id, .. } => self.identity(&identity_id),
            _ => Err(Error::storage()),
        }
    }
    pub fn identity(&self, id: &str) -> Result<Identity> {
        self.connection()?
            .query_row(
                "SELECT id,alias,capabilities FROM identities WHERE id=?1",
                [id],
                identity_row,
            )
            .optional()
            .map_err(|_| Error::storage())?
            .ok_or_else(Error::forbidden)
    }
    pub fn resolve(&self, actor: &Actor, address: &str) -> Result<Identity> {
        let actor = self.revalidate(&actor.credential_id)?;
        actor.require("resolve")?;
        let alias = address
            .strip_prefix("connect:")
            .and_then(|a| a.strip_suffix("@local"))
            .ok_or_else(|| Error::new("invalid-request", "Use connect:alias@local"))?;
        let identity = self
            .connection()?
            .query_row(
                "SELECT id,alias,capabilities FROM identities WHERE alias=?1",
                [alias],
                identity_row,
            )
            .optional()
            .map_err(|_| Error::storage())?
            .ok_or_else(Error::forbidden)?;
        if !actor.permits_recipient(&identity.id) {
            return Err(Error::forbidden());
        };
        Ok(identity)
    }
    pub fn issue(
        &self,
        identity_id: &str,
        scope: Scope,
        expires_at: i64,
    ) -> Result<IssuedCredential> {
        let _authority = self.write_identity_authority(identity_id)?;
        let connection = self.connection()?;
        issue_on_connection(&connection, identity_id, scope, expires_at)
    }
    /// Local administration validates current authority inside the issuance transaction.
    pub fn issue_as(
        &self,
        actor: &Actor,
        identity_id: &str,
        scope: Scope,
        expires_at: i64,
    ) -> Result<IssuedCredential> {
        let _authority = self.write_identity_authority(identity_id)?;
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        actor_from_connection(&tx, &actor.credential_id)?.require("admin")?;
        let credential = issue_on_connection(&tx, identity_id, scope, expires_at)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(credential)
    }

    pub fn authenticate(&self, token: &str) -> Result<Actor> {
        if token.len() != 64 {
            return Err(Error::unauthorized());
        }
        let id: Option<String> = self
            .connection()?
            .query_row(
                "SELECT id FROM credentials WHERE verifier=?1",
                [verifier(token)],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        self.revalidate(&id.ok_or_else(Error::unauthorized)?)
    }
    /// Inspect an existing installation credential without migrations, cleanup,
    /// journaling or artifact mutations. Intended for protected local installers;
    /// runtime operations must still revalidate inside their admission transaction.
    pub fn authenticate_file(path: &std::path::Path, token: &str) -> Result<Actor> {
        if token.len() != 64 {
            return Err(Error::unauthorized());
        }
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| Error::storage())?;
        let id: Option<String> = connection
            .query_row(
                "SELECT id FROM credentials WHERE verifier=?1",
                [verifier(token)],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        actor_from_connection(&connection, &id.ok_or_else(Error::unauthorized)?)
    }
    /// Read-only current operator admission for installation-owned secret storage.
    /// No migration, cleanup, credential issue or journal mutation is performed.
    pub fn authorize_secret_storage_file(path: &std::path::Path, token: &str) -> Result<String> {
        if token.len() != 64 {
            return Err(Error::unauthorized());
        }
        let mut connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| Error::storage())?;
        let tx = connection.transaction().map_err(|_| Error::storage())?;
        let id: Option<String> = tx
            .query_row(
                "SELECT id FROM credentials WHERE verifier=?1",
                [verifier(token)],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        actor_from_connection(&tx, &id.ok_or_else(Error::unauthorized)?)?.require("admin")?;
        tx.query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())
    }
    pub fn revalidate(&self, credential_id: &str) -> Result<Actor> {
        let connection = self.connection()?;
        actor_from_connection(&connection, credential_id)
    }
    pub fn revoke(&self, actor: &Actor, id: &str) -> Result<()> {
        let owner = self.credential_owner(id)?;
        let _authority = self.stable_authority(
            &crate::AuthorityWait::new(std::time::Duration::from_secs(2)),
            || {
                let c = self.connection()?;
                crate::user_delegation::write_scopes(&c, &owner)
            },
        )?;
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        actor_from_connection(&tx, &actor.credential_id)?.require("admin")?;
        let changed = tx
            .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [id])
            .map_err(|_| Error::storage())?;
        if changed != 1 {
            return Err(Error::forbidden());
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(())
    }
}
fn create_identity_on_connection(
    connection: &rusqlite::Connection,
    alias: &str,
) -> Result<Identity> {
    if alias.is_empty()
        || alias.len() > 64
        || !alias
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(Error::new(
            "invalid-request",
            "Alias must be 1–64 lowercase ASCII letters, digits or hyphens",
        ));
    }
    let identity = Identity {
        id: uuid::Uuid::new_v4().to_string(),
        alias: alias.into(),
        capabilities: vec!["messaging".into()],
    };
    match connection.execute(
        "INSERT INTO identities(id,alias,capabilities) VALUES(?1,?2,?3)",
        params![
            identity.id,
            alias,
            serde_json::to_string(&identity.capabilities).map_err(|_| Error::storage())?
        ],
    ) {
        Ok(_) => Ok(identity),
        Err(rusqlite::Error::SqliteFailure(code, _))
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(Error::new("conflict", "Alias already exists"))
        }
        Err(_) => Err(Error::storage()),
    }
}
fn identity_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Identity> {
    let capabilities: String = row.get(2)?;
    Ok(Identity {
        id: row.get(0)?,
        alias: row.get(1)?,
        capabilities: serde_json::from_str(&capabilities)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

pub(crate) fn actor_from_connection(
    connection: &rusqlite::Connection,
    credential_id: &str,
) -> Result<Actor> {
    actor_from_connection_at(connection, credential_id, now())
}

pub(crate) fn actor_from_connection_at(
    connection: &rusqlite::Connection,
    credential_id: &str,
    timestamp: i64,
) -> Result<Actor> {
    let actor = base_actor_from_connection_at(connection, credential_id, timestamp)?;
    crate::user_delegation::revalidate(connection, &actor, timestamp)?;
    Ok(actor)
}
pub(crate) fn base_actor_from_connection_at(
    connection: &rusqlite::Connection,
    credential_id: &str,
    timestamp: i64,
) -> Result<Actor> {
    connection.prepare_cached("SELECT id,identity_id,operations,recipients,expires_at FROM credentials WHERE id=?1 AND revoked=0 AND expires_at>?2 AND EXISTS(SELECT 1 FROM identities i WHERE i.id=credentials.identity_id AND i.revoked=0)").map_err(|_|Error::storage())?.query_row(params![credential_id,timestamp],|row| {
            let ops:String=row.get(2)?;let recipients:String=row.get(3)?;
            Ok(Actor { credential_id:row.get(0)?,identity_id:row.get(1)?,scope:Scope { operations:serde_json::from_str(&ops).map_err(|_|rusqlite::Error::InvalidQuery)?,recipients:serde_json::from_str(&recipients).map_err(|_|rusqlite::Error::InvalidQuery)? },expires_at:row.get(4)? })
        }).optional().map_err(|_|Error::storage())?.ok_or_else(Error::unauthorized)
}

fn issue_on_connection(
    connection: &rusqlite::Connection,
    identity_id: &str,
    scope: Scope,
    expires_at: i64,
) -> Result<IssuedCredential> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1 AND revoked=0)",
            [identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !exists {
        return Err(Error::forbidden());
    }
    if expires_at <= now() || expires_at > now() + 365 * 24 * 60 * 60 {
        return Err(Error::new(
            "invalid-request",
            "Credential expiry must be within the next 365 days",
        ));
    }
    if scope.operations.is_empty()
        || scope.operations.len() > MAX_CREDENTIAL_OPERATIONS
        || scope.recipients.len() > 128
    {
        return Err(Error::new("invalid-request", "Invalid credential scope"));
    }
    let issued = IssuedCredential {
        id: uuid::Uuid::new_v4().to_string(),
        token: random_secret(),
    };
    connection.execute("INSERT INTO credentials(id,identity_id,verifier,operations,recipients,expires_at,issued_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![issued.id,identity_id,verifier(&issued.token),serde_json::to_string(&scope.operations).map_err(|_|Error::storage())?,serde_json::to_string(&scope.recipients).map_err(|_|Error::storage())?,expires_at,now()]).map_err(|_|Error::storage())?;
    Ok(issued)
}
