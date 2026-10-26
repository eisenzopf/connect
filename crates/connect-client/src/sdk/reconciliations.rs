//! Local reconciliation progress and original journal recovery, never a grant.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReconciliationPhase {
    Authorizing,
    AwaitingCompletion,
    Recorded,
    Refused,
    Unresolved,
}
#[derive(Clone, Debug)]
pub struct NativeReconciliation {
    pub key: OperationKey,
    pub expected_revision: u64,
    pub record_id: Option<OperationRecordId>,
    pub subscription_record_id: Option<OperationRecordId>,
    pub phase: NativeReconciliationPhase,
    /// True only after the matching captured bindings are adopted locally.
    pub bindings_adopted: bool,
    pub error: Option<ErrorKind>,
}
impl NativeReconciliation {
    /// Observe the original server-created operation. This never submits it.
    /// A different Client may read the receipt but acquires no native custody.
    pub fn operation(&self, client: &Client) -> Operation<StreamSelectionReceipt> {
        client.recover(OperationReference {
            key: self.key.clone(),
            kind: client.operation_kinds().stream_reconcile,
            continuation: None,
        })
    }
}
impl Connection {
    /// Latest local server-callback progress on this exact owned native peer.
    pub fn native_reconciliation(&self) -> Result<Option<NativeReconciliation>> {
        self.client()
            .shared
            .reconciliation(self.id(), self.session_id(), self.conversation_id())
    }
}
