//! Conversation handles retain IDs; every refresh/mutation uses current authority.
use super::*;
use connect_protocol::Outcome;
#[derive(Clone)]
pub struct Conversation {
    client: Client,
    id: ConversationId,
}
impl Conversation {
    pub(super) fn new(client: Client, id: ConversationId) -> Self {
        Self { client, id }
    }
    pub fn id(&self) -> &ConversationId {
        &self.id
    }
    pub async fn send(
        &self,
        message_id: MessageId,
        to: NonEmpty<ParticipantId>,
        text: String,
        options: MutationOptions,
    ) -> Result<Operation<MessageAcceptance>> {
        let mut draft = crate::MessageDraft {
            msg_id: Some(message_id.as_str().into()),
            to: crate::MessageAudience::Participants(
                to.into_vec().into_iter().map(String::from).collect(),
            ),
            content_type: "text/plain".into(),
            body: text,
            body_encoding: crate::BodyEncoding::Utf8,
            attachments: vec![],
            label: "connect-messages".into(),
            in_reply_to_msg: None,
        };
        draft
            .normalize()
            .map_err(|_| Error::local(ErrorKind::Validation))?;
        let scope = OperationScope {
            conversation_id: Some(self.id.clone()),
            ..Default::default()
        };
        let kind = self.client.operation_kinds().message_send;
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope,
        };
        let body = extend(
            serde_json::to_value(draft).map_err(|_| Error::local(ErrorKind::Validation))?,
            options.extensions,
        )?;
        let batch = self
            .client
            .request(
                kind.command,
                body,
                key.scope.command(Some(&key.operation_id)),
                Some(&key),
                options.request,
            )
            .await?;
        let result = batch
            .ack()
            .map_err(|e| Error::transport(e, Some(&key)))?
            .details
            .clone()
            .ok_or_else(|| Error::request(ErrorKind::Protocol, Some(&key), true))?;
        let accepted = decode_message(result, &self.client, &key)?;
        if accepted.message_id != message_id {
            return Err(Error::request(ErrorKind::Protocol, Some(&key), true));
        }
        Ok(Operation::new(
            self.client.clone(),
            OperationReference {
                key,
                kind,
                continuation: None,
            },
        ))
    }
    pub async fn refresh(&self, options: RequestOptions) -> Result<ConversationSnapshot> {
        Ok(snapshot::read(&self.client, &self.id, options)
            .await?
            .conversation)
    }
    pub(super) fn client(&self) -> &Client {
        &self.client
    }
}
impl Client {
    pub async fn create_conversation(
        &self,
        participants: Vec<IdentityId>,
        policy: ConversationPolicy,
        options: MutationOptions,
    ) -> Result<Operation<Conversation>> {
        let mut input = crate::conversations::CreateRequest::persistent(
            participants.into_iter().map(String::from).collect(),
        );
        input.tenant_id = Some(self.tenant_id.as_str().into());
        input.policy = policy;
        input
            .normalize()
            .map_err(|_| Error::local(ErrorKind::Validation))?;
        let kind = self.operation_kinds().conversation_create;
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: OperationScope::default(),
        };
        let body = extend(
            serde_json::to_value(input).map_err(|_| Error::local(ErrorKind::Validation))?,
            options.extensions,
        )?;
        let batch = self
            .request(
                kind.command,
                body,
                key.scope.command(Some(&key.operation_id)),
                Some(&key),
                options.request,
            )
            .await?;
        let body = batch
            .ack()
            .map_err(|e| Error::transport(e, Some(&key)))?
            .details
            .as_ref()
            .and_then(|v| v.get("conversation"))
            .cloned()
            .ok_or_else(|| Error::request(ErrorKind::Protocol, Some(&key), true))?;
        let room: crate::Conversation = serde_json::from_value(body)
            .map_err(|_| Error::request(ErrorKind::Protocol, Some(&key), true))?;
        validate_conversation(&room, self, &key)?;
        Ok(Operation::new(
            self.clone(),
            OperationReference {
                key,
                kind,
                continuation: None,
            },
        ))
    }
}
#[derive(Clone, Debug)]
pub struct ParticipantSnapshot {
    pub id: ParticipantId,
    pub conversation_id: ConversationId,
    pub identity_id: IdentityId,
    pub kind: connect_protocol::provisioning::IdentityKind,
    pub role: String,
    pub membership_epoch: u64,
    pub active: bool,
    pub removal_reason: Option<String>,
}
impl TryFrom<connect_protocol::membership::MemberView> for ParticipantSnapshot {
    type Error = Error;
    fn try_from(v: connect_protocol::membership::MemberView) -> Result<Self> {
        Ok(Self {
            id: v.participant_id.parse()?,
            conversation_id: v.conversation_id.parse()?,
            identity_id: v.identity_id.parse()?,
            kind: v.kind,
            role: v.role,
            membership_epoch: v.membership_epoch,
            active: v.active,
            removal_reason: v.removal_reason,
        })
    }
}
#[derive(Clone, Debug)]
pub struct ConversationSnapshot {
    pub id: ConversationId,
    pub revision: ConversationRevision,
    pub participants: Vec<ParticipantSnapshot>,
    pub closed: bool,
    pub cursor: String,
}
pub struct MessageAcceptance {
    pub message_id: MessageId,
    pub operation: OperationKey,
    pub boundary: connect_protocol::transitions::LocalMutationBoundary,
}
pub(super) fn decode_message(
    value: Value,
    _client: &Client,
    key: &OperationKey,
) -> Result<MessageAcceptance> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let result: Outcome = serde_json::from_value(value).map_err(|_| fail())?;
    let Outcome::Accepted {
        receipt: Some(receipt),
        request_id,
        message_id,
        conversation_id,
    } = result
    else {
        return Err(fail());
    };
    if request_id != key.operation_id.as_str()
        || key.scope.conversation_id.as_ref().map(|i| i.as_str()) != Some(conversation_id.as_str())
        || !receipt.matches(&request_id, &conversation_id)
    {
        return Err(fail());
    }
    Ok(MessageAcceptance {
        message_id: message_id.parse().map_err(|_| fail())?,
        operation: key.clone(),
        boundary: receipt.completion_boundary,
    })
}
pub(super) fn decode_conversation(
    value: Value,
    client: &Client,
    key: &OperationKey,
) -> Result<Conversation> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let Outcome::Conversation(room) = serde_json::from_value(value).map_err(|_| fail())? else {
        return Err(fail());
    };
    validate_conversation(&room, client, key)?;
    Ok(Conversation::new(
        client.clone(),
        room.id.parse().map_err(|_| fail())?,
    ))
}
fn validate_conversation(
    room: &crate::Conversation,
    client: &Client,
    key: &OperationKey,
) -> Result<()> {
    if !room
        .receipt
        .as_ref()
        .is_some_and(|r| r.matches(key.operation_id.as_str(), &room.id))
        || room.tenant_id != client.tenant_id.as_str()
    {
        return Err(Error::request(ErrorKind::Protocol, Some(key), true));
    }
    Ok(())
}
pub(super) fn extend(
    mut body: Value,
    extensions: Option<std::collections::BTreeMap<String, Value>>,
) -> Result<Value> {
    let object = body
        .as_object_mut()
        .ok_or_else(|| Error::local(ErrorKind::Validation))?;
    for (key, value) in extensions.unwrap_or_default() {
        if object.contains_key(&key) {
            return Err(Error::local(ErrorKind::Validation));
        }
        object.insert(key, value);
    }
    Ok(body)
}
