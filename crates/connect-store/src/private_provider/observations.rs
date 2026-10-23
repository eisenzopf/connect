//! Private provider observations retain their original physical context/audience.
use super::*;
use sha2::{Digest, Sha256};
impl Store {
    pub fn private_provider_uncertain(&self, attempt: &PrivateProviderAttempt) -> Result<()> {
        let context = &attempt.context;
        let origin = &context.selection.origin;
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || origin.runtime != self.runtime_epoch()
            || !origin
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if tx.execute("UPDATE private_provider_contexts SET uncertain=1 WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3 AND started=1 AND delivered_at IS NULL",params![context.delivery,context.attempt,origin.runtime]).map_err(|_|Error::storage())?!=1{return Err(Error::forbidden());}
        tx.execute("UPDATE context_deliveries SET state='uncertain',updated_at=?2 WHERE id=?1 AND state='pending'",params![context.delivery,crate::now()]).map_err(|_|Error::storage())?;
        mark_interrupted(&tx, context.consultation_record_id(), self)?;
        progress(&tx, context.consultation_record_id(), self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// An attributed actual final callback from this exact private provider
    /// instance. It cannot choose a Conversation, Session, sender or audience.
    pub fn record_private_provider_transcript(
        &self,
        context: &PrivateProviderContext,
        receipt: &PrivateProviderApplied,
        source_id: &str,
        role: &str,
        transcript_type: &str,
        body: &str,
    ) -> Result<Option<connect_protocol::Message>> {
        self.record_private_provider_transcript_attempt(
            context,
            receipt,
            source_id,
            role,
            transcript_type,
            body,
            false,
        )
    }
    /// A single current-authority callback attempt for an active media loop.
    /// Neither the process SQL mutex, SQLite writer nor authority writer queue
    /// is waited on. A busy result retains the caller's same logical source ID.
    pub fn try_record_private_provider_transcript(
        &self,
        context: &PrivateProviderContext,
        receipt: &PrivateProviderApplied,
        source_id: &str,
        role: &str,
        transcript_type: &str,
        body: &str,
    ) -> Result<Option<connect_protocol::Message>> {
        self.record_private_provider_transcript_attempt(
            context,
            receipt,
            source_id,
            role,
            transcript_type,
            body,
            true,
        )
    }
    fn record_private_provider_transcript_attempt(
        &self,
        context: &PrivateProviderContext,
        receipt: &PrivateProviderApplied,
        source_id: &str,
        role: &str,
        transcript_type: &str,
        body: &str,
        immediate: bool,
    ) -> Result<Option<connect_protocol::Message>> {
        if transcript_type != "final" {
            return Ok(None);
        }
        if source_id.is_empty()
            || source_id.len() > 512
            || source_id.chars().any(char::is_control)
            || !matches!(role, "user" | "assistant")
            || body.is_empty()
            || body.len() > self.limits.message_bytes
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid private final transcript",
            ));
        }
        let derive = || {
            let c = if immediate {
                self.try_connection()?
            } else {
                self.connection()?
            };
            bound(self, &c, context)?;
            let facts = resolve(self, &c, context)?;
            let mut scopes = scopes(self, &c, context, false)?;
            for (identity, _, _) in facts.audience {
                scopes.push(AuthorityScope::identity(&identity, AuthorityMode::Write)?);
            }
            AuthorityScope::merged(&scopes)
        };
        let held = if immediate {
            self.try_acquire_authority(&derive()?)?
        } else {
            self.stable_authority(
                &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
                derive,
            )?
        };
        let mut c = if immediate {
            self.try_connection()?
        } else {
            self.connection()?
        };
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(self, &tx, context)?;
        let facts = resolve(self, &tx, context)?;
        let mut needed = scopes(self, &tx, context, false)?;
        for (identity, _, _) in &facts.audience {
            needed.push(AuthorityScope::identity(identity, AuthorityMode::Write)?);
        }
        if !self.authority_covers(&held, &needed) {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
            || receipt.delivery != context.delivery
            || receipt.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        let delivered:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM private_provider_contexts p JOIN context_deliveries d ON d.id=p.delivery_id AND d.state='delivered' AND d.provider_ref=p.provider_ref WHERE p.delivery_id=?1 AND p.attempt_id=?2 AND p.provider_ref=?3 AND p.delivered_at IS NOT NULL AND p.closed_at IS NULL)",params![receipt.delivery,receipt.attempt,receipt.provider_ref],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !delivered {
            return Err(Error::forbidden());
        }
        let encoded: String = tx
            .query_row(
                "SELECT selection_json FROM context_deliveries WHERE id=?1",
                [&context.delivery],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        super::media::authorize_memory(self, &tx, &facts, context, &encoded)?;
        let sender_connection = if role == "user" {
            facts.primary.as_str()
        } else {
            facts.candidate.as_str()
        };
        let (sender, pid, epoch): (String, String, u64) = tx
            .query_row(
                "SELECT identity_id,participant_id,membership_epoch FROM connections WHERE id=?1",
                [sender_connection],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| Error::storage())?;
        if !facts
            .audience
            .contains(&(sender.clone(), pid.clone(), epoch))
        {
            return Err(Error::forbidden());
        }
        let digest = hex::encode(Sha256::digest(
            format!(
                "private-provider-transcript/1\0{}\0{}\0{}\0{}",
                context.delivery, receipt.provider_ref, source_id, role
            )
            .as_bytes(),
        ));
        let id = format!("pmsg_{}", &digest[..48]);
        let existing: Option<String> = tx
            .query_row(
                "SELECT body FROM messages WHERE id=?1 AND conversation_id=?2",
                params![id, facts.conversation],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some(existing) = existing {
            if existing != body {
                return Err(Error::new(
                    "conflict",
                    "Private transcript identity changed",
                ));
            }
            return crate::messages::get(&tx, &facts.conversation, &id).map(Some);
        }
        if !crate::conversations::capacity(
            &tx,
            facts.audience.len() as u64,
            body.len() as u64,
            self,
        )? {
            return Err(Error::new(
                "capacity-exceeded",
                "Private transcript capacity exceeded",
            ));
        }
        tx.execute("INSERT INTO messages(id,conversation_id,sender,body,audience_mode,sender_participant_id,content_type,body_encoding,label,accepted_at) VALUES(?1,?2,?3,?4,'explicit',?5,'text/plain','utf8','voice_transcript',?6)",params![id,facts.conversation,sender,body,pid,crate::now()]).map_err(|_|Error::storage())?;
        for (identity, pid, epoch) in &facts.audience {
            tx.execute("INSERT INTO message_audiences(message_id,conversation_id,participant_id,membership_epoch,purpose) VALUES(?1,?2,?3,?4,?5)",params![id,facts.conversation,pid,epoch,if identity==&sender{"sender"}else{"recipient"}]).map_err(|_|Error::storage())?;
        }
        tx.execute("INSERT INTO private_content_sources(kind,source_id,conversation_id,operation_record_id,connection_id) VALUES('message',?1,?2,?3,?4)",params![id,facts.conversation,context.consultation_record_id(),facts.candidate]).map_err(|_|Error::storage())?;
        for (identity, pid, epoch) in &facts.audience {
            let data = json!({"source_kind":"message","source_id":id,"session_id":context.selection.origin.request.session_id,"connection_id":facts.candidate});
            tx.execute("INSERT INTO journal(id,recipient,conversation_id,kind,message_id,data_json,recipient_participant_id,recipient_epoch) VALUES(?1,?2,?3,'voice_transcript',?4,?5,?6,?7)",params![format!("evt_{}",uuid::Uuid::new_v4()),identity,facts.conversation,id,data.to_string(),pid,epoch]).map_err(|_|Error::storage())?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let message = crate::messages::get(&tx, &facts.conversation, &id)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(message))
    }
}

impl Store {
    /// Trusted runtime records only after actual termination of this exact
    /// captured provider instance. This performs no remote cleanup command.
    pub fn record_private_provider_closed(&self, receipt: &PrivateProviderApplied) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<(String,Option<i64>)>=tx.query_row("SELECT operation_record_id,closed_at FROM private_provider_contexts WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3 AND provider_ref=?4 AND delivered_at IS NOT NULL",params![receipt.delivery,receipt.attempt,receipt.runtime,receipt.provider_ref],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (root, closed) = row.ok_or_else(Error::forbidden)?;
        if closed.is_none() {
            tx.execute(
                "UPDATE private_provider_contexts SET closed_at=?2 WHERE delivery_id=?1",
                params![receipt.delivery, crate::now()],
            )
            .map_err(|_| Error::storage())?;
            mark_interrupted(&tx, &root, self)?;
            crate::transitions::settle_after_provider_closed(&tx, &root)?;
            progress(&tx, &root, self)?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
}

pub(crate) fn recover_one(
    c: &Connection,
    row: &crate::transitions::rows::Saved,
    store: &Store,
) -> Result<bool> {
    use connect_protocol::transitions::OperationState;
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(false);
    }
    let (started,runtime,credential):(bool,String,String)=c.query_row("SELECT p.started,p.runtime_epoch,l.credential_id FROM private_provider_contexts p JOIN native_consultation_legs l ON l.operation_record_id=p.operation_record_id WHERE p.operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    let (context_key,delegate,selection,audience,endpoint,endpoint_json):(String,String,String,String,String,String)=c.query_row("SELECT context_credential_id,delegate_credential_id,selection_json,audience_json,endpoint_id,endpoint_json FROM private_provider_contexts WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|_|Error::storage())?;
    let endpoint_json: Value =
        serde_json::from_str(&endpoint_json).map_err(|_| Error::storage())?;
    let endpoint_current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.revoked=0 WHERE e.id=?1 AND e.revision=?2 AND e.verification_state='verified' AND e.transport='vapi-voice' AND e.device_id=?3 AND e.address=?4 AND e.secret_ref=?5 AND d.kind=?6 AND d.platform=?7)",params![endpoint,endpoint_json["revision"].as_u64().ok_or_else(Error::storage)?,endpoint_json["device_id"].as_str().ok_or_else(Error::storage)?,endpoint_json["address"].as_str().ok_or_else(Error::storage)?,endpoint_json["secret_ref"].as_str().ok_or_else(Error::storage)?,endpoint_json["device_kind"].as_str().ok_or_else(Error::storage)?,endpoint_json["device_platform"].as_str().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?;
    let root_source_current = crate::NativeConsultationOrigin::from_private_leg(
        store,
        c,
        row.record.record_id.clone(),
        credential.clone(),
        row.digest.clone(),
        runtime.clone(),
    )
    .is_ok();
    let source_lost = !root_source_current
        || [&credential, &context_key, &delegate]
            .iter()
            .any(|key| crate::identity::actor_from_connection(c, key).is_err())
        || !endpoint_current
        || crate::connection_control::private_consultation::active::validate(
            store,
            c,
            &row.record.record_id,
            &selection,
            &audience,
        )
        .is_err();
    let timed_out:bool=c.query_row("SELECT (started=1 AND created_at IS NULL AND creation_expires_at<=?2) OR (export_started=1 AND delivered_at IS NULL AND export_expires_at<=?2) FROM private_provider_contexts WHERE operation_record_id=?1",params![row.record.record_id,crate::now()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if timed_out
        || row.record.expires_at.is_none_or(|t| t <= crate::now())
        || (store.recover_foreign_runtime && runtime != store.runtime_epoch())
        || source_lost
    {
        let next = if started {
            OperationState::Unknown
        } else {
            OperationState::Failed
        };
        // Provider initialization starts independently of audience application.
        // Accepted work therefore enters preparing before uncertainty is known.
        if row.record.state == OperationState::Accepted && started {
            c.execute(
                "UPDATE operations SET state='preparing' WHERE record_id=?1",
                [&row.record.record_id],
            )
            .map_err(|_| Error::storage())?;
            progress(c, &row.record.record_id, store)?;
        }
        c.execute(
            "UPDATE operations SET state=?2 WHERE record_id=?1",
            params![row.record.record_id, next.as_str()],
        )
        .map_err(|_| Error::storage())?;
        progress(c, &row.record.record_id, store)?;
        crate::transitions::workers::invalidate(c, &row.record.record_id)?;
        return Ok(true);
    }
    Ok(false)
}

fn mark_interrupted(c: &Connection, root: &str, store: &Store) -> Result<()> {
    let state: String = c
        .query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if state == "accepted" {
        c.execute(
            "UPDATE operations SET state='preparing' WHERE record_id=?1",
            [root],
        )
        .map_err(|_| Error::storage())?;
        progress(c, root, store)?;
    }
    if matches!(
        state.as_str(),
        "accepted" | "preparing" | "committing" | "interrupted"
    ) {
        c.execute(
            "UPDATE operations SET state='unknown' WHERE record_id=?1",
            [root],
        )
        .map_err(|_| Error::storage())?;
        progress(c, root, store)?;
        crate::transitions::workers::invalidate(c, root)?;
    }
    Ok(())
}

impl Store {
    pub fn private_provider_export_uncertain(&self, attempt: &PrivateProviderExport) -> Result<()> {
        let context = &attempt.context;
        let origin = &context.selection.origin;
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || origin.runtime != self.runtime_epoch()
            || !origin
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if tx.execute("UPDATE private_provider_contexts SET export_uncertain=1 WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3 AND provider_ref=?4 AND export_started=1 AND delivered_at IS NULL",params![context.delivery,context.attempt,origin.runtime,attempt.provider_ref]).map_err(|_|Error::storage())?!=1{return Err(Error::forbidden());}
        tx.execute("UPDATE context_deliveries SET state='uncertain',updated_at=?2 WHERE id=?1 AND state='pending'",params![context.delivery,crate::now()]).map_err(|_|Error::storage())?;
        mark_interrupted(&tx, context.consultation_record_id(), self)?;
        progress(&tx, context.consultation_record_id(), self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// A fresh call can end before any successful context export. Capturing its
    /// actual termination does not assert that a context was initialized.
    pub fn record_private_provider_created_closed(
        &self,
        receipt: &PrivateProviderCreated,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<(String,Option<i64>)>=tx.query_row("SELECT operation_record_id,closed_at FROM private_provider_contexts WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3 AND provider_ref=?4 AND created_at IS NOT NULL",params![receipt.delivery,receipt.attempt,receipt.runtime,receipt.provider_ref],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (root, closed) = row.ok_or_else(Error::forbidden)?;
        if closed.is_none() {
            tx.execute(
                "UPDATE private_provider_contexts SET closed_at=?2 WHERE delivery_id=?1",
                params![receipt.delivery, crate::now()],
            )
            .map_err(|_| Error::storage())?;
            mark_interrupted(&tx, &root, self)?;
            crate::transitions::settle_after_provider_closed(&tx, &root)?;
            progress(&tx, &root, self)?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
}

impl Store {
    /// A creation response whose uniqueness cannot be established permits only
    /// uncertainty recording, never private export or cleanup of a public call.
    pub fn private_provider_created_uncertain(
        &self,
        receipt: &PrivateProviderCreated,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<(String,bool)>=tx.query_row("SELECT operation_record_id,uncertain FROM private_provider_contexts WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3 AND started=1 AND provider_ref IS NULL AND created_at IS NULL",params![receipt.delivery,receipt.attempt,receipt.runtime],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (root, uncertain) = row.ok_or_else(Error::forbidden)?;
        if !uncertain {
            tx.execute(
                "UPDATE private_provider_contexts SET uncertain=1 WHERE delivery_id=?1",
                [&receipt.delivery],
            )
            .map_err(|_| Error::storage())?;
            tx.execute("UPDATE context_deliveries SET state='uncertain',updated_at=?2 WHERE id=?1 AND state='pending'",params![receipt.delivery,crate::now()]).map_err(|_|Error::storage())?;
            mark_interrupted(&tx, &root, self)?;
            progress(&tx, &root, self)?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
}
