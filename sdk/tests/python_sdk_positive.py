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
    AdministrativeClient, TrustedProvisioningCommand, Json,
    CanonicalHttpClient, HttpCommandScope, HttpEnvelope,
    SynchronousSdk, CapabilitySnapshot, MediaNegotiation, MediaPacket, RtpPacket, SenderClockReport, LocalMediaSubmission,
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


async def administration(admin: AdministrativeClient, options: RequestOptions) -> None:
    command=admin.validate_provisioning({'type':'identity','operation_id':'original','alias':'local','kind':'human','display_name':'Local'})
    assert_type(command,TrustedProvisioningCommand)
    operation=await admin.provision(command,MutationOptions(OperationId('original'),options))
    assert_type(operation,Operation[Json])
    assert_type((await operation.wait(options)).value,Json)
    assert_type(admin.operations.recover(operation.reference),Operation[Json])
    assert_type(admin.operations.restore(command.reference_json()),Operation[Json|DecisionReceipt])
    assert_type(await operation.cancel(MutationOptions(OperationId('cancel'),options)),Operation[DecisionReceipt])


async def http_control(client: CanonicalHttpClient, options: RequestOptions) -> None:
    scope=HttpCommandScope(ConversationId('room'),operation_id=OperationId('original'))
    values=await client.request('message.send',{'text':'synthetic'},scope,options)
    assert_type(values,Sequence[HttpEnvelope])
    assert_type(values[0].to_json(),Json)


def synchronous(native: SynchronousSdk[Client], admin: SynchronousSdk[AdministrativeClient],
                http: SynchronousSdk[CanonicalHttpClient], options: RequestOptions) -> None:
    assert_type(native.run(lambda client:client.capabilities.refresh(options)),CapabilitySnapshot)
    operation=native.run(lambda client:client.conversations.create([IdentityId('peer')],'persistent',MutationOptions(OperationId('original'),options)))
    assert_type(operation,Operation[Conversation])
    assert_type(native.run(lambda client:operation.wait(options)).value,Conversation)
    assert_type(admin.run(lambda client:administration(client,options)),None)
    assert_type(http.run(lambda client:client.request('operation.get',{'operation_id':'original'},HttpCommandScope(),options)),Sequence[HttpEnvelope])


async def media_packets(client: Client, options: RequestOptions) -> None:
    connection = client.conversations.by_id(ConversationId('room')).sessions.by_id(SessionId('session')).connections.by_id(ConnectionId('connection'))
    assert_type(await connection.media.refresh(options), MediaNegotiation)
    stream = connection.streams.by_id(StreamId('audio')).media
    assert_type(await stream.send_rtp(RtpPacket(1, bytes.fromhex('80000001000000a000000001') + bytes([127])*160), options), LocalMediaSubmission)
    assert_type(await stream.receive_packet(options), MediaPacket)
    assert_type(await stream.send_clock(2, SenderClockReport(1, 2**64-1, 160, 1, 160, 'source'), options), LocalMediaSubmission)
