use super::*;
impl Store {
    /// Trusted backend approval only; the returned challenge grants no credential.
    pub fn approve_enrollment(
        &self,
        issuer: &str,
        mut request: EnrollmentRequest,
    ) -> Result<OwnedEnrollment<EnrollmentOffer>> {
        request.normalize()?;
        crate::peer_credentials::key_material(request.algorithm, &request.public_key)?;
        let authority =
            self.stable_authority(&AuthorityWait::new(Duration::from_secs(2)), || {
                let c = self.connection()?;
                Ok(authorize(&c, issuer, &request)?.2)
            })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (actor, tenant, scopes) = authorize(&tx, issuer, &request)?;
        if !self.authority_covers(&authority, &scopes) {
            return Err(Error::forbidden());
        }
        let intent = intent(issuer, &request)?;
        let saved_digest = digest(&intent);
        let existing:Option<String>=tx.query_row("SELECT id FROM client_enrollments WHERE tenant_id=?1 AND issuer_identity_id=?2 AND approve_operation_id=?3",params![tenant,actor.identity_id,request.operation_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let row = if let Some(id) = existing {
            let row = load(&tx, &id)?;
            if row.issuer != issuer || row.digest != saved_digest {
                return Err(conflict());
            }
            if row.state == "completed" {
                current_credential(&tx, &row)?;
            } else if row.state != "pending" || row.request.challenge_expires_at <= crate::now() {
                return Err(Error::new("gone", "Enrollment is no longer pending"));
            }
            check_operation(&tx, &row, row.state == "completed")?;
            let live:bool=tx.query_row("SELECT NOT tombstoned AND command_type='enrollment.approve' AND state='completed' AND digest=?4 FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,request.operation_id,saved_digest],|r|r.get(0)).optional().map_err(|_|Error::storage())?.unwrap_or(false);
            if !live {
                return Err(Error::new("gone", "Enrollment approval receipt expired"));
            }
            row
        } else {
            request.validate_new_at(crate::now())?;
            let count: u64 = tx
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if count.saturating_add(2) > self.limits.operation_receipts {
                return Err(capacity());
            }
            let pending:u64=tx.query_row("SELECT COUNT(*) FROM client_enrollments WHERE identity_id=?1 AND state='pending' AND challenge_expires_at>?2",params![request.identity_id,crate::now()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if pending >= 32 {
                return Err(capacity());
            }
            for id in [&request.operation_id, &request.completion_operation_id] {
                crate::operations::reject_control_id(&tx, &actor, id)?;
                let used:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3)",params![tenant,actor.identity_id,id],|r|r.get(0)).map_err(|_|Error::storage())?;
                if used {
                    return Err(conflict());
                }
            }
            let key_used:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM peer_signing_keys WHERE key_id=?1) OR EXISTS(SELECT 1 FROM client_enrollments WHERE key_id=?1)",[&request.key_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if key_used {
                return Err(conflict());
            }
            let existing_endpoint:Option<(String,String,String,String,String,String)>=tx.query_row("SELECT id,identity_id,device_id,capabilities_json,route_owner_id,tenant_id FROM endpoints WHERE tenant_id=?1 AND transport=?2 AND lower(address)=lower(?3) AND verification_state!='revoked'",params![tenant,request.transport.endpoint_transport().as_str(),request.endpoint_address],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|Error::storage())?;
            let endpoint = if let Some((id, identity, device, caps, owner, ep_tenant)) =
                existing_endpoint
            {
                if identity != request.identity_id
                    || device != request.device.id
                    || serde_json::from_str::<Vec<String>>(&caps).map_err(|_| Error::storage())?
                        != request.capabilities
                    || owner != actor.identity_id
                    || ep_tenant != tenant
                {
                    return Err(conflict());
                }
                id
            } else {
                format!("ep_{}", uuid::Uuid::new_v4())
            };
            let id = format!("enr_{}", uuid::Uuid::new_v4());
            let challenge = secret(
                &tx,
                b"connect-client-enrollment/challenge/1\0",
                &[&id, issuer, &saved_digest],
            )?;
            tx.execute("INSERT INTO client_enrollments(id,tenant_id,issuer_identity_id,issuer_credential_id,identity_id,device_id,key_id,endpoint_id,approve_operation_id,complete_operation_id,request_json,intent_digest,challenge_hash,challenge_expires_at,credential_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![id,tenant,actor.identity_id,issuer,request.identity_id,request.device.id,request.key_id,endpoint,request.operation_id,request.completion_operation_id,crate::provisioning::canonical_json(&serde_json::to_value(&request).map_err(|_|Error::storage())?)?,saved_digest,digest(&challenge),request.challenge_expires_at,request.credential_expires_at]).map_err(|_|conflict())?;
            journal(
                &tx,
                &actor,
                &tenant,
                &request.operation_id,
                "enrollment.approve",
                &intent,
                "completed",
                Some(serde_json::json!({"enrollment_id":id,"endpoint_id":endpoint}).to_string()),
                request.credential_expires_at,
            )?;
            journal(
                &tx,
                &actor,
                &tenant,
                &request.completion_operation_id,
                "enrollment.complete",
                &intent,
                "accepted",
                None,
                request.credential_expires_at,
            )?;
            load(&tx, &id)?
        };
        let until = actor.expires_at.min(if row.state == "completed" {
            request.credential_expires_at
        } else {
            request.challenge_expires_at
        });
        let value = offer(&tx, &row)?;
        if until <= crate::now() {
            return Err(Error::unauthorized());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(OwnedEnrollment {
            value,
            until,
            _authority: authority,
        })
    }
}
