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
    pub fn messaging(recipients: Vec<String>) -> Self {
        Self {
            operations: [
                "resolve",
                "create",
                "send",
                "history",
                "inbox",
                "request_status",
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
        use connect_protocol::Outcome;
        use sha2::{Digest, Sha256};
        if request_id.is_empty()
            || request_id.len() > 128
            || !request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid identity creation request ID",
            ));
        }
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, &actor.credential_id)?;
        actor.require("admin")?;
        let intent = format!(
            "identity-create/1:{}",
            hex::encode(Sha256::digest(alias.as_bytes()))
        );
        let saved: Option<(String, String, String)> = tx
            .query_row(
                "SELECT digest,operation,result FROM requests WHERE actor=?1 AND id=?2",
                params![actor.identity_id, request_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some((digest, operation, result)) = saved {
            if digest != intent || operation != "admin" {
                return Err(Error::new(
                    "conflict",
                    "Logical request ID already has different inputs",
                ));
            }
            let outcome: Result<Outcome> =
                serde_json::from_str(&result).map_err(|_| Error::storage())?;
            return match outcome? {
                Outcome::Identity(identity) => Ok(identity),
                _ => Err(Error::storage()),
            };
        }
        let receipts: u64 = tx
            .query_row(
                "SELECT (SELECT COUNT(*) FROM requests)+(SELECT COUNT(*) FROM task_requests)",
                [],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !crate::tasks::budget::fits(&tx, self.limits.operation_receipts, receipts, 1)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable request receipt capacity exceeded",
            ));
        }
        let identity = create_identity_on_connection(&tx, alias)?;
        let result: Result<Outcome> = Ok(Outcome::Identity(identity.clone()));
        tx.execute(
            "INSERT INTO requests VALUES(?1,?2,?3,'admin','[]',?4)",
            params![
                actor.identity_id,
                request_id,
                intent,
                serde_json::to_string(&result).map_err(|_| Error::storage())?
            ],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(identity)
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
    connection.query_row("SELECT id,identity_id,operations,recipients,expires_at FROM credentials WHERE id=?1 AND revoked=0 AND expires_at>?2 AND EXISTS(SELECT 1 FROM identities i WHERE i.id=credentials.identity_id AND i.revoked=0)",params![credential_id,timestamp],|row| {
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
    connection.execute("INSERT INTO credentials(id,identity_id,verifier,operations,recipients,expires_at) VALUES(?1,?2,?3,?4,?5,?6)",params![issued.id,identity_id,verifier(&issued.token),serde_json::to_string(&scope.operations).map_err(|_|Error::storage())?,serde_json::to_string(&scope.recipients).map_err(|_|Error::storage())?,expires_at]).map_err(|_|Error::storage())?;
    Ok(issued)
}
