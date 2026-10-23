//! Explicit application extensions; canonical communication has one wire dialect.
use crate::Operation;
pub fn response_kind(operation: &Operation) -> Option<&'static str> {
    use Operation::*;
    Some(match operation {
        Health {} => "health",
        Resolve { .. } => "identity",
        Capabilities { .. } => "capabilities",
        Task { .. } => "task",
        ParticipantRoute { .. } => "participant_route",
        BrowserBinding { .. } => "browser_binding",
        ContactState { .. } => "contact",
        ContactPairingStart { .. }
        | ContactPairingComplete { .. }
        | ContactPairingRevoke { .. } => "contact_pairing",
        ContextDeliveries { .. } => "context_deliveries",
        ContextView { .. } => "context_view",
        MembershipSnapshot { .. } => "membership_snapshot",
        MembershipInvitations { .. } => "membership_invitations",
        Membership { request } => {
            use crate::membership::MembershipAction::*;
            if matches!(
                request.change,
                Add { .. } | Remove { .. } | UpdateRole { .. }
            ) {
                return None;
            }
            "membership"
        }
        MessageReceipts { .. } => "message_receipts",
        MessageReceipt {
            state: crate::receipts::ClientReceiptState::Delivered,
            ..
        } => "receipt_acknowledged",
        ReceiptPreferencesGet { .. } | ReceiptPreferencesSet { .. } => "receipt_preferences",
        Inbox { .. } => "inbox",
        ProviderStatus { .. } => "provider_status",
        CallStatus { .. } => "call",
        SipStatus { .. } => "sip_leg",
        AvailabilityGet { .. } | AvailabilitySet { .. } => "availability",
        ArtifactBegin { .. }
        | ArtifactAppend { .. }
        | ArtifactFinish { .. }
        | ArtifactDelete { .. }
        | ArtifactInspect { .. }
        | ArtifactLinkAppend { .. } => "artifact",
        ArtifactGet { .. } | ArtifactLinkGet { .. } => "artifact_chunk",
        ArtifactUploadLink { .. } | ArtifactDownloadLink { .. } => "artifact_link",
        SessionObservations { .. } => "session_observations",
        ConnectionPrepare { .. }
        | SessionControl { .. }
        | CallStart { .. }
        | CallEnd { .. }
        | SipDial { .. }
        | SipEnd { .. }
        | Create { .. }
        | Conversations { .. }
        | Send { .. }
        | History { .. }
        | Close { .. }
        | DurableOperation { .. }
        | Transition { .. }
        | ConversationSnapshot { .. }
        | ConversationEvents { .. }
        | RequestStatus { .. }
        | MessageReceipt {
            state: crate::receipts::ClientReceiptState::Read,
            ..
        }
        | SessionDecision { .. }
        | SessionInvite { .. }
        | SessionAccept { .. }
        | SessionReject { .. }
        | SessionCancel { .. }
        | SessionEnd { .. }
        | SessionGet { .. } => return None,
    })
}
