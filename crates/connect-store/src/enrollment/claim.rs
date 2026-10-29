use super::*;
fn verify(offer: &EnrollmentOffer, signature: &[u8]) -> Result<()> {
    use p256::ecdsa::signature::Verifier;
    let key =
        crate::peer_credentials::key_material(offer.request.algorithm, &offer.request.public_key)?;
    let message = offer.proof_message()?;
    let valid = match offer.request.algorithm {
        SigningAlgorithm::EdDSA => key
            .as_slice()
            .try_into()
            .ok()
            .and_then(|bytes| ed25519_dalek::VerifyingKey::from_bytes(bytes).ok())
            .zip(ed25519_dalek::Signature::from_slice(signature).ok())
            .is_some_and(|(key, sig)| key.verify_strict(&message, &sig).is_ok()),
        SigningAlgorithm::ES256 => p256::ecdsa::VerifyingKey::from_sec1_bytes(&key)
            .ok()
            .zip(p256::ecdsa::Signature::from_slice(signature).ok())
            .is_some_and(|(key, sig)| key.verify(&message, &sig).is_ok()),
    };
    if valid {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
impl Store {
    /// Device-key proof plus original fixed intent; possession of a challenge
    /// alone cannot issue authority. Completed recovery rechecks current state.
    pub fn claim_enrollment(
        &self,
        claim: EnrollmentClaim,
        actual_origin: Option<&str>,
    ) -> Result<OwnedEnrollment<EnrollmentCredential>> {
        connect_protocol::transitions::id(&claim.enrollment_id)?;
        let authority =
            self.stable_authority(&AuthorityWait::new(Duration::from_secs(2)), || {
                let c = self.connection()?;
                let row = load(&c, &claim.enrollment_id)?;
                Ok(authorize(&c, &row.issuer, &row.request)?.2)
            })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = load(&tx, &claim.enrollment_id)?;
        let (actor, tenant, scopes) = authorize(&tx, &row.issuer, &row.request)?;
        if actor.identity_id != row.issuer_identity
            || tenant != row.tenant
            || !self.authority_covers(&authority, &scopes)
        {
            return Err(Error::forbidden());
        }
        let offer = offer(&tx, &row)?;
        let signature = claim.check_binding(&offer, actual_origin)?;
        if row.state != "pending" && row.state != "completed" {
            return Err(Error::forbidden());
        }
        check_operation(&tx, &row, row.state == "completed")?;
        if row.state == "pending" && row.request.challenge_expires_at <= crate::now() {
            tx.execute(
                "UPDATE client_enrollments SET state='expired' WHERE id=?1",
                [&row.id],
            )
            .map_err(|_| Error::storage())?;
            tx.execute("UPDATE operations SET state='failed',revision=revision+1,updated_at=?4 WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,row.request.completion_operation_id,crate::now()]).map_err(|_|Error::storage())?;
            tx.commit().map_err(|_| Error::storage())?;
            return Err(Error::new("gone", "Enrollment challenge expired"));
        }
        if let Err(error) = verify(&offer, &signature) {
            if row.state == "pending" {
                let exhausted = row.attempts + 1 >= 5;
                tx.execute(
                    "UPDATE client_enrollments SET attempts=attempts+1,state=?2 WHERE id=?1",
                    params![row.id, if exhausted { "exhausted" } else { "pending" }],
                )
                .map_err(|_| Error::storage())?;
                if exhausted {
                    tx.execute("UPDATE operations SET state='failed',revision=revision+1,updated_at=?4 WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,row.request.completion_operation_id,crate::now()]).map_err(|_|Error::storage())?;
                }
                tx.commit().map_err(|_| Error::storage())?;
            }
            return Err(error);
        }
        if row.state == "pending" {
            let request = issue_request(&row.request);
            let credential_id = uuid::Uuid::new_v4().to_string();
            let now = crate::now();
            let token = credential_issuance::token(
                &tx,
                &credential_id,
                &row.issuer,
                &request.operation_id,
                &intent(&row.issuer, &row.request)?,
            )?;
            credential_issuance::insert(&tx, &credential_id, &request, &token, now)?;
            if !endpoint_matches(&tx, &row)? {
                crate::provisioning::insert_endpoint(
                    &tx,
                    &actor,
                    &tenant,
                    &endpoint_command(&row),
                    &row.endpoint,
                )?;
            }
            // Even when reusing an exact Endpoint for rotation, Device facts
            // remain current and immutable under the target Identity fence.
            let device:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND identity_id=?2 AND tenant_id=?3 AND kind=?4 AND platform=?5 AND revoked=0)",params![row.request.device.id,row.request.identity_id,tenant,row.request.device.kind,row.request.device.platform],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !device {
                return Err(Error::forbidden());
            }
            let key = ProvisioningCommand::SigningKey {
                operation_id: request.operation_id.clone(),
                identity_id: request.identity_id.clone(),
                credential_id: credential_id.clone(),
                device_id: row.request.device.id.clone(),
                key_id: row.request.key_id.clone(),
                algorithm: row.request.algorithm,
                public_key: row.request.public_key.clone(),
                not_before: now,
                expires_at: request.expires_at,
                expected_revision: None,
            };
            crate::peer_credentials::apply(&tx, &actor, &tenant, &key)?;
            tx.execute("UPDATE client_enrollments SET state='completed',credential_id=?2,issued_at=?3 WHERE id=?1 AND state='pending'",params![row.id,credential_id,now]).map_err(|_|Error::storage())?;
            let receipt = serde_json::json!({"enrollment_id":row.id,"endpoint_id":row.endpoint,"credential_id":credential_id,"device_id":row.request.device.id,"key_id":row.request.key_id,"issued_at":now,"expires_at":request.expires_at});
            tx.execute("UPDATE operations SET state='completed',result_json=?4,revision=revision+1,updated_at=?5 WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3 AND state='accepted'",params![tenant,actor.identity_id,request.operation_id,receipt.to_string(),now]).map_err(|_|Error::storage())?;
            if row.request.challenge_expires_at <= crate::now() {
                return Err(Error::new("gone", "Enrollment challenge expired"));
            }
        }
        let row = load(&tx, &row.id)?;
        let value = current_credential(&tx, &row)?;
        let until = actor.expires_at.min(value.credential.expires_at);
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
