//! Replacement preparation installs custody, never a public Data grant.
use super::*;
use std::sync::OnceLock;

pub(crate) struct PendingObserver {
    scope: StagedRtpScope,
    writer: OnceLock<Arc<dyn NativeDataObserver>>,
}
fn same_scope(left: &StagedRtpScope, right: &StagedRtpScope) -> bool {
    left.session == right.session
        && left.connection == right.connection
        && left.owner == right.owner
        && left.wire_conversation == right.wire_conversation
        && left.wire_session == right.wire_session
        && left.wire_connection == right.wire_connection
}
impl PendingObserver {
    pub(crate) fn new(scope: StagedRtpScope) -> Self {
        Self {
            scope,
            writer: OnceLock::new(),
        }
    }
    pub(crate) fn scope(&self) -> &StagedRtpScope {
        &self.scope
    }
    pub(crate) fn installed(&self) -> bool {
        self.writer.get().is_some()
    }
    pub(crate) fn activate(&self, writer: Observer) -> ControlResult<()> {
        let scope = writer.scope.clone();
        self.install(&scope, Arc::new(writer))
    }
    fn install(
        &self,
        scope: &StagedRtpScope,
        writer: Arc<dyn NativeDataObserver>,
    ) -> ControlResult<()> {
        if !same_scope(&self.scope, scope) {
            return Err(StreamControlError::Authority);
        }
        self.writer
            .set(writer)
            .map_err(|_| StreamControlError::Authority)
    }
}
impl NativeDataObserver for PendingObserver {
    fn observe(&self, scope: &StagedRtpScope, outcome: &NativeDataOutcome) -> ControlResult<()> {
        if !same_scope(&self.scope, scope) || outcome.target != scope.connection {
            return Err(StreamControlError::Authority);
        }
        // The original writer is installed before the ordinary adopted route
        // becomes visible. Keep it for historical outcomes after retirement;
        // the observer itself never grants current publication/delivery rights.
        self.writer
            .get()
            .ok_or(StreamControlError::Authority)?
            .observe(scope, outcome)
    }
}

#[cfg(test)]
mod tests;
