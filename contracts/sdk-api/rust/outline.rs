//! Compile-only target outline, with no mock SDK implementation or media claims.
#![allow(dead_code)]
mod contract;
use contract::*;
use std::sync::Arc;
pub struct SavedJourney {
    pub send: OperationId,
    pub message: MessageId,
    pub consult: OperationId,
    pub handoff: OperationId,
    pub movement: OperationId,
    pub cancel: OperationId,
    pub accept: OperationId,
    pub reject: OperationId,
    pub return_consult: OperationId,
    pub cursor: Option<Cursor>,
}
pub struct Context<A: Api> {
    pub conversation: A::Conversation,
    pub session: A::Session,
    pub control: A::Connection,
    pub retained: A::Connection,
    pub owner: A::Participant,
    pub assistant: A::Participant,
    pub phone: EndpointId,
}
fn mutation(id: &OperationId, request: &RequestOptions) -> MutationOptions {
    MutationOptions {
        operation_id: id.clone(),
        request: request.clone(),
        extensions: None,
    }
}
pub async fn journey<A: Api>(
    client: &impl ParticipantClient<A>,
    context: &Context<A>,
    saved: &SavedJourney,
    wait: RequestOptions,
) -> Result<OperationReference<MoveResult<A::Connection>>> {
    let _ = client;
    // These operation/message IDs were persisted by the application before IO.
    let send = context
        .conversation
        .send(
            saved.message.clone(),
            NonEmpty {
                first: context.owner.id().clone(),
                rest: vec![],
            },
            "Can we talk?".into(),
            mutation(&saved.send, &wait),
        )
        .await?;
    let acceptance = send.wait(wait.clone()).await?;
    assert_eq!(acceptance.value.message_id, saved.message);
    let _receipts = context
        .conversation
        .receipts(saved.message.clone(), wait.clone())
        .await?;
    let before = context.session.refresh(wait.clone()).await?;
    let consultation = context
        .session
        .consult(
            ConsultInput {
                via: context.control.id().clone(),
                retain: context.retained.id().clone(),
                with_participants: NonEmpty {
                    first: context.owner.id().clone(),
                    rest: vec![context.assistant.id().clone()],
                },
                hold: Hold::Silence,
                expected_topology_revision: before.topology_revision,
            },
            mutation(&saved.consult, &wait),
        )
        .await?;
    consultation.wait_until_ready(wait.clone()).await?; // Not decoded audio.
    let before = context.session.refresh(wait.clone()).await?;
    let handoff = context
        .session
        .handoff(
            HandoffInput::<A> {
                via: context.control.id().clone(),
                from_participant: context.assistant.id().clone(),
                to: context.owner.id().clone(),
                retain: context.retained.id().clone(),
                outgoing: Outgoing::Leave,
                role: HandoffRole::Session,
                expected_topology_revision: before.topology_revision,
                candidate: None,
                consultation: Some(consultation.reference()),
            },
            mutation(&saved.handoff, &wait),
        )
        .await?;
    handoff.wait(wait.clone()).await?;
    let before = context.session.refresh(wait.clone()).await?;
    let movement = context
        .control
        .replace_device(
            MoveInput {
                retain: context.retained.id().clone(),
                target: MoveTarget::Endpoint(context.phone.clone()),
                expected_topology_revision: before.topology_revision,
            },
            mutation(&saved.movement, &wait),
        )
        .await?;
    let reference = movement.reference(); // Persist before waiting or losing transport.
    movement.wait(wait).await?;
    Ok(reference)
}
pub async fn recover<A: Api>(
    client: &impl ParticipantClient<A>,
    reference: OperationReference<MoveResult<A::Connection>>,
    wait: RequestOptions,
) -> Result<()> {
    let original = client.recover(reference); // No submission and no IO.
    let observed = original.status(wait.clone()).await?;
    if matches!(
        observed.state,
        OperationState::Unknown | OperationState::Interrupted
    ) {
        return Ok(());
    }
    if observed.state == OperationState::Completed {
        original.wait(wait).await?;
    }
    // No new replace/send/invite call, logical ID or revision in recovery.
    Ok(())
}
pub async fn cancel_wait_then_request_cancellation<A: Api>(
    client: &impl ParticipantClient<A>,
    reference: OperationReference<MoveResult<A::Connection>>,
    saved: &SavedJourney,
    wait: RequestOptions,
    local: Arc<dyn Cancellation>,
) -> Result<()> {
    let original = client.recover(reference);
    local.cancel();
    match original
        .wait(RequestOptions {
            timeout: wait.timeout,
            signal: Some(local),
        })
        .await
    {
        Err(error) if error.kind == ErrorKind::Cancelled => (),
        Err(error) => return Err(error),
        Ok(_) => (),
    }
    original.status(wait.clone()).await?; // No durable cancellation yet.
    let decision = original.cancel(mutation(&saved.cancel, &wait)).await?;
    decision.wait(wait).await?; // Not fabricated rollback of the original effect.
    Ok(())
}
pub async fn invitations<A: Api>(
    client: &impl ParticipantClient<A>,
    saved: &SavedJourney,
    wait: RequestOptions,
    accept: bool,
) -> Result<()> {
    let mut stream = client.incoming_invitations(wait.clone());
    let processing = async {
        while let Some(invitation) = stream.next().await? {
            let current = invitation.session.refresh(wait.clone()).await?;
            if matches!(
                current.state.as_str(),
                "ended" | "rejected" | "cancelled" | "failed"
            ) {
                continue;
            }
            if accept {
                let capabilities = Json::Object(
                    [(
                        "audio".into(),
                        Json::Array(vec![Json::String("opus".into())]),
                    )]
                    .into(),
                );
                let operation = invitation
                    .session
                    .accept(
                        capabilities,
                        current.topology_revision,
                        mutation(&saved.accept, &wait),
                    )
                    .await?;
                operation.wait(wait.clone()).await?;
            } else {
                let operation = invitation
                    .session
                    .reject(
                        486,
                        "Unavailable".into(),
                        current.topology_revision,
                        mutation(&saved.reject, &wait),
                    )
                    .await?;
                operation.wait(wait.clone()).await?;
            }
            break;
        }
        Ok(())
    }
    .await;
    let cleanup = stream.close().await;
    processing?;
    cleanup?;
    Ok(())
}
pub async fn events<A: Api>(
    context: &Context<A>,
    saved: &SavedJourney,
    wait: RequestOptions,
) -> Result<Option<Cursor>> {
    let mut stream = context
        .conversation
        .events(saved.cursor.clone(), wait.clone());
    // close() also runs when processing fails; production Drop still owns a
    // nonblocking retirement request, never an indefinite network wait.
    let processing = async {
        let mut cursor = saved.cursor.clone();
        if let Some(event) = stream.next().await? {
            context.conversation.refresh(wait).await?;
            cursor = Some(event.cursor);
        }
        Ok(cursor)
    }
    .await;
    let cleanup = stream.close().await;
    let cursor = processing?;
    cleanup?;
    Ok(cursor) // Persist only after processing, never as a permission grant.
}
pub async fn capability_refusal<A: Api>(
    client: &impl ParticipantClient<A>,
    wait: RequestOptions,
) -> Result<()> {
    match client
        .capabilities()
        .require("native-programmatic-media", wait)
        .await
    {
        Err(error) if error.kind == ErrorKind::Unsupported => Ok(()),
        other => other,
    }
}
pub async fn return_consultation<A: Api>(
    context: &Context<A>,
    saved: &SavedJourney,
    wait: RequestOptions,
) -> Result<()> {
    let current = context.session.refresh(wait.clone()).await?;
    let consultation = context
        .session
        .consult(
            ConsultInput {
                via: context.control.id().clone(),
                retain: context.retained.id().clone(),
                with_participants: NonEmpty {
                    first: context.owner.id().clone(),
                    rest: vec![],
                },
                hold: Hold::Silence,
                expected_topology_revision: current.topology_revision,
            },
            mutation(&saved.consult, &wait),
        )
        .await?;
    consultation.wait_until_ready(wait.clone()).await?;
    let current = context.session.refresh(wait.clone()).await?;
    let returning = consultation
        .return_to_retained(
            current.topology_revision,
            mutation(&saved.return_consult, &wait),
        )
        .await?;
    returning.wait(wait).await?;
    Ok(())
}
pub async fn managed_lifetime<A: Api>(
    client: &impl ParticipantClient<A>,
    context: &Context<A>,
    saved: &SavedJourney,
    wait: RequestOptions,
) -> Result<()> {
    let result = journey(client, context, saved, wait.clone()).await;
    let cleanup = client.close(wait).await;
    result?;
    cleanup
}

pub async fn recover_saved<A: Api>(
    client: &impl ParticipantClient<A>,
    saved: Json,
    wait: RequestOptions,
) -> Result<()> {
    let original = client.restore(saved, client.operation_kinds().connection_replace)?;
    let observed = original.status(wait.clone()).await?;
    if matches!(
        observed.state,
        OperationState::Unknown | OperationState::Interrupted
    ) {
        return Ok(());
    }
    if observed.state == OperationState::Completed {
        original.wait(wait).await?;
    }
    Ok(())
}

// Native callers may move the pending public workflow onto a task executor.
pub fn assert_send_journey<A: Api>(
    client: &impl ParticipantClient<A>,
    context: &Context<A>,
    saved: &SavedJourney,
    wait: RequestOptions,
) {
    fn require_send(_: impl std::future::Future + Send) {}
    require_send(journey(client, context, saved, wait));
}

// The native receipt is historical capture, not a set of newly owned Streams.
pub async fn native_receipts<A: Api>(
    connection: &A::Connection,
    from: ParticipantId,
    revision: StreamRevision,
    mutations: [MutationOptions; 3],
    wait: RequestOptions,
) -> Result<Option<usize>> {
    let [update, select, end] = mutations;
    let changed = connection
        .update(
            ConnectionUpdate::Quality { interval_ms: 250 },
            revision,
            update,
        )
        .await?
        .wait(wait.clone())
        .await?
        .value;
    let selected = connection
        .subscribe(
            NonEmpty {
                first: StreamSelector::Participant {
                    participant_id: from,
                    kinds: None,
                },
                rest: vec![],
            },
            changed.stream_revision,
            select,
        )
        .await?
        .wait(wait.clone())
        .await?
        .value;
    let observed = selected.bindings.as_ref().map(Vec::len);
    connection
        .end(
            1000,
            "Application completed".into(),
            Some(selected.stream_revision),
            end,
        )
        .await?
        .wait(wait)
        .await?;
    Ok(observed)
}
