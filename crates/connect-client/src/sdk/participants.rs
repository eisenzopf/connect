//! Stable Participant handles; cached receipt observations carry no authority.
use super::*;
use connect_protocol::membership::{MembershipAction, MembershipOutcome, MembershipRequest};
#[derive(Clone)]
pub struct Participant {
    client: Client,
    id: ParticipantId,
    conversation_id: ConversationId,
    receipt: Option<Arc<ParticipantSnapshot>>,
    receipt_revision: Option<ConversationRevision>,
}
impl Participant {
    fn new(client: Client, id: ParticipantId, conversation_id: ConversationId) -> Self {
        Self {
            client,
            id,
            conversation_id,
            receipt: None,
            receipt_revision: None,
        }
    }
    pub fn id(&self) -> &ParticipantId {
        &self.id
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }
    /// Authorized receipt-view observation when read; never a current authority grant.
    pub fn receipt_snapshot(&self) -> Option<&ParticipantSnapshot> {
        self.receipt.as_deref()
    }
    pub fn receipt_conversation_revision(&self) -> Option<ConversationRevision> {
        self.receipt_revision
    }
    pub async fn refresh(&self, options: RequestOptions) -> Result<ParticipantSnapshot> {
        snapshot::read(&self.client, &self.conversation_id, options)
            .await?
            .conversation
            .participants
            .into_iter()
            .find(|p| p.id == self.id)
            .ok_or_else(|| Error::request(ErrorKind::ReceiptUnavailable, None, false))
    }
    pub async fn update(
        &self,
        role: String,
        expected_conversation_revision: ConversationRevision,
        options: MutationOptions,
    ) -> Result<Operation<Participant>> {
        self.client
            .membership(
                self.conversation_id.clone(),
                MembershipAction::UpdateRole {
                    participant_id: self.id.as_str().into(),
                    role,
                },
                expected_conversation_revision,
                self.client
                    .operation_kinds()
                    .conversation_participant_update,
                options,
            )
            .await
    }
    pub async fn remove(
        &self,
        expected_conversation_revision: ConversationRevision,
        options: MutationOptions,
    ) -> Result<Operation<Participant>> {
        self.client
            .membership(
                self.conversation_id.clone(),
                MembershipAction::Remove {
                    participant_id: self.id.as_str().into(),
                    reason: None,
                },
                expected_conversation_revision,
                self.client
                    .operation_kinds()
                    .conversation_participant_remove,
                options,
            )
            .await
    }
}
impl Conversation {
    pub fn participant_by_id(&self, id: ParticipantId) -> Participant {
        Participant::new(self.client().clone(), id, self.id().clone())
    }
    pub async fn add_participant(
        &self,
        identity_id: IdentityId,
        kind: IdentityKind,
        role: String,
        expected_revision: ConversationRevision,
        options: MutationOptions,
    ) -> Result<Operation<Participant>> {
        self.client()
            .membership(
                self.id().clone(),
                MembershipAction::Add {
                    identity_id: identity_id.as_str().into(),
                    kind,
                    role,
                },
                expected_revision,
                self.client().operation_kinds().conversation_participant_add,
                options,
            )
            .await
    }
}
impl Client {
    async fn membership(
        &self,
        conversation_id: ConversationId,
        change: MembershipAction,
        expected: ConversationRevision,
        kind: OperationKind<Participant>,
        options: MutationOptions,
    ) -> Result<Operation<Participant>> {
        let mut request = MembershipRequest {
            operation_id: options.operation_id.as_str().into(),
            conversation_id: conversation_id.as_str().into(),
            expected_revision: expected.get(),
            change,
        };
        request
            .normalize()
            .map_err(|_| Error::local(ErrorKind::Validation))?;
        let body = match request.change {
            MembershipAction::Add {
                identity_id,
                kind,
                role,
            } => serde_json::json!({"identity_id":identity_id,"kind":kind,"role":role}),
            MembershipAction::UpdateRole {
                participant_id,
                role,
            } => serde_json::json!({"participant_id":participant_id,"role":role}),
            MembershipAction::Remove { participant_id, .. } => {
                serde_json::json!({"participant_id":participant_id})
            }
            _ => return Err(Error::local(ErrorKind::Validation)),
        };
        self.submit_local(
            kind,
            OperationScope {
                conversation_id: Some(conversation_id),
                ..Default::default()
            },
            Some(expected.get()),
            body,
            options,
        )
        .await
    }
}
pub(super) fn decode(value: Value, client: &Client, key: &OperationKey) -> Result<Participant> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let connect_protocol::Outcome::Membership(MembershipOutcome::Participant {
        participant,
        revision,
        receipt: Some(receipt),
    }) = serde_json::from_value(value).map_err(|_| fail())?
    else {
        return Err(fail());
    };
    if key.scope.conversation_id.as_ref().map(|i| i.as_str())
        != Some(participant.conversation_id.as_str())
        || !receipt.matches(key.operation_id.as_str(), &participant.conversation_id)
    {
        return Err(fail());
    }
    let snapshot = ParticipantSnapshot::try_from(participant).map_err(|_| fail())?;
    let mut result = Participant::new(
        client.clone(),
        snapshot.id.clone(),
        snapshot.conversation_id.clone(),
    );
    result.receipt = Some(Arc::new(snapshot));
    result.receipt_revision = Some(ConversationRevision::try_from(revision).map_err(|_| fail())?);
    Ok(result)
}
