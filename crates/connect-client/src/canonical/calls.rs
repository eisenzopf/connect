//! Typed calls over the signed, correlated batch transport.
use super::*;
use crate::{conversations, MessageDraft, Outcome, SendReceipt};

pub struct CanonicalHistoryPage {
    /// Canonical observations identify sender/recipients by Participant IDs.
    /// They do not manufacture HTTP Identity mappings or membership epochs.
    pub messages: Vec<payloads::MessageSend>,
    pub next_cursor: Option<String>,
}
macro_rules! canonical_calls { ($client:ty) => { impl $client {
    /// Apply a Session control decision. A completed local receipt does not
    /// establish a ready Connection or promise physical media completion.
    pub async fn session_command(&mut self, mut request: connect_protocol::session_control::Request) -> Result<Outcome> {
        use connect_protocol::session_control::Action;
        request.normalize()?;
        let body=match &request.action {
            Action::Invite{participants,medium,intent,capabilities}=>json!({"from":request.by_participant,"to":participants,"medium":medium,"intent":intent,"capabilities_offer":capabilities}),
            Action::Accept{capabilities}=>json!({"by":request.by_participant,"capabilities_answer":capabilities}),
            Action::Reject{reason_code,reason}|Action::Cancel{reason_code,reason}|Action::End{reason_code,reason}=>json!({"by":request.by_participant,"reason_code":reason_code,"reason":reason}),
            Action::Leave{participant_id}=>json!({"kind":"participant-leave","details":{"participant_id":participant_id}}),
            Action::Role{participant_id,role}=>json!({"kind":"role-change","details":{"participant_id":participant_id,"role":role}}),
            Action::Upgrade{media}=>json!({"kind":"medium-upgrade","details":{"media":media}}),
            Action::Renegotiate{codec_preferences}=>json!({"kind":"codec-renegotiate","details":{"codec_preferences":codec_preferences}}),
        };
        let batch=self.request(typed(request.action.required_scope(),body)?,CommandScope {
            operation_id:Some(request.operation_id.clone()),conversation_id:Some(request.conversation_id.clone()),session_id:Some(request.session_id.clone()),expected_revision:request.expected_revision,..Default::default()
        }).await?;
        let update=operation_reply(&batch,&request.operation_id)?;
        if serde_json::to_value(update.state).map_err(|_|ClientError::Protocol)?!=json!("completed")||update.problem.is_some(){return Err(ClientError::Protocol);}
        let outcome:Outcome=serde_json::from_value(update.result.ok_or(ClientError::Protocol)?).map_err(|_|ClientError::Protocol)?;
        let revision=match &outcome {
            Outcome::SessionControl(result)=>{
                if matches!(request.action,Action::Reject{..}|Action::Cancel{..})||result.operation_id!=request.operation_id||result.action!=request.action.name()||result.session.id!=request.session_id||result.session.conversation_id!=request.conversation_id{return Err(ClientError::Protocol);}
                result.session.topology_revision
            }
            Outcome::SessionDecision(result)=>{
                let state=match request.action {Action::Reject{..}=>"rejected",Action::Cancel{..}=>"cancelled",_=>return Err(ClientError::Protocol)};
                let (code,reason)=request.action.reason();
                if result.operation_id!=request.operation_id||result.conversation_id!=request.conversation_id||result.session_id!=request.session_id||result.state!=state||result.reason_code!=code||result.reason!=reason{return Err(ClientError::Protocol);}
                result.revision
            }
            _=>return Err(ClientError::Protocol),
        };
        if revision>9_007_199_254_740_991||request.expected_revision.is_some_and(|v|v.checked_add(1)!=Some(revision)){return Err(ClientError::Protocol);}
        Ok(outcome)
    }

    /// Decide an existing invitation under its original logical operation ID.
    /// This changes pending invitation state, not an established media route.
    pub async fn decide_session_invitation(
        &mut self,
        request: connect_protocol::session_decisions::Request,
    ) -> Result<connect_protocol::session_decisions::DecisionOutcome> {
        request.validate()?;
        let batch = self
            .request(
                typed(
                    request.decision.required_scope(),
                    json!({"by":request.by_participant,
                "reason_code":request.reason_code,"reason":request.reason}),
                )?,
                CommandScope {
                    operation_id: Some(request.operation_id.clone()),
                    conversation_id: Some(request.conversation_id.clone()),
                    session_id: Some(request.session_id.clone()),
                    expected_revision: request.expected_revision,
                    ..Default::default()
                },
            )
            .await?;
        let update = operation_reply(&batch, &request.operation_id)?;
        if serde_json::to_value(update.state).map_err(|_| ClientError::Protocol)?
            != json!("completed")
            || update.problem.is_some()
        {
            return Err(ClientError::Protocol);
        }
        let Outcome::SessionDecision(outcome) =
            serde_json::from_value(update.result.ok_or(ClientError::Protocol)?)
                .map_err(|_| ClientError::Protocol)?
        else {
            return Err(ClientError::Protocol);
        };
        let state = match request.decision {
            connect_protocol::session_decisions::Decision::Reject => "rejected",
            connect_protocol::session_decisions::Decision::Cancel => "cancelled",
        };
        if outcome.operation_id != request.operation_id
            || outcome.conversation_id != request.conversation_id
            || outcome.session_id != request.session_id
            || outcome.state != state
            || outcome.reason_code != request.reason_code
            || outcome.reason != request.reason
            || outcome.revision == 0
            || outcome.revision > 9_007_199_254_740_991
            || request
                .expected_revision
                .is_some_and(|v| v.checked_add(1) != Some(outcome.revision))
        {
            return Err(ClientError::Protocol);
        }
        Ok(outcome)
    }
    pub async fn create_conversation(
        &mut self,
        operation_id: &str,
        mut input: conversations::CreateRequest,
    ) -> Result<crate::Conversation> {
        input.normalize()?;
        if input.tenant_id.is_none() {
            return Err(invalid("Canonical create requires an explicit tenant"));
        }
        let batch = self
            .request(
                typed(
                    "conversation.create",
                    serde_json::to_value(input).map_err(|_| ClientError::Protocol)?,
                )?,
                operation_scope(None, operation_id),
            )
            .await?;
        conversation(&batch, operation_id, None, false)
    }
    pub async fn close_conversation(
        &mut self,
        operation_id: &str,
        cid: &str,
        options: conversations::CloseOptions,
    ) -> Result<crate::Conversation> {
        options.validate()?;
        let scope = CommandScope {
            expected_revision: options.expected_revision,
            ..operation_scope(Some(cid), operation_id)
        };
        let batch = self
            .request(
                typed(
                    "conversation.close",
                    json!({"reason_code":options.reason_code,"reason":options.reason}),
                )?,
                scope,
            )
            .await?;
        conversation(&batch, operation_id, Some(cid), true)
    }
    pub async fn list_conversations(
        &mut self,
        query: conversations::ListQuery,
    ) -> Result<conversations::ListPage> {
        query.validate()?;
        let batch = self
            .request(
                typed(
                    "conversation.list",
                    serde_json::to_value(query).map_err(|_| ClientError::Protocol)?,
                )?,
                Default::default(),
            )
            .await?;
        let page: conversations::ListPage = serde_json::from_value(
            batch
                .ack()?
                .details
                .as_ref()
                .and_then(|v| v.get("page"))
                .cloned()
                .ok_or(ClientError::Protocol)?,
        )
        .map_err(|_| ClientError::Protocol)?;
        let mut facts = batch.frames[..batch.frames.len() - 1].iter();
        for item in &page.conversations {
            for kind in std::iter::once("conversation.opened")
                .chain(item.closed.then_some("conversation.closed"))
            {
                if !facts.next().is_some_and(|f| {
                    f.kind() == kind && f.scope(Scope::Conversation) == Some(item.id.as_str())
                }) {
                    return Err(ClientError::Protocol);
                }
            }
        }
        if facts.next().is_some() || batch.ack()?.next_cursor != page.next_cursor {
            return Err(ClientError::Protocol);
        }
        Ok(page)
    }
    /// Both IDs belong to the caller's persisted logical intent; neither is
    /// generated to conceal uncertainty or to resolve a conflicting retry.
    pub async fn send_message(
        &mut self,
        operation_id: &str,
        cid: &str,
        message_id: &str,
        mut draft: MessageDraft,
    ) -> Result<SendReceipt> {
        if draft.msg_id.as_deref().is_some_and(|id| id != message_id) {
            return Err(invalid("Message identity conflicts with the saved draft"));
        }
        draft.msg_id = Some(message_id.into());
        draft.normalize()?;
        let batch = self
            .request(
                typed(
                    "message.send",
                    serde_json::to_value(draft).map_err(|_| ClientError::Protocol)?,
                )?,
                operation_scope(Some(cid), operation_id),
            )
            .await?;
        let result: Outcome =
            serde_json::from_value(batch.ack()?.details.clone().ok_or(ClientError::Protocol)?)
                .map_err(|_| ClientError::Protocol)?;
        let Outcome::Accepted {
            receipt: Some(receipt),
            request_id,
            message_id: accepted,
            conversation_id,
        } = result
        else {
            return Err(ClientError::Protocol);
        };
        if request_id != operation_id
            || accepted != message_id
            || conversation_id != cid
            || !receipt.matches(operation_id, cid)
        {
            return Err(ClientError::Protocol);
        }
        Ok(SendReceipt {
            receipt,
            request_id,
            message_id: accepted,
            conversation_id,
        })
    }
    pub async fn history(
        &mut self,
        cid: &str,
        query: payloads::MessageHistory,
    ) -> Result<CanonicalHistoryPage> {
        let batch = self
            .request(Payload::MessageHistory(query), resource_scope(cid))
            .await?;
        let cursor = batch.ack()?.next_cursor.clone();
        let mut messages = Vec::new();
        for frame in batch.frames {
            match frame.payload() {
                Payload::MessageSend(value) if value.r#from.is_some() => {
                    messages.push(value.clone())
                }
                Payload::Ack(_) => {}
                _ => return Err(ClientError::Protocol),
            }
        }
        Ok(CanonicalHistoryPage {
            messages,
            next_cursor: cursor,
        })
    }
    pub async fn conversation_snapshot(
        &mut self,
        cid: &str,
        query: payloads::ConversationGet,
    ) -> Result<payloads::ConversationSnapshot> {
        let batch = self
            .request(Payload::ConversationGet(query), resource_scope(cid))
            .await?;
        batch.ack()?;
        let mut result = None;
        for frame in &batch.frames {
            match frame.payload() {
                Payload::ConversationSnapshot(value) if result.is_none() => {
                    result = Some(value.clone())
                }
                Payload::Ack(_) => {}
                _ => return Err(ClientError::Protocol),
            }
        }
        result.ok_or(ClientError::Protocol)
    }
    /// Discover Session records through current authorized snapshot metadata.
    /// UUID/revision hints do not grant retrieval or expose a canonical digest.
    pub async fn conversation_records(
        &mut self, cid: &str, query: payloads::ConversationGet,
    ) -> Result<CanonicalRecordPage> {
        records::discovery(self.conversation_snapshot(cid,query).await?)
    }
    pub async fn subscribe_conversation(
        &mut self,
        cid: &str,
        query: payloads::ConversationSubscribe,
    ) -> Result<ReplyBatch> {
        let batch = self
            .request(Payload::ConversationSubscribe(query), resource_scope(cid))
            .await?;
        batch.ack()?;
        Ok(batch)
    }
    /// Returns the observed canonical state, including unknown or in-progress
    /// state. It does not turn an ACK into a completed logical operation.
    pub async fn operation_get(
        &mut self,
        operation_id: &str,
        scope: CommandScope,
    ) -> Result<payloads::OperationUpdated> {
        let payload = typed("operation.get", json!({"operation_id":operation_id}))?;
        let batch = self.request(payload, scope).await?;
        operation_reply(&batch, operation_id)
    }
    /// Submit one durable consult/handoff/device-replacement intent. Accepted
    /// state reserves work; it does not establish ready media or completion.
    pub async fn transition(
        &mut self,
        mut command: crate::transitions::TransitionCommand,
    ) -> Result<payloads::OperationUpdated> {
        command.normalize()?;
        let mut value = serde_json::to_value(&command.intent).map_err(|_| ClientError::Protocol)?;
        value
            .as_object_mut()
            .ok_or(ClientError::Protocol)?
            .retain(|key, value| key != "type" && !value.is_null());
        value.as_object_mut().ok_or(ClientError::Protocol)?.extend(command.payload_extensions);
        let batch = self
            .request_value(
                command.intent.required_scope(),value,
                CommandScope {
                    operation_id: Some(command.operation_id.clone()),
                    conversation_id: Some(command.conversation_id),
                    session_id: Some(command.session_id),
                    connection_id: command.connection_id,
                    expected_revision: Some(command.expected_revision),
                },
            )
            .await?;
        operation_reply(&batch, &command.operation_id)
    }
    pub async fn report_read(
        &mut self,
        operation_id: &str,
        cid: &str,
        message_id: &str,
        read_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<crate::receipts::ReceiptAcknowledgement> {
        let batch = self
            .request(
                typed(
                    "message.read",
                    json!({"msg_id":message_id,"read_at":read_at}),
                )?,
                operation_scope(Some(cid), operation_id),
            )
            .await?;
        let ack = batch.ack()?;
        let records: Vec<_> = batch
            .frames
            .iter()
            .filter_map(|f| {
                if let Payload::OperationUpdated(v) = f.payload() {
                    Some(v)
                } else {
                    None
                }
            })
            .collect();
        if records.len() != 1
            || batch.frames.len() != 2
            || records[0].operation_id.as_str() != operation_id
            || serde_json::to_value(records[0].state).map_err(|_| ClientError::Protocol)?
                != json!("completed")
        {
            return Err(ClientError::Protocol);
        }
        let Outcome::ReceiptAcknowledged(value) =
            serde_json::from_value(records[0].result.clone().ok_or(ClientError::Protocol)?)
                .map_err(|_| ClientError::Protocol)?
        else {
            return Err(ClientError::Protocol);
        };
        let details = ack.details.as_ref().ok_or(ClientError::Protocol)?;
        if value.conversation_id != cid
            || value.message_id != message_id
            || details["message_id"] != value.message_id
            || details["participant_id"] != value.participant_id
            || details["membership_epoch"] != value.membership_epoch
            || details["recorded"]
                != serde_json::to_value(value.recorded).map_err(|_| ClientError::Protocol)?
        {
            return Err(ClientError::Protocol);
        }
        Ok(value)
    }
    /// Add/update/remove use canonical Participant IDs and current revision
    /// assertions. Other membership administration remains in its named facade.
    pub async fn participant_change(
        &mut self,
        cid: &str,
        operation_id: &str,
        expected_revision: Option<u64>,
        payload: Payload,
    ) -> Result<crate::membership::MembershipOutcome> {
        let kind = match &payload {
            Payload::ConversationParticipantAdd(_) => "added",
            Payload::ConversationParticipantUpdate(_) => "updated",
            Payload::ConversationParticipantRemove(_) => "removed",
            _ => return Err(invalid("Not a participant mutation")),
        };
        let batch = self
            .request(
                payload,
                CommandScope {
                    expected_revision,
                    ..operation_scope(Some(cid), operation_id)
                },
            )
            .await?;
        batch.ack()?;
        let records: Vec<_> = batch
            .frames
            .iter()
            .filter_map(|f| {
                if let Payload::OperationUpdated(v) = f.payload() {
                    Some(v)
                } else {
                    None
                }
            })
            .collect();
        if records.len() != 1
            || batch.frames.len() != 3
            || records[0].operation_id.as_str() != operation_id
            || serde_json::to_value(records[0].state).map_err(|_| ClientError::Protocol)?
                != json!("completed")
        {
            return Err(ClientError::Protocol);
        }
        let Outcome::Membership(result) =
            serde_json::from_value(records[0].result.clone().ok_or(ClientError::Protocol)?)
                .map_err(|_| ClientError::Protocol)?
        else {
            return Err(ClientError::Protocol);
        };
        let crate::membership::MembershipOutcome::Participant { participant, .. } = &result else {
            return Err(ClientError::Protocol);
        };
        let notice = batch.frames[1].original();
        if batch.frames[0].kind() != "operation.updated"
            || batch.frames[1].kind() != "conversation.participant.changed"
            || notice["payload"]["kind"] != kind
            || notice["payload"]["participant_id"] != participant.participant_id
            || notice["payload"]["membership_epoch"] != participant.membership_epoch
            || result.conversation_id() != cid
            || !result
                .receipt()
                .is_some_and(|r| r.matches(operation_id, cid))
        {
            return Err(ClientError::Protocol);
        }
        Ok(result)
    }
}
}; }
canonical_calls!(CanonicalClient);
canonical_calls!(super::http::CanonicalHttpClient);

fn conversation(
    batch: &ReplyBatch,
    operation_id: &str,
    cid: Option<&str>,
    close: bool,
) -> Result<crate::Conversation> {
    let value: crate::Conversation = serde_json::from_value(
        batch
            .ack()?
            .details
            .as_ref()
            .and_then(v_get_conversation)
            .cloned()
            .ok_or(ClientError::Protocol)?,
    )
    .map_err(|_| ClientError::Protocol)?;
    if cid.is_some_and(|cid| cid != value.id)
        || !value
            .receipt
            .as_ref()
            .is_some_and(|r| r.matches(operation_id, &value.id))
    {
        return Err(ClientError::Protocol);
    }
    let facts = &batch.frames[..batch.frames.len() - 1];
    let expected: Vec<&str> = if close {
        vec!["conversation.closed"]
    } else if value.closed {
        vec!["conversation.opened", "conversation.closed"]
    } else {
        vec!["conversation.opened"]
    };
    if facts.len() != expected.len()
        || facts.iter().zip(expected).any(|(f, kind)| {
            f.kind() != kind || f.scope(Scope::Conversation) != Some(value.id.as_str())
        })
    {
        return Err(ClientError::Protocol);
    }
    Ok(value)
}
fn v_get_conversation(value: &Value) -> Option<&Value> {
    value.get("conversation")
}
fn operation_scope(cid: Option<&str>, id: &str) -> CommandScope {
    CommandScope {
        conversation_id: cid.map(str::to_owned),
        operation_id: Some(id.into()),
        ..Default::default()
    }
}
fn resource_scope(cid: &str) -> CommandScope {
    CommandScope {
        conversation_id: Some(cid.into()),
        ..Default::default()
    }
}
pub(super) fn typed(kind: &str, value: Value) -> Result<Payload> {
    serde_json::from_value(json!({"type":kind,"payload":value})).map_err(|_| ClientError::Protocol)
}
pub(super) fn invalid(message: &str) -> ClientError {
    ClientError::Application(connect_protocol::Error::new("invalid-request", message))
}

pub(super) fn operation_reply(
    batch: &ReplyBatch,
    operation_id: &str,
) -> Result<payloads::OperationUpdated> {
    batch.ack()?;
    let mut record = None;
    for frame in &batch.frames {
        match frame.payload() {
            Payload::OperationUpdated(value)
                if record.is_none() && value.operation_id.as_str() == operation_id =>
            {
                record = Some(value.clone())
            }
            Payload::Ack(_) => {}
            _ => return Err(ClientError::Protocol),
        }
    }
    record.ok_or(ClientError::Protocol)
}
