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
await connection.replaceDevice({retain:connection.id,target:{endpoint:'endpoint' as import('../typescript/dist/index.js').EndpointId},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error A Connection ID cannot substitute for an Endpoint ID.
connection.replaceDevice({retain:connection.id,target:{endpoint:connection.id},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});
// @ts-expect-error Movement target is exactly one endpoint or candidate.
connection.replaceDevice({retain:connection.id,target:{endpoint:'endpoint' as import('../typescript/dist/index.js').EndpointId,candidate:connection.id},expectedTopologyRevision:topology},{timeoutMs:1000,operationId});

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

const incomingReader=client.incomingInvitations({timeoutMs:1000});
const incomingResult=await incomingReader.next();
if(!incomingResult.done){
 const incomingHint=incomingResult.value;
 const sourceOperation:OperationId=incomingHint.operationId;
 const targetConversation:ConversationId=incomingHint.session.conversationId;
 const inviter:import('../typescript/dist/index.js').ParticipantId=incomingHint.from;
 // @ts-expect-error An invitation source does not grant native packet custody.
 incomingHint.session.native;
 // @ts-expect-error Incoming metadata is immutable.
 incomingHint.source.kind='foreign';
 // @ts-expect-error Session and participant identities remain distinct.
 const foreignParticipant:import('../typescript/dist/index.js').ParticipantId=incomingHint.session.id;
}
for await(const hint of incomingReader){const original:OperationId=hint.operationId;break;}
await incomingReader.close();
// @ts-expect-error Approved iterator options are captured at construction.
client.incomingInvitations();
// @ts-expect-error next uses captured stream options, not a separate per-wait API.
incomingReader.next({timeoutMs:1000});

import type {SdkCursor,SdkJournalEvent} from '../typescript/dist/index.js';
declare const cursor:SdkCursor;
const events=client.conversations.byId(roomId).events({after:cursor},{timeoutMs:1000});
for await(const fact of events){ const original:SdkJournalEvent=fact; void original; break; }
// @ts-expect-error Resource IDs cannot substitute for an opaque journal cursor.
client.conversations.byId(roomId).events({after:roomId},{timeoutMs:1000});
// @ts-expect-error Cursor cannot be used as a mutation ID.
const eventOperation:OperationId=cursor;
declare const fact:SdkJournalEvent;
// @ts-expect-error Event audiences retain their readonly evidence boundary.
fact.audience.push(fact.audience[0]);
// @ts-expect-error Journal data is immutable recursively.
const mutableData:{[key:string]:string}=fact.data;


import {SdkRtpPacket,SdkBinaryData,SdkJsonData} from '../typescript/dist/index.js';
import type {SdkDataAcceptance,SdkReceivedData,SdkMediaPacket,MessageId,SdkSenderClockReport} from '../typescript/dist/index.js';
const mediaStream=connection.streams.byId('media' as import('../typescript/dist/index.js').StreamId).media;
declare const dataMessageId:MessageId;
const dataOperation:SdkOperation<SdkDataAcceptance>=await mediaStream.sendData({messageId:dataMessageId,to:{first:participant.id,rest:[]},payload:new SdkJsonData({typed:true})},{timeoutMs:1000,operationId});
const restoredData:SdkOperation<SdkDataAcceptance>=client.operations.restore(dataOperation.referenceJson(),client.operationKinds.data_send);
const receivedData:SdkReceivedData=await mediaStream.receiveData({timeoutMs:1000});
const receivedMedia:SdkMediaPacket=await mediaStream.receivePacket({timeoutMs:1000});
const wideDataCounter:bigint=(await client.media.dataStats({timeoutMs:1000})).acceptedMessages;
// @ts-expect-error A message ID is distinct from an operation ID.
mediaStream.sendData({messageId:operationId,to:{first:participant.id,rest:[]},payload:new SdkBinaryData(new Uint8Array())},{timeoutMs:1000,operationId});
// @ts-expect-error Reliable data requires captured typed payloads, not raw bytes.
mediaStream.sendData({messageId:dataMessageId,to:{first:participant.id,rest:[]},payload:new Uint8Array()},{timeoutMs:1000,operationId});
// @ts-expect-error Data admission cannot be relabeled as ordinary Conversation message acceptance.
const wrongData:SdkOperation<SdkMessageAcceptance>=restoredData;
// @ts-expect-error NTP is an exact unsigned64 bigint, not a rounded number.
const lossyClock:SdkSenderClockReport={ssrc:1,ntp:123,rtp:2,packets:3,octets:4,cname:'source'};
// @ts-expect-error Datagram sequence is a numeric u32, not an ID.
new SdkRtpPacket(operationId,new Uint8Array());
// @ts-expect-error 64-bit counters cannot be implicitly rounded to number.
const lossyDataCounter:number=wideDataCounter;
// @ts-expect-error Captured data values cannot be replaced after construction.
new SdkJsonData({typed:true}).value=null;
// @ts-expect-error A Participant ID does not name a Connection's media owner.
client.media.connection({conversationId:roomId,sessionId:session.id,connectionId:participant.id});
void [receivedData,receivedMedia,wrongData,lossyClock,lossyDataCounter];
