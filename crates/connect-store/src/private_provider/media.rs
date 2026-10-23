//! Fresh authority for forwarding to a provider that already owns opaque memory.
use super::*;

/// A bounded current interval, not a retained initialization or future grant.
/// Keep it through the actual provider/native write and drop it before callbacks
/// that record metadata. No context JSON is exposed by this reader.
pub struct PrivateProviderMediaRead {
    _held: Arc<AuthorityGuardSet>,
    owner: Arc<InstallationOwnership>,
    expires_at: i64,
    delivery: String,
    attempt: String,
    provider_ref: String,
    connection: String,
    generation: u64,
}
impl std::fmt::Debug for PrivateProviderMediaRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateProviderMediaRead(redacted)")
    }
}
impl PrivateProviderMediaRead {
    /// Captured facts only; current() and the original scope interval still gate use.
    pub fn connection_id(&self) -> &str {
        &self.connection
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn observation_interval(
        &self,
        context: &PrivateProviderContext,
        applied: &PrivateProviderApplied,
    ) -> Result<&Arc<AuthorityGuardSet>> {
        if !self.current()
            || self.delivery != context.delivery
            || self.attempt != context.attempt
            || self.delivery != applied.delivery
            || self.attempt != applied.attempt
            || self.provider_ref != applied.provider_ref
        {
            return Err(Error::forbidden());
        }
        Ok(&self._held)
    }

    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires_at
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}
impl Store {
    /// Shared private memory validation in the caller's SQL snapshot. No
    /// additional authority acquisition occurs while the snapshot is held.
    pub(crate) fn private_provider_observation_facts(
        &self,
        c: &Connection,
        context: &PrivateProviderContext,
        receipt: &PrivateProviderApplied,
    ) -> Result<crate::events::observations::memory::Facts> {
        bound(self, c, context)?;
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
        let encoded: String = c.query_row(
            "SELECT d.selection_json FROM private_provider_contexts p JOIN context_deliveries d ON d.id=p.delivery_id AND d.state='delivered' AND d.provider_ref=p.provider_ref WHERE p.delivery_id=?1 AND p.attempt_id=?2 AND p.provider_ref=?3 AND p.delivered_at IS NOT NULL AND p.closed_at IS NULL AND p.uncertain=0 AND p.export_uncertain=0",
            params![receipt.delivery,receipt.attempt,receipt.provider_ref], |r|r.get(0),
        ).map_err(|_|Error::forbidden())?;
        let facts = resolve(self, c, context)?;
        authorize_memory(self, c, &facts, context, &encoded)?;
        let expires = crate::records::context::encoded_deadline(&encoded)?
            .map_or(facts.expires_at, |until| facts.expires_at.min(until));
        if crate::now() >= expires {
            return Err(Error::forbidden());
        }
        let selected = context.selection.snapshot();
        let source = selected["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .find(|p| p["connection_id"] == facts.candidate)
            .ok_or_else(Error::forbidden)?;
        let (sid,pid,epoch,generation):(String,String,u64,u64) = c.query_row(
            "SELECT session_id,participant_id,membership_epoch,generation FROM connections WHERE id=?1", [&facts.candidate],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).map_err(|_|Error::forbidden())?;
        if source["participant_id"] != pid
            || source["membership_epoch"].as_u64() != Some(epoch)
            || source["generation"].as_u64() != Some(generation)
            || sid != context.selection.origin.request.session_id
        {
            return Err(Error::forbidden());
        }
        // Observation publication/projection also revalidates the publisher and
        // current Conversation membership. Freeze those identities in the same
        // interval so a retained private reader never has to enter behind a
        // writer for an additional observation reader.
        let mut scopes = scopes(self, c, context, false)?;
        scopes.extend(crate::authority_admission::conversation_scopes(
            c,
            &facts.conversation,
            crate::AuthorityMode::Read,
        )?);
        let scopes = crate::AuthorityScope::merged(&scopes)?;
        if crate::now() >= facts.expires_at {
            return Err(Error::new(
                "authority-busy",
                "Private observation memory interval expired during validation",
            ));
        }
        Ok(crate::events::observations::memory::Facts {
            cid: facts.conversation,
            sid,
            connection: facts.candidate,
            pid,
            epoch,
            generation,
            audience: facts
                .audience
                .into_iter()
                .map(|(_, pid, epoch)| (pid, epoch))
                .collect(),
            scopes,
            expires_at: expires,
        })
    }
    pub fn try_private_provider_media_read(
        &self,
        context: &PrivateProviderContext,
        receipt: &PrivateProviderApplied,
    ) -> Result<PrivateProviderMediaRead> {
        let needed = {
            let c = self.try_connection()?;
            bound(self, &c, context)?;
            let mut needed = scopes(self, &c, context, false)?;
            needed.extend(crate::authority_admission::conversation_scopes(
                &c,
                &context.selection.origin.request.conversation_id,
                crate::AuthorityMode::Read,
            )?);
            crate::AuthorityScope::merged(&needed)?
        };
        let held = self.try_acquire_authority(&needed)?;
        let c = self.try_connection()?;
        let facts = self.private_provider_observation_facts(&c, context, receipt)?;
        if !self.authority_covers(&held, &facts.scopes) {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        Ok(PrivateProviderMediaRead {
            _held: held,
            owner,
            expires_at: facts.expires_at,
            delivery: context.delivery.clone(),
            attempt: context.attempt.clone(),
            provider_ref: receipt.provider_ref.clone(),
            connection: facts.connection,
            generation: facts.generation,
        })
    }
}

pub(super) fn authorize_memory(
    store: &Store,
    c: &Connection,
    facts: &Facts,
    context: &PrivateProviderContext,
    encoded: &str,
) -> Result<()> {
    let view: ContextView = serde_json::from_str(encoded).map_err(|_| Error::storage())?;
    let current = crate::context::view_for_private_scope_at(
        store,
        c,
        &facts.actor,
        &facts.conversation,
        1000,
        Some(&facts.delegate.identity_id),
        Some(&facts.delegate),
        Some(view.watermark),
        Some(context.consultation_record_id()),
    )?;
    if serde_json::to_value(&current).map_err(|_| Error::storage())?
        != serde_json::to_value(&view).map_err(|_| Error::storage())?
    {
        return Err(Error::forbidden());
    }
    crate::context::authorize_private_export_events(
        c,
        &facts.actor,
        &facts.delegate.identity_id,
        Some(&facts.delegate),
        &facts.conversation,
        &view.items.iter().map(|i| i.sequence).collect::<Vec<_>>(),
        Some(context.consultation_record_id()),
    )?;
    Ok(())
}
