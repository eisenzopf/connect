//! Once-only compound create/history export. Unknown creation may have exported.
use super::*;
pub struct AssistantProviderInitialization {
    attempt: AssistantProviderAttempt,
    json: String,
}
redacted!(AssistantProviderInitialization);
impl AssistantProviderInitialization {
    pub fn attempt(&self) -> &AssistantProviderAttempt {
        &self.attempt
    }
    pub fn export_for(
        &self,
        created: &AssistantProviderCreated,
    ) -> Result<AssistantProviderExport> {
        if created.context.delivery != self.attempt.context.delivery
            || created.context.attempt != self.attempt.context.attempt
            || !created.context.initialization
        {
            return Err(Error::forbidden());
        }
        Ok(AssistantProviderExport {
            context: created.context.clone(),
            reference: created.reference.clone(),
            json: self.json.clone(),
        })
    }
}
impl Store {
    pub fn begin_native_assistant_initialization(
        &self,
        context: AssistantProviderContext,
        mut read: AssistantProviderRead,
    ) -> Result<(AssistantProviderInitialization, AssistantProviderRead)> {
        if !context.initialization
            || !read.current()
            || read.update.is_some()
            || read.context.delivery != context.delivery
            || read.context.attempt != context.attempt
            || read.provider_ref.is_some()
        {
            return Err(Error::forbidden());
        }
        let json = read.read.context_json()?.to_owned();
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, &context)?;
        if tx.execute("UPDATE native_assistant_providers SET creation_started=1,creation_expires_at=?2 WHERE delivery_id=?1 AND creation_started=0 AND uncertain=0 AND closed_at IS NULL",params![context.delivery,read.expires_at()]).map_err(|_|Error::storage())?!=1 {
            return Err(Error::new("reconciliation-required","Provider initialization was already attempted"));
        }
        if tx.execute("UPDATE native_assistant_initializations SET started=1,expires_at=?2 WHERE delivery_id=?1 AND started=0",params![context.delivery,read.expires_at()]).map_err(|_|Error::storage())?!=1 { return Err(Error::forbidden()); }
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        read.initialization_admitted = true;
        Ok((
            AssistantProviderInitialization {
                attempt: AssistantProviderAttempt { context },
                json,
            },
            read,
        ))
    }
    /// Acknowledgment-only IO for the already admitted inline export. The
    /// original export interval cannot be renewed by opening another read.
    pub fn try_native_assistant_initialization_read(
        &self,
        context: &AssistantProviderContext,
    ) -> Result<AssistantProviderRead> {
        original_runtime(self, context)?;
        if !context.initialization {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        bound(&c, context)?;
        let (json,endpoint,audience,reference,expires):(String,String,String,String,i64)=c.query_row("SELECT d.selection_json,p.endpoint_json,p.audience_json,p.provider_ref,i.expires_at FROM native_assistant_providers p JOIN native_assistant_initializations i ON i.delivery_id=p.delivery_id JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1 AND i.started=1 AND p.created_at IS NOT NULL AND p.export_started=1 AND p.delivered_at IS NULL AND p.rejected_at IS NULL AND p.uncertain=0 AND p.end_started=0 AND p.closed_at IS NULL AND d.state='pending'",[&context.delivery],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|Error::forbidden())?;
        drop(c);
        let saved: connect_protocol::ContextView =
            serde_json::from_str(&json).map_err(|_| Error::storage())?;
        let mut read = self.try_native_assistant_context_read_at(
            &context.origin,
            &context.credential,
            &context.endpoint,
            Some(saved.watermark),
        )?;
        if endpoint != crate::provisioning::canonical_json(&read.endpoint)?
            || audience != serde_json::to_string(&read.audience).map_err(|_| Error::storage())?
            || serde_json::from_str::<Value>(&read.json).map_err(|_| Error::storage())?
                != serde_json::to_value(saved).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        read.native.expires_at = read.native.expires_at.min(expires);
        if !read.current() {
            return Err(Error::forbidden());
        }
        read.json = json;
        Ok(AssistantProviderRead {
            context: context.clone(),
            read,
            provider_ref: Some(reference),
            initialization_admitted: false,
            update: None,
        })
    }
}
