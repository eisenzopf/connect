"""Forty-three unsuppressed public-type constraints; compilation must fail exactly forty-three."""
from connect_sdk import (
    BinaryData, TextData, OperationId, NonEmpty,
    Client, Conversation, ConversationId, MessageId, MutationOptions, Operation,
    ParticipantId, RequestOptions, TopologyRevision,
    ConnectionId, Quality, Session, SessionId, SessionRevision, StreamId, StreamResourceRevision, StreamRevision,
    ConversationRevision, ConversationRole, HandoffInput, IdentityId, MoveInput, MoveResult, SessionRole,
    AdministrativeClient,
    CanonicalHttpClient, HttpCommandScope, Scope,
    SynchronousSdk, RtpPacket,
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


async def invalid_http(client: CanonicalHttpClient, options: RequestOptions) -> None:
    HttpCommandScope(ParticipantId('wrong-conversation-family'))
    await client.request('operation.get',{},Scope(ConversationId('room')),options)
    CanonicalHttpClient('http://127.0.0.1','credential','automatic')
    values=await client.request('operation.get',{},HttpCommandScope(ConversationId('room')),options)
    values[0].value=values[0].value


def synchronous_roles(native: SynchronousSdk[Client], admin: SynchronousSdk[AdministrativeClient], options: RequestOptions) -> None:
    native.run(lambda client:client.provision({}))
    admin.run(lambda client:client.conversations.list(options))
    native._client=native._client


async def invalid_media(client: Client, options: RequestOptions) -> None:
    connection = client.conversations.by_id(ConversationId('room')).sessions.by_id(SessionId('session')).connections.by_id(ConnectionId('connection'))
    await connection.media.stream_by_id(StreamId('audio')).send_rtp(b'not-a-packet', options)
    connection.media.stream_by_id(ParticipantId('wrong-family'))
    RtpPacket('not-a-sequence', b'not-a-packet')


async def invalid_data(client: Client, options: RequestOptions, mutation: MutationOptions) -> None:
    stream=client.media.connection(ConversationId('room'),SessionId('session'),ConnectionId('connection')).stream_by_id(StreamId('data'))
    await stream.send_data(OperationId('wrong-id'),NonEmpty(ParticipantId('recipient')),BinaryData(b'bytes'),mutation)
    await stream.send_data(MessageId('message'),NonEmpty(ParticipantId('recipient')),b'untyped',mutation)
    await stream.send_data(MessageId('message'),NonEmpty(ParticipantId('recipient')),TextData('text'),options)
