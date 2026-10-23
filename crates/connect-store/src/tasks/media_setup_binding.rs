//! Durable setup identity outlives callback listeners and process-local media.
use crate::Store;
use connect_protocol::{tasks::GrantDefinition, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

impl Store {
    /// Reserve one original protected call setup for an accepted task session.
    /// This is trusted composition, not a model operation or provider dispatch.
    /// Losing the listener never authorizes replacement of this intent. A new
    /// voice leg may continue the same task and conversation with a new session.
    pub fn register_vapi_task_session_setup(
        &self,
        credential: &str,
        definition: &GrantDefinition,
        session: &str,
        expected: &crate::VapiSessionRoute,
    ) -> Result<()> {
        let authority = self.runtime_conversation_authority(|c| {
            let (_, task) = super::media_setup::check_definition(
                c,
                credential,
                definition,
                session,
                expected,
                crate::now(),
            )?;
            Ok(vec![task.conversation_id])
        })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_runtime_query(
            &tx,
            &authority,
            "SELECT conversation_id FROM sessions WHERE id=?1",
            &[&session],
        )?;
        super::media_setup::check_definition(
            &tx,
            credential,
            definition,
            session,
            expected,
            crate::now(),
        )?;
        let saved = binding(credential, definition)?;
        let changed = tx.execute(
            "INSERT INTO vapi_session_setups(session_id,nonce,binding_json) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",
            params![session, definition.binding.external_id, saved],
        ).map_err(|_| Error::storage())?;
        if changed != 1 {
            return Err(Error::new(
                "conflict",
                "Session or invocation already has a protected Vapi setup; retain the original intent",
            ));
        }
        tx.commit().map_err(|_| Error::storage())
    }

    /// Trusted media composition checks this after authorizing the session route.
    /// A previously configured setup, including a legacy unknown-intent marker,
    /// cannot be replaced by an ordinary call when its listener is unavailable.
    /// Sessions without a configured tool setup retain their voice-only behavior.
    pub fn vapi_session_requires_task_setup(&self, session: &str) -> Result<bool> {
        let c = self.connection()?;
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM vapi_session_setups WHERE session_id=?1)",
            [session],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())
    }
}

pub(super) fn authorize(
    c: &Connection,
    credential: &str,
    definition: &GrantDefinition,
    session: &str,
) -> Result<()> {
    let saved: Option<Option<String>> = c
        .query_row(
            "SELECT binding_json FROM vapi_session_setups WHERE session_id=?1",
            [session],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if saved.flatten().as_deref() != Some(binding(credential, definition)?.as_str()) {
        return Err(Error::forbidden());
    }
    Ok(())
}

fn binding(credential: &str, definition: &GrantDefinition) -> Result<String> {
    // Credential reference and bounded grant definition only: never bearer keys,
    // callback secrets, private filesystem paths or provider response bodies.
    serde_json::to_string(&(credential, definition)).map_err(|_| Error::storage())
}
