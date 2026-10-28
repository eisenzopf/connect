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

const session=client.conversations.byId(roomId).sessions.byId('session' as import('../typescript/dist/index.js').SessionId);
declare const stateRevision:SdkRevision<'session'>;
// @ts-expect-error Session state revision cannot authorize a topology mutation.
session.end({reasonCode:1000,reason:'done',expectedTopologyRevision:stateRevision},{timeoutMs:1000,operationId});
// @ts-expect-error Invitation into an existing Session requires nonempty targets.
session.inviteParticipants({to:[],medium:'voice',intent:'join',capabilities:{},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error Session completion is not a Conversation result.
const wrongSessionResult:SdkOperation<SdkConversation>=client.operations.restore(saved,client.operationKinds.session_invite);

const connection=session.connections.byId('connection' as import('../typescript/dist/index.js').ConnectionId);
// @ts-expect-error Session topology revision cannot authorize native stream controls.
connection.update({action:'quality',intervalMs:250},topology,{timeoutMs:1000,operationId});
// @ts-expect-error Stream resource revision is not the negotiated native Stream revision.
connection.update({action:'quality',intervalMs:250},1 as SdkRevision<'stream-resource'>,{timeoutMs:1000,operationId});
// @ts-expect-error Connection results cannot become Session results.
const wrongConnectionResult:SdkOperation<import('../typescript/dist/index.js').SdkSession>=client.operations.restore(saved,client.operationKinds.connection_offer);

const capabilities = await client.capabilities.refresh({timeoutMs:1000});
// @ts-expect-error support set is immutable
capabilities.features.add('native-programmatic-media');
// @ts-expect-error Node facade cannot report the Rust runtime role
const helperRole: 'rust' = capabilities.runtimeRole;

const consulting = await session.consult({via:connection.id,retain:'retained' as import('../typescript/dist/index.js').ConnectionId,with:{first:participant.id,rest:[]},hold:'silence',expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
await consulting.returnToRetained({expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error A message operation cannot be a consultation continuation.
message.returnToRetained({expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
