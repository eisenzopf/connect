use super::*;
fn require(
    c: &Connection,
    issuer: &str,
    row: &Row,
) -> Result<(Actor, String, Vec<AuthorityScope>)> {
    let actor = actor_from_connection_at(c, issuer, crate::now())?;
    actor.require("admin")?;
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1 AND revoked=0",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    crate::provisioning::require_identity(c, &actor, &tenant, &row.request.identity_id, false)?;
    if tenant != row.tenant {
        return Err(Error::forbidden());
    }
    let scopes = vec![
        AuthorityScope::identity(&actor.identity_id, AuthorityMode::Read)?,
        AuthorityScope::identity(&row.request.identity_id, AuthorityMode::Write)?,
    ];
    Ok((actor, tenant, scopes))
}
impl Store {
    /// Revoke this approval/credential/key. Shared Device and Endpoint resources
    /// retain independent lifetimes so an admitted replacement can keep using them.
    pub fn revoke_enrollment(
        &self,
        issuer: &str,
        enrollment_id: &str,
        operation_id: &str,
    ) -> Result<OwnedEnrollment<EnrollmentRevocation>> {
        connect_protocol::transitions::id(enrollment_id)?;
        connect_protocol::transitions::id(operation_id)?;
        let authority =
            self.stable_authority(&AuthorityWait::new(Duration::from_secs(2)), || {
                let c = self.connection()?;
                let row = load(&c, enrollment_id)?;
                Ok(require(&c, issuer, &row)?.2)
            })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = load(&tx, enrollment_id)?;
        let (actor, tenant, scopes) = require(&tx, issuer, &row)?;
        if !self.authority_covers(&authority, &scopes) {
            return Err(Error::forbidden());
        }
        crate::operations::reject_control_id(&tx, &actor, operation_id)?;
        let intent = crate::provisioning::canonical_json(
            &serde_json::json!({"enrollment_id":enrollment_id}),
        )?;
        let existing:Option<(String,String,String,bool)>=tx.query_row("SELECT command_type,digest,state,tombstoned FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        if let Some((kind, old_digest, state, tombstoned)) = existing {
            if kind != "enrollment.revoke" || old_digest != digest(&intent) {
                return Err(conflict());
            }
            if tombstoned {
                return Err(Error::new("gone", "Enrollment revocation receipt expired"));
            }
            if state != "completed" || row.state != "revoked" {
                return Err(conflict());
            }
        } else {
            let count: u64 = tx
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if count >= self.limits.operation_receipts {
                return Err(capacity());
            }
            if let Some(credential) = &row.credential {
                tx.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [credential])
                    .map_err(|_| Error::storage())?;
                let changed=tx.execute("UPDATE peer_signing_keys SET revoked=1,revision=revision+1 WHERE key_id=?1 AND revoked=0 AND revision<9007199254740991",[&row.request.key_id]).map_err(|_|Error::storage())?;
                let revoked: bool = tx
                    .query_row(
                        "SELECT revoked FROM peer_signing_keys WHERE key_id=?1",
                        [&row.request.key_id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                if changed == 0 && !revoked {
                    return Err(conflict());
                }
            } else {
                tx.execute("UPDATE operations SET state='cancelled',revision=revision+1,updated_at=?4 WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3 AND state='accepted'",params![tenant,row.issuer_identity,row.request.completion_operation_id,crate::now()]).map_err(|_|Error::storage())?;
            }
            tx.execute(
                "UPDATE client_enrollments SET state='revoked' WHERE id=?1",
                [enrollment_id],
            )
            .map_err(|_| Error::storage())?;
            let value = EnrollmentRevocation {
                enrollment_id: enrollment_id.into(),
                credential_id: row.credential.clone(),
                revoked: true,
            };
            journal(
                &tx,
                &actor,
                &tenant,
                operation_id,
                "enrollment.revoke",
                &intent,
                "completed",
                Some(serde_json::to_string(&value).map_err(|_| Error::storage())?),
                actor.expires_at,
            )?;
        }
        if actor.expires_at <= crate::now() {
            return Err(Error::unauthorized());
        }
        let value = EnrollmentRevocation {
            enrollment_id: enrollment_id.into(),
            credential_id: row.credential,
            revoked: true,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(OwnedEnrollment {
            value,
            until: actor.expires_at,
            _authority: authority,
        })
    }
}
