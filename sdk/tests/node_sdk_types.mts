import type {NodeClient,SdkOperation,SdkMessageAcceptance,SdkConversation,OperationId,ConversationId,SdkRevision} from '../typescript/dist/index.js';
declare const client:NodeClient;
declare const saved:ReturnType<SdkOperation<SdkMessageAcceptance>['referenceJson']>;
const message=client.operations.restore(saved,client.operationKinds.message_send);
const boundId:OperationId=message.reference.operationId;
// @ts-expect-error Scope identity cannot be used as an operation ID.
const foreignId:OperationId=message.reference.scope.conversationId!;
const same:SdkOperation<SdkMessageAcceptance>=client.operations.recover(message.reference);
// @ts-expect-error Result phantom is invariant; a message cannot become a room.
const wrong:SdkOperation<SdkConversation>=client.operations.recover(message.reference);
// @ts-expect-error Original operation identity is immutable.
message.reference.operationId='replacement';
// @ts-expect-error Saved scope is immutable.
message.reference.scope.conversationId='foreign';
declare const operationId:OperationId;
// @ts-expect-error Different identity families cannot substitute.
const conversationId:ConversationId=operationId;
void [same,wrong,conversationId];

declare const topology:SdkRevision<'topology'>;
declare const roomId:ConversationId;
const participant=client.conversations.byId(roomId).participants.byId('participant' as import('../typescript/dist/index.js').ParticipantId);
// @ts-expect-error Topology revision cannot authorize a Conversation membership mutation.
participant.remove({expectedConversationRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error A bare number is not a validated branded Conversation revision.
participant.update({role:'observer',expectedConversationRevision:1},{timeoutMs:1000,operationId});
