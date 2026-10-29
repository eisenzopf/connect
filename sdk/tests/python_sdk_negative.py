"""Thirty unsuppressed public-type constraints; compilation must fail exactly thirty."""
from connect_sdk import (
    Client, Conversation, ConversationId, MessageId, MutationOptions, Operation,
    ParticipantId, RequestOptions, TopologyRevision,
    ConnectionId, Quality, Session, SessionId, SessionRevision, StreamId, StreamResourceRevision, StreamRevision,
    ConversationRevision, ConversationRole, HandoffInput, IdentityId, MoveInput, MoveResult, SessionRole,
    AdministrativeClient,
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
    await message.return_to_retained(TopologyRevision(3),mutation)
    MoveInput(ConnectionId('retained'),{'endpoint':'endpoint','candidate':'candidate'},TopologyRevision(3))
    HandoffInput(ConnectionId('controller'),IdentityId('wrong-identity-kind'),ParticipantId('to'),ConnectionId('retained'),'leave',SessionRole(),TopologyRevision(3))
    ConversationRole(TopologyRevision(3))
    wrong_handoff: Operation[MoveResult] = client.operations.restore({},client.operation_kinds.session_handoff)
    consult=client.operations.restore({},client.operation_kinds.session_consult)
    await consult.return_to_retained(SessionRevision(3),mutation)
    stream=room.events(MessageId('wrong-cursor-family'),options)
    event=await anext(stream)
    event.cursor=event.cursor
    event.audience.append(event.audience[0])
    hint=await anext(client.incoming_invitations(options))
    hint.native_owner
    await client.raw.request({},options)
    checked=client.raw.validate({})
    checked.value=None
    checked.native_owner


async def invalid_administration(admin: AdministrativeClient, client: Client, mutation: MutationOptions) -> None:
    admin.conversations
    client.validate_provisioning({})
    await admin.provision({},mutation)
    command=admin.validate_provisioning({})
    command.value=None
    operation=await admin.provision(command,mutation)
    wrong: Operation[Conversation]=admin.operations.recover(operation.reference)
