//! Typed application conveniences. No hidden retries or automatic logical IDs.
use crate::{ClientError, ConnectClient, Conversation, Event, Message, Operation, Outcome, Result};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendReceipt {
    pub receipt: connect_protocol::transitions::MutationReceipt,
    pub request_id: String,
    pub message_id: String,
    pub conversation_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub next: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxPage {
    pub events: Vec<Event>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestState {
    Accepted(Box<Outcome>),
    Rejected,
    NotFound,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerHealth {
    pub installation_id: String,
    pub schema: u32,
    pub profile: String,
    /// Running process observations, when supported by the server.
    pub readiness: Option<connect_protocol::Readiness>,
}

/// Metadata declared before a resumable artifact upload accepts bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactUpload {
    pub to: connect_protocol::messages::MessageAudience,
    pub name: String,
    pub media_type: String,
    pub size: u64,
    pub sha256: String,
    pub expires_at: Option<i64>,
}

impl ArtifactUpload {
    pub fn new(
        name: impl Into<String>,
        media_type: impl Into<String>,
        size: u64,
        sha256: impl Into<String>,
    ) -> Self {
        Self {
            to: connect_protocol::messages::MessageAudience::all(),
            name: name.into(),
            media_type: media_type.into(),
            size,
            sha256: sha256.into(),
            expires_at: None,
        }
    }

    /// Freeze the artifact audience at upload acceptance, including the owner.
    pub fn with_audience(mut self, to: connect_protocol::messages::MessageAudience) -> Self {
        self.to = to;
        self
    }
    /// Set an absolute Unix timestamp after which bytes are unavailable.
    pub fn with_expiry(mut self, expires_at: i64) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

impl ConnectClient {
    /// Invite one participant to a bounded media Session in a Conversation.
    pub async fn session_invite(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        participant_id: &str,
        media: Vec<String>,
    ) -> Result<crate::Session> {
        self.session_operation(Operation::SessionInvite {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            participant_id: participant_id.into(),
            media,
        })
        .await
    }
    /// Accept an invitation as its invited participant.
    pub async fn session_accept(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<crate::Session> {
        self.session_operation(Operation::SessionAccept {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            session_id: session_id.into(),
        })
        .await
    }
    /// Reject an invitation with an optional application reason.
    pub async fn session_reject(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        session_id: &str,
        reason: Option<String>,
    ) -> Result<crate::Session> {
        self.session_operation(Operation::SessionReject {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            session_id: session_id.into(),
            reason,
        })
        .await
    }
    /// Cancel an invitation as its initiator before it becomes active.
    pub async fn session_cancel(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<crate::Session> {
        self.session_operation(Operation::SessionCancel {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            session_id: session_id.into(),
        })
        .await
    }
    /// End a bounded Session without closing its persistent Conversation.
    pub async fn session_end(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        session_id: &str,
        reason: Option<String>,
    ) -> Result<crate::Session> {
        self.session_operation(Operation::SessionEnd {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            session_id: session_id.into(),
            reason,
        })
        .await
    }
    /// Read the current durable Session state under current authority.
    pub async fn session_get(
        &mut self,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<crate::Session> {
        self.session_operation(Operation::SessionGet {
            conversation_id: conversation_id.into(),
            session_id: session_id.into(),
        })
        .await
    }
    async fn session_operation(&mut self, operation: Operation) -> Result<crate::Session> {
        match self.request(operation).await? {
            Outcome::Session(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Originate an outbound SIP connection inside an accepted audio Session.
    pub async fn sip_dial(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        session_id: &str,
        destination: &str,
    ) -> Result<crate::SipLeg> {
        self.sip_operation(Operation::SipDial {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            session_id: session_id.into(),
            destination: destination.into(),
        })
        .await
    }
    /// Read the current durable state of an outbound SIP leg.
    pub async fn sip_status(
        &mut self,
        conversation_id: &str,
        sip_leg_id: &str,
    ) -> Result<crate::SipLeg> {
        self.sip_operation(Operation::SipStatus {
            conversation_id: conversation_id.into(),
            sip_leg_id: sip_leg_id.into(),
        })
        .await
    }
    /// Record termination intent for an outbound SIP leg.
    pub async fn sip_end(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        sip_leg_id: &str,
    ) -> Result<crate::SipLeg> {
        self.sip_operation(Operation::SipEnd {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            sip_leg_id: sip_leg_id.into(),
        })
        .await
    }
    async fn sip_operation(&mut self, operation: Operation) -> Result<crate::SipLeg> {
        match self.request(operation).await? {
            Outcome::SipLeg(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Publish the caller's expiring availability and optional route hint.
    pub async fn availability_set(
        &mut self,
        request_id: &str,
        state: crate::AvailabilityState,
        preferred_route: Option<String>,
        expires_at: i64,
    ) -> Result<crate::Availability> {
        match self
            .request(Operation::AvailabilitySet {
                request_id: request_id.into(),
                state,
                preferred_route,
                expires_at,
            })
            .await?
        {
            Outcome::Availability(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Read an authorized identity's fresh availability observation.
    pub async fn availability_get(&mut self, identity_id: &str) -> Result<crate::Availability> {
        match self
            .request(Operation::AvailabilityGet {
                identity_id: identity_id.into(),
            })
            .await?
        {
            Outcome::Availability(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Select a bounded, source-attributed Conversation context view.
    pub async fn context_view(
        &mut self,
        conversation_id: &str,
        limit: u32,
    ) -> Result<crate::ContextView> {
        match self
            .request(Operation::ContextView {
                conversation_id: conversation_id.into(),
                limit,
            })
            .await?
        {
            Outcome::ContextView(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Begin a resumable artifact upload without an expiry.
    pub async fn artifact_begin(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        name: &str,
        media_type: &str,
        size: u64,
        sha256: &str,
    ) -> Result<crate::Artifact> {
        self.artifact_begin_with_options(
            request_id,
            conversation_id,
            ArtifactUpload::new(name, media_type, size, sha256),
        )
        .await
    }
    /// Begin a resumable artifact upload with an optional absolute Unix expiry.
    pub async fn artifact_begin_with_options(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        upload: ArtifactUpload,
    ) -> Result<crate::Artifact> {
        self.artifact_operation(Operation::ArtifactBegin {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            to: upload.to,
            name: upload.name,
            media_type: upload.media_type,
            size: upload.size,
            sha256: upload.sha256,
            expires_at: upload.expires_at,
        })
        .await
    }
    pub async fn artifact_inspect(&mut self, cid: &str, id: &str) -> Result<crate::Artifact> {
        self.artifact_operation(Operation::ArtifactInspect {
            conversation_id: cid.into(),
            artifact_id: id.into(),
        })
        .await
    }
    pub async fn artifact_upload_link(
        &mut self,
        cid: &str,
        id: &str,
        expires_in: u32,
    ) -> Result<crate::ArtifactLink> {
        self.link_operation(Operation::ArtifactUploadLink {
            conversation_id: cid.into(),
            artifact_id: id.into(),
            expires_in,
        })
        .await
    }
    pub async fn artifact_download_link(
        &mut self,
        cid: &str,
        id: &str,
        message_id: &str,
        expires_in: u32,
    ) -> Result<crate::ArtifactLink> {
        self.link_operation(Operation::ArtifactDownloadLink {
            conversation_id: cid.into(),
            artifact_id: id.into(),
            message_id: message_id.into(),
            expires_in,
        })
        .await
    }
    async fn link_operation(&mut self, operation: Operation) -> Result<crate::ArtifactLink> {
        match self.request(operation).await? {
            Outcome::ArtifactLink(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn artifact_link_get(
        &mut self,
        link: &crate::ArtifactLink,
        offset: u64,
        limit: u32,
    ) -> Result<crate::ArtifactChunk> {
        match self
            .request(Operation::ArtifactLinkGet {
                conversation_id: link.conversation_id.clone(),
                artifact_id: link.artifact_id.clone(),
                link: link.path.clone(),
                offset,
                limit,
            })
            .await?
        {
            Outcome::ArtifactChunk(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn artifact_link_append(
        &mut self,
        request_id: &str,
        link: &crate::ArtifactLink,
        offset: u64,
        data: &str,
    ) -> Result<crate::Artifact> {
        self.artifact_operation(Operation::ArtifactLinkAppend {
            request_id: request_id.into(),
            conversation_id: link.conversation_id.clone(),
            artifact_id: link.artifact_id.clone(),
            link: link.path.clone(),
            offset,
            data: data.into(),
        })
        .await
    }
    /// Append one base64-encoded chunk at the exact current offset.
    pub async fn artifact_append(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        artifact_id: &str,
        offset: u64,
        data: &str,
    ) -> Result<crate::Artifact> {
        self.artifact_operation(Operation::ArtifactAppend {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            artifact_id: artifact_id.into(),
            offset,
            data: data.into(),
        })
        .await
    }
    /// Verify and atomically publish a completely uploaded artifact.
    pub async fn artifact_finish(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        artifact_id: &str,
    ) -> Result<crate::Artifact> {
        self.artifact_operation(Operation::ArtifactFinish {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            artifact_id: artifact_id.into(),
        })
        .await
    }
    /// Delete artifact bytes while retaining a durable deleted record.
    pub async fn artifact_delete(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        artifact_id: &str,
    ) -> Result<crate::Artifact> {
        self.artifact_operation(Operation::ArtifactDelete {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            artifact_id: artifact_id.into(),
        })
        .await
    }
    async fn artifact_operation(&mut self, operation: Operation) -> Result<crate::Artifact> {
        match self.request(operation).await? {
            Outcome::Artifact(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Read one authorized bounded artifact chunk.
    pub async fn artifact_get(
        &mut self,
        conversation_id: &str,
        artifact_id: &str,
        offset: u64,
        limit: u32,
    ) -> Result<crate::ArtifactChunk> {
        match self
            .request(Operation::ArtifactGet {
                conversation_id: conversation_id.into(),
                artifact_id: artifact_id.into(),
                offset,
                limit,
            })
            .await?
        {
            Outcome::ArtifactChunk(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn call_start(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        route_id: &str,
        destination: &str,
    ) -> Result<crate::Call> {
        self.call_operation(Operation::CallStart {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            route_id: route_id.into(),
            destination: destination.into(),
        })
        .await
    }
    pub async fn call_status(
        &mut self,
        conversation_id: &str,
        call_id: &str,
    ) -> Result<crate::Call> {
        self.call_operation(Operation::CallStatus {
            conversation_id: conversation_id.into(),
            call_id: call_id.into(),
        })
        .await
    }
    pub async fn call_end(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        call_id: &str,
    ) -> Result<crate::Call> {
        self.call_operation(Operation::CallEnd {
            request_id: request_id.into(),
            conversation_id: conversation_id.into(),
            call_id: call_id.into(),
        })
        .await
    }
    async fn call_operation(&mut self, operation: Operation) -> Result<crate::Call> {
        match self.request(operation).await? {
            Outcome::Call(c) => Ok(c),
            _ => Err(ClientError::Protocol),
        }
    }

    /// Persist request_id and inputs in the caller's state before this call.
    pub async fn create(
        &mut self,
        request_id: &str,
        participants: Vec<String>,
    ) -> Result<Conversation> {
        self.create_conversation(
            request_id,
            connect_protocol::conversations::CreateRequest::persistent(participants),
        )
        .await
    }
    /// Create with explicit conversation policy, metadata and membership roles.
    pub async fn create_conversation(
        &mut self,
        request_id: &str,
        mut request: connect_protocol::conversations::CreateRequest,
    ) -> Result<Conversation> {
        request.normalize().map_err(|_| ClientError::Protocol)?;
        match self
            .request(Operation::Create {
                request_id: request_id.into(),
                request,
            })
            .await?
        {
            Outcome::Conversation(c)
                if c.receipt
                    == Some(connect_protocol::transitions::MutationReceipt::committed(
                        request_id.into(),
                        c.id.clone(),
                    )) =>
            {
                Ok(c)
            }
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn list_conversations(
        &mut self,
        query: connect_protocol::conversations::ListQuery,
    ) -> Result<connect_protocol::conversations::ListPage> {
        match self.request(Operation::Conversations { query }).await? {
            Outcome::Conversations(page) => Ok(page),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn close_conversation_with_options(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        options: connect_protocol::conversations::CloseOptions,
    ) -> Result<Conversation> {
        match self
            .request(Operation::Close {
                request_id: request_id.into(),
                conversation_id: conversation_id.into(),
                options,
            })
            .await?
        {
            Outcome::Conversation(c)
                if c.id == conversation_id
                    && c.receipt
                        == Some(connect_protocol::transitions::MutationReceipt::committed(
                            request_id.into(),
                            c.id.clone(),
                        )) =>
            {
                Ok(c)
            }
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn change_membership(
        &mut self,
        request: connect_protocol::membership::MembershipRequest,
    ) -> Result<connect_protocol::membership::MembershipOutcome> {
        let reference = connect_protocol::transitions::MutationReceipt::committed(
            request.operation_id.clone(),
            request.conversation_id.clone(),
        );
        match self.request(Operation::Membership { request }).await? {
            Outcome::Membership(value)
                if value.receipt() == Some(&reference)
                    && value.conversation_id() == reference.conversation_id =>
            {
                Ok(value)
            }
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn membership_snapshot(
        &mut self,
        conversation_id: &str,
    ) -> Result<connect_protocol::membership::MembershipSnapshot> {
        match self
            .request(Operation::MembershipSnapshot {
                conversation_id: conversation_id.into(),
            })
            .await?
        {
            Outcome::MembershipSnapshot(value)
                if value.conversation_id == conversation_id
                    && value
                        .participants
                        .iter()
                        .all(|p| p.conversation_id == conversation_id) =>
            {
                Ok(value)
            }
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn membership_invitations(
        &mut self,
        after: Option<String>,
        limit: u32,
    ) -> Result<connect_protocol::membership::InvitationPage> {
        match self
            .request(Operation::MembershipInvitations { after, limit })
            .await?
        {
            Outcome::MembershipInvitations(value) => Ok(value),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Success proves durable acceptance, not recipient application processing.
    pub async fn send(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        body: &str,
    ) -> Result<SendReceipt> {
        self.send_message(
            request_id,
            conversation_id,
            connect_protocol::messages::MessageDraft::text_all(body),
        )
        .await
    }
    /// Send an explicit audience and typed body. The caller retains the logical request ID.
    pub async fn send_message(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        message: connect_protocol::messages::MessageDraft,
    ) -> Result<SendReceipt> {
        match self
            .request(Operation::Send {
                request_id: request_id.into(),
                conversation_id: conversation_id.into(),
                message,
            })
            .await?
        {
            Outcome::Accepted {
                receipt: Some(receipt),
                request_id: accepted_id,
                message_id,
                conversation_id: accepted_cid,
            } if accepted_id == request_id
                && accepted_cid == conversation_id
                && receipt.matches(request_id, conversation_id) =>
            {
                Ok(SendReceipt {
                    receipt,
                    request_id: accepted_id,
                    message_id,
                    conversation_id: accepted_cid,
                })
            }
            _ => Err(ClientError::Protocol),
        }
    }
    /// Page count is an upper bound; large entries may yield a shorter page.
    pub async fn history(
        &mut self,
        conversation_id: &str,
        after: Option<String>,
        limit: u32,
    ) -> Result<HistoryPage> {
        match self
            .request(Operation::History {
                conversation_id: conversation_id.into(),
                after,
                limit,
            })
            .await?
        {
            Outcome::History { messages, next } => Ok(HistoryPage { messages, next }),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Persist the cursor only after durably handling its returned events.
    pub async fn inbox(
        &mut self,
        cursor: Option<String>,
        limit: u32,
        wait_ms: u32,
    ) -> Result<InboxPage> {
        match self
            .request(Operation::Inbox {
                cursor,
                limit,
                wait_ms,
            })
            .await?
        {
            Outcome::Inbox { events, cursor } => Ok(InboxPage { events, cursor }),
            _ => Err(ClientError::Protocol),
        }
    }
    /// NotFound does not imply an uncertain in-flight request cannot commit.
    pub async fn request_status(&mut self, request_id: &str) -> Result<RequestState> {
        match self
            .request(Operation::RequestStatus {
                request_id: request_id.into(),
            })
            .await?
        {
            Outcome::RequestStatus { state, outcome } => match (state.as_str(), outcome) {
                ("accepted", Some(saved)) => Ok(RequestState::Accepted(saved)),
                ("rejected", None) => Ok(RequestState::Rejected),
                ("not_found", None) => Ok(RequestState::NotFound),
                _ => Err(ClientError::Protocol),
            },
            _ => Err(ClientError::Protocol),
        }
    }
    /// Closing blocks new messages while retaining history and prior receipts.
    pub async fn close_conversation(
        &mut self,
        request_id: &str,
        conversation_id: &str,
    ) -> Result<Conversation> {
        self.close_conversation_with_options(request_id, conversation_id, Default::default())
            .await
    }
    /// Most recent bounded page of provider deliveries for this conversation.
    pub async fn provider_status(
        &mut self,
        conversation_id: &str,
    ) -> Result<Vec<connect_protocol::ProviderDelivery>> {
        match self
            .request(Operation::ProviderStatus {
                conversation_id: conversation_id.into(),
            })
            .await?
        {
            Outcome::ProviderStatus { deliveries } => Ok(deliveries),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn health(&mut self) -> Result<ServerHealth> {
        match self.request(Operation::Health {}).await? {
            Outcome::Health {
                installation_id,
                schema,
                profile,
                readiness,
            } => Ok(ServerHealth {
                installation_id,
                schema,
                profile,
                readiness,
            }),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Cancellation stops this wait, not a transaction already accepted by the
    /// server. It closes this physical peer so abandoned waits cannot block later
    /// work. Reopen a client and recover with the original logical ID/cursor.
    pub async fn request_cancelled(
        &mut self,
        operation: Operation,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        let result = tokio::select! {biased;_ = cancel.cancelled()=>Err(ClientError::Cancelled),result=self.request(operation)=>result};
        if matches!(result, Err(ClientError::Cancelled)) {
            self.disconnect();
        }
        result
    }
}
