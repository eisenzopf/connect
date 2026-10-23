//! Durable explicit human consent with the issuer's current authority ceiling.
use crate::{
    identity::actor_from_connection_at, Actor, AuthorityMode, AuthorityScope, AuthorityWait,
    IssuedCredential, Store,
};
use connect_protocol::{user_delegation::UserDelegationRequest, Error, Result};
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::Sha256;

#[derive(Clone, PartialEq, Eq)]
pub struct UserAuthorization {
    pub user_id: String,
    pub scopes: Vec<String>,
}
impl std::fmt::Debug for UserAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserAuthorization(redacted)")
    }
}
pub struct IssuedUserDelegation {
    pub credential: IssuedCredential,
    pub identity_id: String,
    pub authorization: UserAuthorization,
    pub expires_at: i64,
}
impl std::fmt::Debug for IssuedUserDelegation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IssuedUserDelegation(redacted)")
    }
}
fn token(c: &Connection, id: &str, issuer: &str, intent: &str) -> Result<String> {
    let key: String = c
        .query_row(
            "SELECT cursor_key FROM installation WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(b"connect-user-delegation/1\0");
    for field in [id, issuer, intent] {
        mac.update(field.as_bytes());
        mac.update(b"\0");
    }
    Ok(hex::encode(mac.finalize().into_bytes()))
}
fn require_issuer(
    c: &Connection,
    actor: &Actor,
    request: &UserDelegationRequest,
    now: i64,
) -> Result<()> {
    actor.require("credential.delegate")?;
    let eligible: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM identities u JOIN identities target ON target.id=?2 WHERE u.id=?1 AND u.kind='human' AND u.revoked=0 AND target.revoked=0 AND u.tenant_id=target.tenant_id)",params![actor.identity_id,request.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    let delegated: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM user_delegations WHERE credential_id=?1)",
            [&actor.credential_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !eligible
        || delegated
        || !actor.permits_recipient(&request.identity_id)
        || request.expires_at <= now
        || request.expires_at > actor.expires_at
        || request.operations.iter().any(|op| !actor.permits(op))
        || request
            .recipients
            .iter()
            .any(|id| !actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    /// Explicit consent by a currently authenticated human. Retries preserve the
    /// same credential and expiry; changed intent conflicts. No parent scopes or
    /// secret enter the successor. Enroll its signing key separately.
    pub fn issue_user_delegation(
        &self,
        issuer: &str,
        mut request: UserDelegationRequest,
    ) -> Result<IssuedUserDelegation> {
        request.normalize()?;
        let intent = crate::provisioning::canonical_json(
            &serde_json::to_value(&request).map_err(|_| Error::storage())?,
        )?;
        let guard = self.stable_authority(
            &AuthorityWait::new(std::time::Duration::from_secs(2)),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection_at(&c, issuer, crate::now())?;
                require_issuer(&c, &actor, &request, crate::now())?;
                Ok(vec![
                    AuthorityScope::identity(&actor.identity_id, AuthorityMode::Read)?,
                    AuthorityScope::identity(&request.identity_id, AuthorityMode::Write)?,
                ])
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let now = crate::now();
        let actor = actor_from_connection_at(&tx, issuer, now)?;
        require_issuer(&tx, &actor, &request, now)?;
        let required = [
            AuthorityScope::identity(&actor.identity_id, AuthorityMode::Read)?,
            AuthorityScope::identity(&request.identity_id, AuthorityMode::Write)?,
        ];
        if !self.authority_covers(&guard, &required) {
            return Err(Error::forbidden());
        }
        let existing:Option<(String,String,String)>=tx.query_row("SELECT credential_id,issuer_credential_id,intent_json FROM user_delegations WHERE user_identity_id=?1 AND operation_id=?2",params![actor.identity_id,request.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        let id = if let Some((id, original_issuer, original_intent)) = existing {
            if original_issuer != issuer || original_intent != intent {
                return Err(Error::new(
                    "request-conflict",
                    "Delegation operation has different intent",
                ));
            }
            actor_from_connection_at(&tx, &id, now)?;
            id
        } else {
            let distinct:u64=tx.query_row("SELECT COUNT(DISTINCT child.identity_id) FROM user_delegations d JOIN credentials child ON child.id=d.credential_id WHERE d.user_identity_id=?1 AND child.identity_id!=?2",params![actor.identity_id,request.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if distinct >= 1023 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Delegation authority subject capacity exhausted",
                ));
            }
            let count: u64 = tx
                .query_row("SELECT COUNT(*) FROM user_delegations", [], |r| r.get(0))
                .map_err(|_| Error::storage())?;
            if count >= self.limits.operation_receipts {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Delegation receipt capacity exhausted",
                ));
            }
            let id = uuid::Uuid::new_v4().to_string();
            let secret = token(&tx, &id, issuer, &intent)?;
            tx.execute("INSERT INTO credentials(id,identity_id,verifier,operations,recipients,expires_at,issued_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,request.identity_id,crate::verifier(&secret),serde_json::to_string(&request.operations).map_err(|_|Error::storage())?,serde_json::to_string(&request.recipients).map_err(|_|Error::storage())?,request.expires_at,now]).map_err(|_|Error::storage())?;
            tx.execute(
                "INSERT INTO user_delegations VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    id,
                    issuer,
                    actor.identity_id,
                    request.operation_id,
                    intent,
                    now
                ],
            )
            .map_err(|_| Error::storage())?;
            id
        };
        let secret = token(&tx, &id, issuer, &intent)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(IssuedUserDelegation {
            credential: IssuedCredential { id, token: secret },
            identity_id: request.identity_id,
            authorization: UserAuthorization {
                user_id: actor.identity_id,
                scopes: request.operations,
            },
            expires_at: request.expires_at,
        })
    }
}
/// Shared by every credential entry point, not merely canonical authentication.
pub(crate) fn revalidate(
    c: &Connection,
    actor: &Actor,
    now: i64,
) -> Result<Option<UserAuthorization>> {
    let row:Option<(String,String,String)>=c.query_row("SELECT issuer_credential_id,user_identity_id,intent_json FROM user_delegations WHERE credential_id=?1",[&actor.credential_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    let Some((issuer, user_id, intent)) = row else {
        return Ok(None);
    };
    // Source must be ordinary; a successor cannot manufacture a consent chain.
    let issuer_delegated: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM user_delegations WHERE credential_id=?1)",
            [&issuer],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if issuer_delegated {
        return Err(Error::unauthorized());
    }
    let source = crate::identity::base_actor_from_connection_at(c, &issuer, now)?;
    let mut request: UserDelegationRequest =
        serde_json::from_str(&intent).map_err(|_| Error::storage())?;
    request.normalize()?;
    require_issuer(c, &source, &request, now)?;
    if source.identity_id != user_id
        || request.identity_id != actor.identity_id
        || request.operations != actor.scope.operations
        || request.recipients != actor.scope.recipients
        || request.expires_at != actor.expires_at
    {
        return Err(Error::unauthorized());
    }
    Ok(Some(UserAuthorization {
        user_id,
        scopes: request.operations,
    }))
}
/// Revoking a consenting Identity/credential must wait for every delegated
/// subject's existing IO readers, including generic/native entry points.
pub(crate) fn write_scopes(c: &Connection, identity: &str) -> Result<Vec<AuthorityScope>> {
    let mut scopes = vec![AuthorityScope::identity(identity, AuthorityMode::Write)?];
    let mut statement=c.prepare("SELECT DISTINCT child.identity_id FROM user_delegations d JOIN credentials child ON child.id=d.credential_id WHERE d.user_identity_id=?1 ORDER BY child.identity_id LIMIT 1024").map_err(|_|Error::storage())?;
    let ids = statement
        .query_map([identity], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?;
    for id in ids {
        scopes.push(AuthorityScope::identity(
            &id.map_err(|_| Error::storage())?,
            AuthorityMode::Write,
        )?);
    }
    if scopes.len() > 1024 {
        return Err(Error::new(
            "capacity-exceeded",
            "Delegation authority capacity exceeded",
        ));
    }
    Ok(scopes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::Scope;
    use connect_protocol::{provisioning::ProvisioningCommand, Limits};
    use std::{
        sync::{mpsc, Arc},
        time::Duration,
    };
    struct Fixture {
        _dir: tempfile::TempDir,
        store: Arc<Store>,
        admin: Actor,
        issuer: String,
        user: String,
        target: String,
        request: UserDelegationRequest,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = Arc::new(
                Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
            );
            let admin = store.create_identity("administrator").unwrap().id;
            let admin_key = store
                .issue(&admin, Scope::operator(), crate::now() + 600)
                .unwrap();
            let admin = store.authenticate(&admin_key.token).unwrap();
            let user = store.create_identity("human-consenter").unwrap().id;
            let target = store.create_identity("agent").unwrap().id;
            let issuer = store
                .issue(
                    &user,
                    Scope {
                        operations: vec![
                            "credential.delegate".into(),
                            "send".into(),
                            "history".into(),
                        ],
                        recipients: vec![target.clone()],
                    },
                    crate::now() + 500,
                )
                .unwrap()
                .id;
            let request = UserDelegationRequest {
                operation_id: "explicit-consent".into(),
                identity_id: target.clone(),
                operations: vec!["send".into(), "history".into()],
                recipients: vec![user.clone()],
                expires_at: crate::now() + 300,
            };
            Self {
                _dir: dir,
                store,
                admin,
                issuer,
                user,
                target,
                request,
            }
        }
    }
    #[test]
    fn explicit_human_delegation_replays_same_secret_and_never_copies_parent_authority() {
        let f = Fixture::new();
        let issued = f
            .store
            .issue_user_delegation(&f.issuer, f.request.clone())
            .unwrap();
        let mut equivalent = f.request.clone();
        equivalent.operations.reverse();
        equivalent.operations.push("send".into());
        let replay = f
            .store
            .issue_user_delegation(&f.issuer, equivalent)
            .unwrap();
        assert_eq!(issued.credential.id, replay.credential.id);
        assert_eq!(issued.credential.token, replay.credential.token);
        let actor = f.store.authenticate(&issued.credential.token).unwrap();
        assert_eq!(actor.identity_id, f.target);
        assert!(actor.permits("send"));
        assert!(!actor.permits("credential.delegate"));
        assert!(!actor.permits("admin"));
        assert_eq!(actor.expires_at, f.request.expires_at);
        let metadata = f.store.credential_metadata(&actor.credential_id).unwrap();
        let authorization = metadata.user_authorization.unwrap();
        assert_eq!(authorization.user_id, f.user);
        assert_eq!(authorization.scopes, vec!["history", "send"]);
        assert!(f
            .store
            .credential_metadata(&f.issuer)
            .unwrap()
            .user_authorization
            .is_none());
        assert!(!format!("{issued:?}").contains(&issued.credential.token));
        let c = f.store.connection().unwrap();
        let saved: String = c
            .query_row(
                "SELECT intent_json FROM user_delegations WHERE credential_id=?1",
                [&actor.credential_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!saved.contains(&issued.credential.token));
        assert!(c
            .execute(
                "DELETE FROM user_delegations WHERE credential_id=?1",
                [&actor.credential_id]
            )
            .is_err());
        assert!(c
            .execute(
                "UPDATE user_delegations SET operation_id='other' WHERE credential_id=?1",
                [&actor.credential_id]
            )
            .is_err());
    }
    #[test]
    fn consent_refuses_scope_recipient_expiry_role_and_delegation_chain_escalation() {
        let f = Fixture::new();
        for change in [
            "operation",
            "recipient",
            "expiry",
            "wildcard",
            "admin",
            "delegate",
            "unknown-user-field",
        ] {
            let mut request = f.request.clone();
            match change {
                "operation" => request.operations = vec!["close".into()],
                "recipient" => request.recipients = vec!["ungranted-peer".into()],
                "expiry" => request.expires_at = crate::now() + 550,
                "wildcard" => request.operations = vec!["*".into()],
                "admin" => request.operations = vec!["admin".into()],
                "delegate" => request.operations = vec!["credential.delegate".into()],
                _ => {
                    let mut value = serde_json::to_value(&request).unwrap();
                    value["user_id"] = serde_json::json!("selected-user");
                    assert!(serde_json::from_value::<UserDelegationRequest>(value).is_err());
                    continue;
                }
            }
            assert!(
                f.store.issue_user_delegation(&f.issuer, request).is_err(),
                "{change}"
            );
        }
        let issued = f
            .store
            .issue_user_delegation(&f.issuer, f.request.clone())
            .unwrap();
        assert!(f
            .store
            .issue_user_delegation(&issued.credential.id, f.request.clone())
            .is_err());
        let mut changed = f.request.clone();
        changed.operations = vec!["send".into()];
        assert_eq!(
            f.store
                .issue_user_delegation(&f.issuer, changed)
                .unwrap_err()
                .code,
            "request-conflict"
        );
        let c = f.store.connection().unwrap();
        c.execute("UPDATE identities SET kind='ai' WHERE id=?1", [&f.user])
            .unwrap();
        drop(c);
        assert!(f.store.authenticate(&issued.credential.token).is_err());
        assert!(f.store.issue_user_delegation(&f.issuer, f.request).is_err());
    }
    #[test]
    fn successor_scope_drift_revocation_expiry_and_tenant_changes_cannot_keep_consent_current() {
        for change in [
            "scope",
            "recipient",
            "expiry",
            "issuer-expired",
            "tenant",
            "revoked",
        ] {
            let f = Fixture::new();
            let issued = f.store.issue_user_delegation(&f.issuer, f.request).unwrap();
            let c = f.store.connection().unwrap();
            match change {
                "scope" => {
                    c.execute(
                        "UPDATE credentials SET operations='[\"send\",\"admin\"]' WHERE id=?1",
                        [&issued.credential.id],
                    )
                    .unwrap();
                }
                "recipient" => {
                    c.execute(
                        "UPDATE credentials SET recipients='[\"extra-peer\"]' WHERE id=?1",
                        [&issued.credential.id],
                    )
                    .unwrap();
                }
                "expiry" => {
                    c.execute(
                        "UPDATE credentials SET expires_at=expires_at+1 WHERE id=?1",
                        [&issued.credential.id],
                    )
                    .unwrap();
                }
                "issuer-expired" => {
                    c.execute(
                        "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                        params![f.issuer, crate::now() - 1],
                    )
                    .unwrap();
                }
                "tenant" => {
                    c.execute("INSERT INTO tenants(id) VALUES('other')", [])
                        .unwrap();
                    c.execute(
                        "UPDATE identities SET tenant_id='other' WHERE id=?1",
                        [&f.user],
                    )
                    .unwrap();
                }
                _ => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&issued.credential.id],
                    )
                    .unwrap();
                }
            }
            drop(c);
            assert!(
                f.store.authenticate(&issued.credential.token).is_err(),
                "{change}"
            );
            assert!(
                f.store.credential_metadata(&issued.credential.id).is_err(),
                "{change}"
            );
        }
    }
    #[test]
    fn issuer_revoke_waits_for_subject_io_readers_and_preserves_unrelated_credentials() {
        let f = Fixture::new();
        let issued = f
            .store
            .issue_user_delegation(&f.issuer, f.request.clone())
            .unwrap();
        let independent = f
            .store
            .issue(
                &f.target,
                Scope::messaging(vec![f.user.clone()]),
                crate::now() + 300,
            )
            .unwrap();
        let held = f
            .store
            .acquire_authority(
                &[AuthorityScope::identity(&f.target, AuthorityMode::Read).unwrap()],
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        let store = f.store.clone();
        let admin = f.admin.clone();
        let issuer = f.issuer.clone();
        let (tx, rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            tx.send(store.revoke(&admin, &issuer)).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(held);
        rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        writer.join().unwrap();
        assert!(f.store.authenticate(&issued.credential.token).is_err());
        assert!(f.store.issue_user_delegation(&f.issuer, f.request).is_err());
        assert!(f.store.authenticate(&independent.token).is_ok());
    }
    #[test]
    fn protected_credential_and_identity_revocation_share_delegated_io_custody() {
        for identity in [false, true] {
            let f = Fixture::new();
            let issued = f.store.issue_user_delegation(&f.issuer, f.request).unwrap();
            let held = f
                .store
                .acquire_authority(
                    &[AuthorityScope::identity(&f.target, AuthorityMode::Read).unwrap()],
                    &AuthorityWait::new(Duration::from_secs(1)),
                )
                .unwrap();
            let store = f.store.clone();
            let credential = f.admin.credential_id.clone();
            let command = if identity {
                ProvisioningCommand::RevokeIdentity {
                    operation_id: "revoke-human".into(),
                    identity_id: f.user.clone(),
                    expected_revision: 0,
                }
            } else {
                ProvisioningCommand::RevokeCredential {
                    operation_id: "revoke-consenter".into(),
                    credential_id: f.issuer.clone(),
                }
            };
            let (tx, rx) = mpsc::channel();
            let writer = std::thread::spawn(move || {
                tx.send(store.execute_provisioning(&credential, command))
                    .unwrap();
            });
            assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
            drop(held);
            rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
            writer.join().unwrap();
            assert!(f.store.authenticate(&issued.credential.token).is_err());
        }
    }
}
