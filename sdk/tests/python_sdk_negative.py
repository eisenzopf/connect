"""Six unsuppressed public-type constraints; compilation must fail exactly six."""
from connect_sdk import (
    Client, Conversation, ConversationId, MessageId, MutationOptions, Operation,
    ParticipantId, RequestOptions, TopologyRevision,
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
