"""Twelve unsuppressed public-type constraints; compilation must fail exactly twelve."""
from connect_sdk import (
    Client, Conversation, ConversationId, MessageId, MutationOptions, Operation,
    ParticipantId, RequestOptions, TopologyRevision,
    ConnectionId, Quality, Session, SessionId, SessionRevision, StreamId, StreamResourceRevision, StreamRevision,
)


async def invalid(client: Client, options: RequestOptions, mutation: MutationOptions) -> None:
    client.conversations.by_id(ParticipantId('wrong-id-family'))
    room = client.conversations.by_id(ConversationId('room'))
    await room.participants.by_id(ParticipantId('participant')).remove(TopologyRevision(7), mutation)
    await room.send(MessageId('message'), [], 'text', mutation)
    message = client.operations.restore({}, client.operation_kinds.message_send)
    wrong: Operation[Conversation] = client.operations.recover(message.reference)
    client.provision({})
    client.operations.recover({'operation_id':'original'})
    session = room.sessions.by_id(SessionId('session'))
    await session.end(1000, 'done', SessionRevision(2), mutation)
    connection = session.connections.by_id(ConnectionId('connection'))
    await connection.update(Quality(250), TopologyRevision(2), mutation)
    await connection.update(Quality(250), StreamResourceRevision(2), mutation)
    await connection.streams.subscribe([], StreamRevision(1), mutation)
    wrong_connection: Operation[Session] = client.operations.restore({}, client.operation_kinds.connection_offer)
    connection.streams.by_id(StreamId('audio')).send_audio(b'no custody')
