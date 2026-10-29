"""Compiled against actual public Python runtime types; never runs a server."""
from typing import assert_type
from connect_sdk import (
    Client, Conversation, ConversationId, ConversationRevision, DecisionReceipt,
    IdentityId, MessageAcceptance, MessageId, MutationOptions, NonEmpty, Operation,
    OperationId, Participant, ParticipantId, RequestOptions,
    Connection, ConnectionId, ConnectionUpdateReceipt, OfferedStream,
    Quality, Session, SessionDecision, SessionId, StreamId, StreamRevision,
    StreamSelectionReceipt, StreamTarget, TopologyRevision,
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
