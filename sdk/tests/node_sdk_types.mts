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

await session.handoff({via:connection.id,retain:connection.id,from:participant.id,to:participant.id,outgoing:'observe',role:{scope:'conversation',expectedConversationRevision:1 as SdkRevision<'conversation'>},expectedTopologyRevision:topology,consultation:consulting},{timeoutMs:1000,operationId});
// @ts-expect-error A Session topology revision cannot authorize a Conversation role assertion.
session.handoff({via:connection.id,retain:connection.id,from:participant.id,to:participant.id,outgoing:'leave',role:{scope:'conversation',expectedConversationRevision:topology},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error A message operation cannot supply a consultation continuation.
session.handoff({via:connection.id,retain:connection.id,from:participant.id,to:participant.id,outgoing:'leave',role:{scope:'session'},expectedTopologyRevision:topology,consultation:message},{timeoutMs:1000,operationId});
await connection.move({retain:connection.id,target:{endpoint:'endpoint' as import('../typescript/dist/index.js').EndpointId},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error A Connection ID cannot substitute for an Endpoint ID.
connection.move({retain:connection.id,target:{endpoint:connection.id},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error Movement target is exactly one endpoint or candidate.
connection.move({retain:connection.id,target:{endpoint:'endpoint' as import('../typescript/dist/index.js').EndpointId,candidate:connection.id},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});

const moveOperation=client.operations.restore(saved,client.operationKinds.connection_replace);
// @ts-expect-error Distinct invariant movement results cannot be relabeled as a handoff.
const wrongMovementResult:SdkOperation<import('../typescript/dist/index.js').SdkHandoffResult>=moveOperation;

declare const receiptMessageId:import('../typescript/dist/index.js').MessageId;
const deliveryFacts = await client.conversations.byId(roomId).receipts(receiptMessageId,{timeoutMs:1000});
// @ts-expect-error Delivery facts retain their independent readonly evidence boundary.
deliveryFacts.push(deliveryFacts[0]);
// @ts-expect-error A participant ID cannot name the message being observed.
client.conversations.byId(roomId).receipts(participant.id,{timeoutMs:1000});

const checkedEnvelope = client.raw.validate({v:2,id:'attempt',ts:'2026-10-28T00:00:00Z',type:'operation.get',payload:{operation_id:'original'}});
const rawReplies = await client.raw.request(checkedEnvelope,{timeoutMs:1000});
// @ts-expect-error Local validation token cannot be forged from ordinary JSON.
client.raw.request({value:checkedEnvelope.value},{timeoutMs:1000});
// @ts-expect-error Validated token value is immutable.
checkedEnvelope.value = null;
// @ts-expect-error Returned envelope batch is immutable.
rawReplies.push(checkedEnvelope);

import type {NodeAdministrativeClient,SdkTrustedProvisioningCommand} from '../typescript/dist/index.js';
declare const administrator:NodeAdministrativeClient;
const trustedCommand=administrator.validateProvisioning({type:'identity',operation_id:operationId,alias:'local',kind:'human',display_name:null});
await administrator.provision(trustedCommand,{timeoutMs:1000,operationId});
// @ts-expect-error Ordinary participant clients have no provisioning method.
client.provision(trustedCommand,{timeoutMs:1000,operationId});
// @ts-expect-error A raw canonical envelope is not a trusted administrative command.
administrator.provision(checkedEnvelope,{timeoutMs:1000,operationId});
// @ts-expect-error Structural JSON cannot forge a trusted administrative command.
const forgedTrusted:SdkTrustedProvisioningCommand={value:trustedCommand.value,operationId,referenceJson:()=>null};
// @ts-expect-error Administrative commands retain their immutable captured intent.
trustedCommand.value=null;
// @ts-expect-error Trusted backend exposes no participant Conversation API.
administrator.conversations.byId(roomId);
