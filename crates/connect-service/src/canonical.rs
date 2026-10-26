//! Canonical commands use the same durable writer and current visibility as HTTP.
use crate::Service;
use async_trait::async_trait;
use connect_protocol::{Error, Message, Outcome};
use rvoip_uctp::v2::{
    auth_dispatch::{
        AuthenticatedHandler, AuthorizedEnvelope, DispatchError, HandlerReplies,
        MAX_HANDLER_REPLIES,
    },
    scope_policy, Envelope, Payload, Scope,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// Trusted transport composition. The authenticated dispatcher must supply the
/// principal, signature proof and current authority lease; request fields cannot.
/// Unsupported command families fail explicitly without stock-core fallthrough.
pub struct CanonicalHandler {
    pub(crate) physical: Option<Arc<crate::canonical_runtime::Physical>>,
    pub(crate) service: Arc<Service>,
    pub(crate) subscriptions: Option<Arc<crate::canonical_subscriptions::Subscriptions>>,
}
impl CanonicalHandler {
    pub fn new(service: Arc<Service>) -> Arc<Self> {
        Arc::new(Self {
            service,
            subscriptions: None,
            physical: None,
        })
    }
    async fn authorize(&self, authorized: &AuthorizedEnvelope) -> connect_protocol::Result<String> {
        let authority = authorized
            .authority::<crate::StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        self.authorize_with(authorized, authority).await
    }
    async fn authorize_with(
        &self,
        authorized: &AuthorizedEnvelope,
        authority: &crate::StoreInvocationAuthority,
    ) -> connect_protocol::Result<String> {
        let request = authorized.envelope();
        if self
            .service
            .assurance_policy
            .get()
            .is_some_and(|policy| !policy.permits(request, authorized.principal()))
        {
            return Err(Error::forbidden());
        }
        if request.scope(Scope::Reply).is_some()
            && !crate::canonical_sessions::correlated_cancel(request)
        {
            return Err(Error::forbidden());
        }
        if !authority.peer_guard().is_some_and(|proof| proof.current()) {
            return Err(Error::forbidden());
        }
        let credential = self
            .service
            .authorize_local_principal(
                authorized.principal(),
                Some(authority.binding()),
                Some(connect_protocol::REQUIRED_SCOPE),
            )
            .await?;
        if scope_policy::required_scope(request.payload())
            .is_some_and(|scope| !authorized.principal().has_scope(scope))
        {
            return Err(Error::forbidden());
        }
        if let Some(physical) = &self.physical {
            physical.validate(authorized, authority)?;
        }
        if crate::canonical_sessions::correlated_cancel(request) {
            let runtime = self
                .service
                .canonical_runtime
                .get()
                .ok_or_else(Error::forbidden)?;
            if !runtime
                .invitation_callers
                .lock()
                .map_err(|_| Error::storage())?
                .owns_cancellation(authorized)
            {
                return Err(Error::forbidden());
            }
        }
        Ok(credential)
    }
    async fn execute(
        &self,
        authorized: &AuthorizedEnvelope,
    ) -> connect_protocol::Result<Vec<Envelope>> {
        let credential = self.authorize(authorized).await?;
        let request = authorized.envelope();
        let authority = authorized
            .authority::<crate::StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        self.execute_command(request, credential, Some(authority), Some(authorized))
            .await
    }

    pub(crate) async fn execute_command(
        &self,
        request: &Envelope,
        credential: String,
        authority: Option<&crate::StoreInvocationAuthority>,
        authorized: Option<&AuthorizedEnvelope>,
    ) -> connect_protocol::Result<Vec<Envelope>> {
        if crate::canonical_sessions::correlated_cancel(request) && authorized.is_none() {
            return Err(Error::forbidden());
        }
        if matches!(
            request.payload(),
            Payload::ConnectionOffer(_)
                | Payload::ConnectionEnd(_)
                | Payload::ConnectionUpdate(_)
                | Payload::CapabilityAdvertise(_)
                | Payload::DtmfSend(_)
                | Payload::StreamSubscribe(_)
                | Payload::StreamUnsubscribe(_)
        ) || (matches!(request.payload(), Payload::MessageSend(_))
            && (request.scope(Scope::Session).is_some()
                || request.scope(Scope::Connection).is_some()))
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "Native connection control and scoped data publication require their physical peer",
            ));
        }
        if let Some(command) = crate::canonical_receipts::command(request)? {
            let record = self
                .service
                .canonical_read_receipt_held(credential, command, authority.map(|a| a.retained()))
                .await?;
            let outcome = record.result.clone().ok_or_else(Error::storage)?;
            let Outcome::ReceiptAcknowledged(receipt) =
                serde_json::from_value(outcome).map_err(|_| Error::storage())?
            else {
                return Err(Error::storage());
            };
            return Ok(vec![
                reply(
                    request,
                    "operation.updated",
                    json!({"operation_id":record.operation_id,"state":record.state,"revision":record.revision,"result":record.result}),
                )?,
                reply(
                    request,
                    "ack",
                    json!({"details":{"message_id":receipt.message_id,"participant_id":receipt.participant_id,"membership_epoch":receipt.membership_epoch,"recorded":receipt.recorded}}),
                )?,
            ]);
        }
        if let Some(action) = crate::canonical_operations::action(request)? {
            let record = self
                .service
                .canonical_operation_held(
                    credential,
                    connect_store::PeerOperationClaim {
                        action,
                        conversation_id: request.scope(Scope::Conversation).map(str::to_owned),
                        session_id: request.scope(Scope::Session).map(str::to_owned),
                        connection_id: request.scope(Scope::Connection).map(str::to_owned),
                    },
                    authority.map(|a| a.retained()),
                )
                .await?;
            let mut payload = json!({"operation_id":record.operation_id,"record_id":record.record_id,"command_type":record.command_type,"expected_revision":record.expected_revision,"state":record.state,"revision":record.revision});
            if let Some(subscription) = record.subscription_record_id {
                payload["subscription_record_id"] = json!(subscription);
            }
            if let Some(result) = record.result {
                payload["result"] = result;
            }
            if let Some(problem) = record.problem {
                payload["problem"] = problem;
            }
            return Ok(vec![reply(request, "operation.updated", payload)?]);
        }
        if let Some(command) = crate::canonical_sessions::command(request)? {
            let initialization = command.clone();
            let initialization_credential = credential.clone();
            let record = self
                .service
                .canonical_session_held(
                    credential,
                    command,
                    authority.map(|a| a.retained()),
                    authorized.and_then(|a| a.peer_admission().cloned()),
                    authorized.map(|a| a.principal().clone()),
                )
                .await?;
            if matches!(
                initialization.action,
                connect_protocol::session_control::Action::Invite { .. }
            ) && record.state == connect_protocol::transitions::OperationState::Completed
            {
                let caller = authorized
                    .map(crate::canonical_invitations::Caller::capture)
                    .transpose()?;
                if let Some(caller) = &caller {
                    self.service
                        .remember_session_invitation_caller(&record.record_id, caller)
                        .await?;
                }
                self.service
                    .queue_session_invitations(record.record_id.clone());
            }
            self.service.queue_session_initialization(
                initialization_credential,
                initialization,
                record.state,
            );
            return Ok(vec![reply(
                request,
                "operation.updated",
                json!({"operation_id":record.operation_id,"state":record.state,"revision":record.revision,"result":record.result}),
            )?]);
        }
        if let Some(command) = crate::canonical_transitions::command(request)? {
            let record = self
                .service
                .canonical_transition_held(credential, command, authority.map(|a| a.retained()))
                .await?;
            let mut payload = json!({"operation_id":record.operation_id,"record_id":record.record_id,"command_type":record.command_type,"expected_revision":record.expected_revision,"state":record.state,"revision":record.revision});
            if let Some(result) = record.result {
                payload["result"] = result;
            }
            if let Some(problem) = record.problem {
                payload["problem"] = problem;
            }
            return Ok(vec![reply(request, "operation.updated", payload)?]);
        }
        if let Some(change) = crate::canonical_membership::request(request)? {
            let record = self
                .service
                .canonical_membership_held(credential, change, authority.map(|a| a.retained()))
                .await?;
            let mut frames = vec![reply(
                request,
                "operation.updated",
                serde_json::json!({"operation_id":record.operation_id,"state":record.state,"revision":record.revision,"result":record.result}),
            )?];
            let Outcome::Membership(connect_protocol::membership::MembershipOutcome::Participant {
                participant,
                ..
            }) = serde_json::from_value(record.result.ok_or_else(Error::storage)?)
                .map_err(|_| Error::storage())?
            else {
                return Err(Error::storage());
            };
            let kind = match request.payload() {
                Payload::ConversationParticipantAdd(_) => "added",
                Payload::ConversationParticipantUpdate(_) => "updated",
                _ => "removed",
            };
            frames.push(reply(request,"conversation.participant.changed",json!({"participant_id":participant.participant_id,"kind":kind,"membership_epoch":participant.membership_epoch}))?);
            return Ok(frames);
        }
        if matches!(request.payload(), Payload::ConversationSubscribe(_)) {
            return if let Some(authorized) = authorized {
                self.subscribe(
                    authorized,
                    credential,
                    authority.ok_or_else(Error::forbidden)?,
                )
                .await
            } else {
                self.service
                    .canonical_events_http(request, credential)
                    .await
            };
        }
        if let Some(operation) = crate::canonical_conversations::operation(request)? {
            let outcome = self
                .service
                .canonical_conversation_held(credential, operation, authority.map(|a| a.retained()))
                .await?;
            return conversation_replies(request, outcome);
        }
        // The remaining installed slice is conversation data only. Extra session or
        // connection assertions and revision preconditions need their own
        // transactional validation; never silently discard them.
        if request.scope(Scope::Session).is_some()
            || request.scope(Scope::Connection).is_some()
            || request.expected_revision().is_some()
        {
            return Err(Error::new(
                "unsupported-command",
                "Resource assertion is not installed",
            ));
        }
        match request.payload() {
            Payload::MessageSend(input) => {
                // Accepted durable messages always use the reliable ordered
                // path. Do not silently change an explicitly requested mode.
                if input.reliability.as_ref().is_some_and(|r| {
                    serde_json::to_value(r).ok() != Some(json!({"mode":"reliable_ordered"}))
                }) {
                    return Err(Error::new(
                        "unsupported-command",
                        "Unsupported message reliability",
                    ));
                }
                let outcome = self
                    .service
                    .canonical_message_send_held(
                        credential,
                        required(request, Scope::Operation)?,
                        required(request, Scope::Conversation)?,
                        input.clone(),
                        authority.map(|a| a.retained()),
                    )
                    .await?;
                Ok(vec![reply(request, "ack", json!({"details":outcome}))?])
            }
            Payload::MessageHistory(input) => {
                if request.scope(Scope::Operation).is_some() {
                    return Err(Error::new(
                        "invalid-request",
                        "History does not select a logical mutation",
                    ));
                }
                let mut input = input.clone();
                let limit = input.limit.unwrap_or(100);
                if limit == 0 || limit >= MAX_HANDLER_REPLIES as u64 {
                    return Err(Error::new(
                        "invalid-request",
                        "History page exceeds reply capacity",
                    ));
                }
                input.limit = Some(limit);
                let outcome = self
                    .service
                    .canonical_message_history_held(
                        credential,
                        required(request, Scope::Conversation)?,
                        input,
                        authority.map(|a| a.retained()),
                    )
                    .await?;
                let Outcome::History { messages, next } = outcome else {
                    return Err(Error::storage());
                };
                let mut frames = Vec::with_capacity(messages.len() + 1);
                for message in messages {
                    frames.push(reply(request, "message.send", history_payload(message)?)?);
                }
                let mut ack = json!({});
                if let Some(cursor) = next {
                    ack["next_cursor"] = json!(cursor);
                }
                frames.push(reply(request, "ack", ack)?);
                Ok(frames)
            }
            Payload::ConversationGet(input) => {
                crate::canonical_subscriptions::assert_read_scopes(request)?;
                let query = crate::canonical_subscriptions::snapshot_query(input)?;
                let outcome = self
                    .service
                    .canonical_snapshot_held(
                        credential,
                        required(request, Scope::Conversation)?,
                        query,
                        authority.map(|a| a.retained()),
                    )
                    .await?;
                let Outcome::ConversationSnapshot(page) = outcome else {
                    return Err(Error::storage());
                };
                let mut payload = json!({"revision":page.revision,"cursor":page.cursor,"items":page.items,"has_more":page.has_more});
                if let Some(next) = page.next {
                    payload["next_page"] = json!(next);
                }
                Ok(vec![reply(request, "conversation.snapshot", payload)?])
            }
            _ => Err(Error::new(
                "unsupported-command",
                "Command is not installed",
            )),
        }
    }
}

#[async_trait]
impl AuthenticatedHandler for CanonicalHandler {
    async fn next_delivery(
        &self,
    ) -> Result<rvoip_uctp::v2::auth_dispatch::HandlerDelivery, DispatchError> {
        let subscription = async {
            match &self.subscriptions {
                Some(subscriptions) => subscriptions.next(&self.service).await,
                None => std::future::pending().await,
            }
        };
        let lifecycle = async {
            match &self.physical {
                Some(physical) => physical.lifecycle.next_delivery().await,
                None => std::future::pending().await,
            }
        };
        let capabilities = async {
            match (&self.physical, self.service.canonical_runtime.get()) {
                (Some(physical), Some(runtime)) => physical.capabilities.next(runtime).await,
                _ => std::future::pending().await,
            }
        };
        let dtmf = async {
            match &self.physical {
                Some(physical) => physical.dtmf.next_delivery().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! { result=subscription=>result, result=lifecycle=>result, result=capabilities=>result, result=dtmf=>result }
    }

    async fn handle(&self, authorized: AuthorizedEnvelope) -> Result<Envelope, DispatchError> {
        self.handle_replies(authorized).await?.into_one()
    }
    async fn handle_replies(
        &self,
        authorized: AuthorizedEnvelope,
    ) -> Result<HandlerReplies, DispatchError> {
        if matches!(
            authorized.envelope().payload(),
            Payload::RecordingVconFetch(_)
        ) {
            let custody = authorized
                .authority::<Arc<rvoip_uctp::v2::recording::RecordFetchCustody>>()
                .ok_or(DispatchError::Authority)?;
            let base = custody
                .original_authority()
                .authority::<crate::StoreInvocationAuthority>()
                .ok_or(DispatchError::Authority)?;
            if let Err(error) = self.authorize_with(&authorized, base).await {
                return HandlerReplies::new(vec![problem(authorized.envelope(), error)?]);
            }
            let complete = reply(
                authorized.envelope(),
                "ack",
                json!({"details":{"reply_complete":true}}),
            )
            .map_err(|_| DispatchError::Closed)?;
            let result = rvoip_uctp::v2::recording::RecordFetchHandler
                .handle(authorized)
                .await?;
            return if result.kind() == "error" {
                HandlerReplies::new(vec![result])
            } else {
                HandlerReplies::new(vec![result, complete])
            };
        }
        let is_data = matches!(authorized.envelope().payload(), Payload::MessageSend(_))
            && (authorized.envelope().scope(Scope::Session).is_some()
                || authorized.envelope().scope(Scope::Connection).is_some());
        if matches!(
            authorized.envelope().payload(),
            Payload::ConnectionOffer(_)
                | Payload::StreamSubscribe(_)
                | Payload::StreamUnsubscribe(_)
                | Payload::ConnectionEnd(_)
                | Payload::ConnectionUpdate(_)
                | Payload::CapabilityAdvertise(_)
                | Payload::DtmfSend(_)
        ) || is_data
        {
            let is_offer = matches!(authorized.envelope().payload(), Payload::ConnectionOffer(_));
            let a = Arc::new(authorized);
            let result = async {
                if is_offer {
                    crate::canonical_native::intent(a.envelope())?;
                } else {
                    crate::canonical_native_streams::intent(a.envelope())?;
                }

                let credential = self.authorize(&a).await?;
                let physical = self.physical.clone().ok_or_else(|| {
                    Error::new(
                        "unsupported-media-profile",
                        "Native connection control requires its physical peer",
                    )
                })?;
                let runtime = self
                    .service
                    .canonical_runtime
                    .get()
                    .ok_or_else(Error::forbidden)?;
                if is_offer {
                    runtime.native_offer(a.clone(), physical, credential).await
                } else if is_data {
                    runtime.native_data(a.clone(), physical, credential).await
                } else if a.envelope().kind() == "dtmf.send" {
                    runtime.native_dtmf(a.clone(), physical, credential).await
                } else {
                    runtime.native_stream(a.clone(), physical, credential).await
                }
            }
            .await;
            return match result {
                Ok(value) if value.answered => a.finish_answered_offer(),
                Ok(value) => {
                    let refusal = if a.envelope().kind() == "stream.subscribe"
                        && value.record.state
                            == connect_protocol::transitions::OperationState::Failed
                    {
                        value
                            .record
                            .problem
                            .as_ref()
                            .and_then(|p| p["code"].as_str())
                            .filter(|code| {
                                matches!(*code, "not-found" | "incompatible-capabilities")
                            })
                            .map(|code| Error::new(code, "Subscription selection refused"))
                    } else {
                        None
                    };
                    let mut payload = json!({"operation_id":value.record.operation_id,"record_id":value.record.record_id,"state":value.record.state,"revision":value.record.revision});
                    if let Some(result) = value.record.result {
                        payload["result"] = result;
                    }
                    if let Some(problem) = value.record.problem {
                        payload["problem"] = problem;
                    }
                    HandlerReplies::new(vec![
                        reply(a.envelope(), "operation.updated", payload)
                            .map_err(|_| DispatchError::Closed)?,
                        if let Some(error) = refusal {
                            problem(a.envelope(), error)?
                        } else {
                            reply(
                                a.envelope(),
                                "ack",
                                json!({"details":{"reply_complete":true}}),
                            )
                            .map_err(|_| DispatchError::Closed)?
                        },
                    ])
                }
                Err(error) => HandlerReplies::new(vec![problem(a.envelope(), error)?]),
            };
        }
        let mut frames = match self.execute(&authorized).await {
            Ok(frames) => frames,
            Err(error) => vec![problem(authorized.envelope(), error)?],
        };
        // A successful application batch always has an explicit terminator.
        // This is local submission/response completion, never media readiness
        // or completion of an asynchronous logical operation.
        if frames
            .last()
            .is_some_and(|frame| !matches!(frame.kind(), "ack" | "error"))
        {
            frames.push(
                reply(
                    authorized.envelope(),
                    "ack",
                    json!({"details":{"reply_complete":true}}),
                )
                .map_err(|_| DispatchError::Closed)?,
            );
        }
        HandlerReplies::new(frames)
    }
}

fn conversation_replies(
    request: &Envelope,
    outcome: Outcome,
) -> connect_protocol::Result<Vec<Envelope>> {
    use connect_protocol::Conversation;
    fn fact(
        request: &Envelope,
        kind: &str,
        value: &Conversation,
        payload: Value,
    ) -> connect_protocol::Result<Envelope> {
        reply_scoped(request, kind, payload, Some(&value.id))
    }
    fn closed(request: &Envelope, value: &Conversation) -> connect_protocol::Result<Envelope> {
        fact(
            request,
            "conversation.closed",
            value,
            json!({"reason_code":value.reason_code.ok_or_else(Error::storage)?,"reason":value.reason.as_ref().ok_or_else(Error::storage)?,"closed_at":chrono::DateTime::from_timestamp(value.closed_at.ok_or_else(Error::storage)?,0).ok_or_else(Error::storage)?}),
        )
    }
    fn opened(request: &Envelope, value: &Conversation) -> connect_protocol::Result<Envelope> {
        let mut payload = json!({"tenant_id":value.tenant_id,"policy":value.policy,"participants":value.participants.iter().map(|p|{let mut v=json!({"participant_id":p.participant_id,"identity_id":p.identity_id,"kind":p.kind,"role":p.role});if let Some(d)=&p.display_name{v["display_name"]=json!(d);}v}).collect::<Vec<_>>(),"opened_at":chrono::DateTime::from_timestamp(value.opened_at,0).ok_or_else(Error::storage)?});
        if let Some(idle) = value.idle_close_secs {
            payload["idle_close_secs"] = json!(idle);
        }
        if let Some(metadata) = &value.metadata {
            payload["metadata"] = metadata.clone();
        }
        fact(request, "conversation.opened", value, payload)
    }
    let mut frames = vec![];
    match outcome {
        Outcome::Conversation(value) => {
            if matches!(request.payload(), Payload::ConversationClose(_)) {
                frames.push(closed(request, &value)?);
            } else {
                frames.push(opened(request, &value)?);
                if value.closed {
                    frames.push(closed(request, &value)?);
                }
            }
            frames.push(reply(
                request,
                "ack",
                json!({"details":{"reply_complete":true,"conversation":value}}),
            )?);
        }
        Outcome::Conversations(page) => {
            for value in &page.conversations {
                frames.push(opened(request, &value)?);
                if value.closed {
                    frames.push(closed(request, &value)?);
                }
            }
            let mut payload = json!({});
            if let Some(cursor) = &page.next_cursor {
                payload["next_cursor"] = json!(cursor);
            }
            payload["details"] = json!({"page":page});
            frames.push(reply(request, "ack", payload)?);
        }
        _ => return Err(Error::storage()),
    }
    Ok(frames)
}
fn required(request: &Envelope, scope: Scope) -> connect_protocol::Result<String> {
    request
        .scope(scope)
        .map(str::to_owned)
        .ok_or_else(|| Error::new("invalid-request", "Missing resource scope"))
}
fn history_payload(message: Message) -> connect_protocol::Result<Value> {
    let from = message.sender_participant_id.ok_or_else(Error::storage)?;
    // Use the accepted audience, including its historical membership, rather
    // than expanding `all` against today's membership at the receiving peer.
    let to: Vec<_> = message
        .audience
        .into_iter()
        .map(|recipient| recipient.participant_id)
        .collect();
    if to.is_empty() {
        return Err(Error::storage());
    }
    let mut value = json!({
        "msg_id":message.id,"from":from,"to":to,
        "content_type":message.content_type,"label":message.label,
        "body":message.body,"body_encoding":message.body_encoding,
        "reliability":{"mode":"reliable_ordered"},"attachments":message.attachments,
    });
    if let Some(id) = message.in_reply_to_msg {
        value["in_reply_to_msg"] = json!(id);
    }
    Ok(value)
}
pub(crate) fn reply(
    request: &Envelope,
    kind: &str,
    payload: Value,
) -> connect_protocol::Result<Envelope> {
    reply_scoped(request, kind, payload, None)
}
fn reply_scoped(
    request: &Envelope,
    kind: &str,
    payload: Value,
    cid: Option<&str>,
) -> connect_protocol::Result<Envelope> {
    let mut value = json!({"v":2,"type":kind,"id":format!("reply_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"in_reply_to":request.id(),"payload":payload});
    for (scope, field) in [
        (Scope::Conversation, "cid"),
        (Scope::Session, "sid"),
        (Scope::Connection, "connid"),
        (Scope::Operation, "operation_id"),
    ] {
        if let Some(id) = request.scope(scope) {
            value[field] = json!(id);
        }
    }
    if let Some(cid) = cid {
        value["cid"] = json!(cid);
    }
    Envelope::from_value(value).map_err(|_| Error::storage())
}
pub(crate) fn problem(request: &Envelope, error: Error) -> Result<Envelope, DispatchError> {
    let problem = connect_protocol::canonical_http::Problem::from_error(&error);
    reply(
        request,
        "error",
        serde_json::to_value(problem).map_err(|_| DispatchError::Closed)?,
    )
    .map_err(|_| DispatchError::Closed)
}
