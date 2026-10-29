"""Compiled against actual public Python runtime types; never runs a server."""
from typing import assert_type
from collections.abc import Sequence
from connect_sdk import (
    Client, Conversation, ConversationId, ConversationRevision, DecisionReceipt,
    IdentityId, MessageAcceptance, MessageId, MutationOptions, NonEmpty, Operation,
    OperationId, Participant, ParticipantId, RequestOptions,
    Connection, ConnectionId, ConnectionUpdateReceipt, OfferedStream,
    Quality, Session, SessionDecision, SessionId, StreamId, StreamRevision,
    StreamSelectionReceipt, StreamTarget, TopologyRevision,
    CandidateTarget, ConsultInput, ConsultationOperation, ConsultationResult, EndpointId,
    EndpointTarget, HandoffInput, HandoffResult, MoveInput, MoveResult, SessionRole,
    Cursor, EventStream, InvitationStream, JournalEvent, IncomingInvitation,
    RawCanonical, ValidatedEnvelope,
)


async def control(client: Client, options: RequestOptions) -> MessageAcceptance:
    create = await client.conversations.create([IdentityId('peer')], 'persistent', MutationOptions(OperationId('create'), options))
    assert_type(create, Operation[Conversation])
    room = (await create.wait(options)).value
    assert_type(room, Conversation)
    participant = room.participants.by_id(ParticipantId('participant'))
    assert_type(await participant.remove(ConversationRevision(7), MutationOptions(OperationId('remove'), options)), Operation[Participant])
    sent = await room.send(MessageId('message'), NonEmpty(ParticipantId('participant')), 'text', MutationOptions(OperationId('original'), options))
    assert_type(sent, Operation[MessageAcceptance])
    saved = sent.reference_json()
    recovered = client.operations.restore(saved, client.operation_kinds.message_send)
    assert_type(client.operations.recover(recovered.reference), Operation[MessageAcceptance])
    assert_type(client.conversations.by_id(ConversationId('room')), Conversation)
    assert_type(await sent.cancel(MutationOptions(OperationId('decision'), options)), Operation[DecisionReceipt])
    await recovered.status(options)
    return (await recovered.wait(options)).value


async def session_control(client: Client, options: RequestOptions, mutation: MutationOptions) -> None:
    room = client.conversations.by_id(ConversationId('room'))
    invited = await room.sessions.invite(SessionId('session'), [], 'voice', 'intent', {'audio':['g.711-mu']}, mutation)
    assert_type(invited, Operation[Session])
    session = (await invited.wait(options)).value
    assert_type(await session.reject(486, 'unavailable', TopologyRevision(2), mutation), Operation[SessionDecision])
    offered = await session.connections.offer(ConnectionId('connection'), NonEmpty(OfferedStream(StreamId('audio'), 'audio', 'sendrecv', NonEmpty('g.711-mu'))), mutation)
    assert_type(offered, Operation[Connection])
    connection = (await offered.wait(options)).value
    assert_type(await connection.update(Quality(250), StreamRevision(1), mutation), Operation[ConnectionUpdateReceipt])
    assert_type(await connection.streams.subscribe(NonEmpty(StreamTarget(StreamId('audio'))), StreamRevision(1), mutation), Operation[StreamSelectionReceipt])


async def transitions(client: Client, mutation: MutationOptions) -> None:
    session=client.conversations.by_id(ConversationId('room')).sessions.by_id(SessionId('session'))
    consulted=await session.consult(ConsultInput(ConnectionId('controller'),ConnectionId('retained'),NonEmpty(ParticipantId('peer')),'silence',TopologyRevision(2)),mutation)
    assert_type(consulted,ConsultationOperation)
    assert_type(consulted,Operation[ConsultationResult])
    assert_type(await consulted.return_to_retained(TopologyRevision(3),mutation),Operation[Session])
    handed=await session.handoff(HandoffInput(ConnectionId('controller'),ParticipantId('from'),ParticipantId('to'),ConnectionId('retained'),'leave',SessionRole(),TopologyRevision(3),consultation=consulted),mutation)
    assert_type(handed,Operation[HandoffResult])
    connection=session.connections.by_id(ConnectionId('controller'))
    moved=await connection.replace_device(MoveInput(ConnectionId('retained'),EndpointTarget(EndpointId('endpoint')),TopologyRevision(4)),mutation)
    assert_type(moved,Operation[MoveResult])
    assert_type(await connection.replace_device(MoveInput(ConnectionId('retained'),CandidateTarget(ConnectionId('candidate')),TopologyRevision(4)),mutation),Operation[MoveResult])
    assert_type(client.operations.restore(moved.reference_json(),client.operation_kinds.connection_replace),Operation[MoveResult])


async def readers(client: Client, options: RequestOptions) -> None:
    stream=client.conversations.by_id(ConversationId('room')).events(Cursor('saved-cursor'),options)
    assert_type(stream,EventStream)
    assert_type(await stream.ready(),Cursor)
    async with stream as owned:
        assert_type(owned,EventStream)
        async for event in owned:
            assert_type(event,JournalEvent)
            assert_type(event.cursor,Cursor)
            break
    hints=client.incoming_invitations(options)
    assert_type(hints,InvitationStream)
    async with hints:
        assert_type(await anext(hints),IncomingInvitation)


async def raw(client: Client, options: RequestOptions) -> None:
    assert_type(client.raw,RawCanonical)
    checked=client.raw.validate({'v':2,'id':'local','ts':'2026-10-28T00:00:00Z','type':'operation.get','payload':{'operation_id':'original'}})
    assert_type(checked,ValidatedEnvelope)
    assert_type(await client.raw.request(checked,options),Sequence[ValidatedEnvelope])
