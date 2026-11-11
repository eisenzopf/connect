//! Fresh internal authority facts. No conversion to a public operation record.
use super::*;
mod operation_scopes;
pub(in crate::transitions) use operation_scopes::operation_source_scopes;

// Do not implement Clone, Deref, AsRef<Saved>, Serialize or a record accessor.
// The inner row has no rendered causal page and must never escape as a reply.
pub(in crate::transitions) struct AuthoritySaved(Saved);

impl AuthoritySaved {
    pub(super) fn new(row: Saved) -> Self {
        Self(row)
    }
    pub(in crate::transitions) fn digest(&self) -> &str {
        &self.0.digest
    }
    pub(in crate::transitions) fn request(&self) -> Result<TransitionRequest> {
        super::request(&self.0)
    }
    pub(in crate::transitions) fn state(&self) -> OperationState {
        self.0.record.state
    }
    pub(in crate::transitions) fn worker(&self) -> Option<&str> {
        self.0.worker.as_deref()
    }
    pub(in crate::transitions) fn effects(&self) -> &[OperationEffect] {
        &self.0.record.effects
    }
    pub(in crate::transitions) fn is_ledger(&self) -> bool {
        self.0.ledger
    }
    pub(in crate::transitions) fn is_native_replacement(&self) -> bool {
        self.0.native_replacement
    }
    pub(in crate::transitions) fn tombstoned(&self) -> bool {
        self.0.tombstoned
    }
    pub(in crate::transitions) fn cancel_requested(&self) -> bool {
        self.0.record.cancel_requested
    }
    pub(in crate::transitions) fn expires_at(&self) -> Option<i64> {
        self.0.record.expires_at
    }

    // Reuse the same complete current authorization, including every existing
    // kind-specific branch. Only status display rendering was omitted.
    pub(in crate::transitions) fn require_read(&self, c: &Connection, actor: &Actor) -> Result<()> {
        super::super::authority::read(c, actor, &self.0)
    }
    pub(in crate::transitions) fn require_cleanup(
        &self,
        c: &Connection,
        actor: &Actor,
    ) -> Result<()> {
        super::super::cleanup::authorize(c, actor, &self.0)
    }
    pub(in crate::transitions) fn require_execution(
        &self,
        c: &Connection,
        actor: &Actor,
        store: &Store,
    ) -> Result<()> {
        super::super::authority::execution(c, actor, &self.0, store)
    }
}
