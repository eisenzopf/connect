"""Compile-only shared contract outline; this does not execute an SDK."""
from dataclasses import dataclass
from contract import (
    Json,
    ParticipantClient, Conversation, Session, Connection, Participant, EndpointId,
    OperationId, MessageId, Cursor, OperationReference, MoveResult, RequestOptions,
    MutationOptions, NonEmpty, ConsultInput, HandoffInput, SessionRole, MoveInput,
    EndpointTarget, ConnectError, Cancellation,
)

@dataclass(frozen=True)
class SavedJourney:
    # Already committed by the application before the first SDK call.
    send: OperationId
    message: MessageId
    consult: OperationId
    handoff: OperationId
    move: OperationId
    cancel: OperationId
    accept: OperationId
    reject: OperationId
    return_consult: OperationId
    cursor: Cursor | None = None

@dataclass(frozen=True)
class Context:
    conversation: Conversation
    session: Session
    control: Connection
    retained: Connection
    owner: Participant
    assistant: Participant
    phone: EndpointId

async def journey(client: ParticipantClient, context: Context, saved: SavedJourney, wait: RequestOptions) -> OperationReference[MoveResult]:
    conversation, session = context.conversation, context.session
    send = await conversation.send(saved.message, NonEmpty(context.owner.id), 'Can we talk?', MutationOptions(saved.send, wait))
    acceptance = await send.wait(wait)
    if acceptance.value.message_id != saved.message:
        raise ValueError('Wrong original message')
    await conversation.receipts(saved.message, wait)  # Separate attributed evidence.
    before_consult = await session.refresh(wait)
    consultation = await session.consult(ConsultInput(context.control.id, context.retained.id, NonEmpty(context.owner.id, [context.assistant.id]), 'silence', before_consult.topology_revision), MutationOptions(saved.consult, wait))
    await consultation.wait_until_ready(wait)  # State observation, not decoded audio.
    before_handoff = await session.refresh(wait)
    handoff = await session.handoff(HandoffInput(context.control.id, context.assistant.id, context.owner.id, context.retained.id, 'leave', SessionRole(), before_handoff.topology_revision, consultation=consultation), MutationOptions(saved.handoff, wait))
    await handoff.wait(wait)
    before_move = await session.refresh(wait)
    movement = await context.control.replace_device(MoveInput(context.retained.id, EndpointTarget(context.phone), before_move.topology_revision), MutationOptions(saved.move, wait))
    reference = movement.reference  # Persist before waiting; do not replace the ID.
    await movement.wait(wait)
    return reference

async def recover(client: ParticipantClient, reference: OperationReference[MoveResult], wait: RequestOptions) -> None:
    original = client.operations.recover(reference)  # Local handle; no submission.
    observed = await original.status(wait)          # Original-ID query first.
    if observed.state in ('unknown', 'interrupted'):
        return
    if observed.state == 'completed':
        await original.wait(wait)
    # No new replace/send/invite or changed revision in recovery.

async def cancel_wait_then_request_cancellation(client: ParticipantClient, reference: OperationReference[MoveResult], saved: SavedJourney, wait: RequestOptions, local: Cancellation) -> None:
    original = client.operations.recover(reference)
    local.cancel()
    try:
        await original.wait(RequestOptions(wait.timeout_ms, local))
    except ConnectError as error:
        if error.kind != 'cancelled':
            raise
    await original.status(wait)  # Local cancellation made no durable decision.
    decision = await original.cancel(MutationOptions(saved.cancel, wait))
    await decision.wait(wait)   # Decision outcome is not fabricated rollback.

async def invitations(client: ParticipantClient, saved: SavedJourney, wait: RequestOptions, accept: bool) -> None:
    stream = client.incoming_invitations(wait)
    try:
        async for invitation in stream:
            current = await invitation.session.refresh(wait)
            if current.state != 'invited':
                continue
            if accept:
                await (await invitation.session.accept({'audio': ['opus']}, current.revision, MutationOptions(saved.accept, wait))).wait(wait)
            else:
                await (await invitation.session.reject(486, 'Unavailable', current.revision, MutationOptions(saved.reject, wait))).wait(wait)
            break
    finally:
        await stream.close()

async def events(context: Context, saved: SavedJourney, wait: RequestOptions) -> Cursor | None:
    stream = context.conversation.events(saved.cursor, wait)
    cursor = saved.cursor
    try:
        async for event in stream:
            await context.conversation.refresh(wait)
            cursor = event.cursor
            break
    finally:
        await stream.close()
    return cursor  # Persist after processing, never as a permission grant.

async def capability_refusal(client: ParticipantClient, wait: RequestOptions) -> None:
    try:
        await client.capabilities.require('native-programmatic-media', wait)
    except ConnectError as error:
        if error.kind != 'unsupported':
            raise

async def return_consultation(context: Context, saved: SavedJourney, wait: RequestOptions) -> None:
    current = await context.session.refresh(wait)
    consultation = await context.session.consult(ConsultInput(context.control.id, context.retained.id, NonEmpty(context.owner.id), 'silence', current.topology_revision), MutationOptions(saved.consult, wait))
    await consultation.wait_until_ready(wait)
    before_return = await context.session.refresh(wait)
    await (await consultation.return_to_retained(before_return.topology_revision, MutationOptions(saved.return_consult, wait))).wait(wait)

async def managed_lifetime(client: ParticipantClient, context: Context, saved: SavedJourney, wait: RequestOptions) -> None:
    async with client as owned:
        await journey(owned, context, saved, wait)

async def recover_saved(client: ParticipantClient, saved: Json, wait: RequestOptions) -> None:
    original = client.operations.restore(saved, client.operation_kinds.connection_replace)
    observed = await original.status(wait)
    if observed.state in ('unknown', 'interrupted'):
        return
    if observed.state == 'completed':
        await original.wait(wait)
