// Compile-only public browser workflow; no Node helper types or protocol frames.
import {BrowserSdkClient,browserOperationKinds,sdkId,sdkRevision,type BrowserSdkRequestOptions,type BrowserOperation,type BrowserConversation,type BrowserSession,type BrowserSdkOperationReference} from '../typescript/dist/browser.js';
import type {ConversationId,OperationId,SdkRevision} from '../typescript/dist/index.js';
declare const client:BrowserSdkClient;
declare const cid:ConversationId;
declare const saved:ReturnType<BrowserOperation<BrowserSession>['referenceJson']>;
const room=client.conversations.byId(cid);
const restored=client.operations.restore(saved,browserOperationKinds.sessionInvite);
const recovered:BrowserOperation<BrowserSession>=client.operations.recover(restored.reference);
const operationId:OperationId=restored.reference.operationId;
const topology:SdkRevision<'topology'>=sdkRevision<'topology'>(0);
// @ts-expect-error Result family is invariant and cannot be relabeled.
const foreignResult:BrowserOperation<BrowserConversation>=client.operations.recover(restored.reference);
// @ts-expect-error Original scope identities cannot replace the logical operation ID.
const foreignId:OperationId=restored.reference.scope.sessionId!;
// @ts-expect-error Saved operation identity is immutable.
restored.reference.operationId=sdkId<'operation'>('replacement');
// @ts-expect-error A raw shape cannot manufacture an invariant typed original reference.
const forged:BrowserSdkOperationReference<BrowserSession>={operationId,command:'session.invite',scope:{}};
// @ts-expect-error Session revisions do not authorize topology changes.
room.sessions.byId(sdkId<'session'>('session')).end({reasonCode:200,reason:'finished',expectedTopologyRevision:sdkRevision<'session'>(0)},{timeoutMs:1000,operationId});
const incoming=client.incomingInvitations({timeoutMs:1000});
const hint=(await incoming.next()).value;
if(hint){
 const snapshot=await hint.session.refresh({timeoutMs:1000});
 const decision=await hint.session.accept({capabilities:hint.capabilities,expectedTopologyRevision:snapshot.topologyRevision},{timeoutMs:1000,operationId});
 const accepted:BrowserSession=(await decision.wait({timeoutMs:1000})).value;
 // @ts-expect-error Session identity cannot become a Participant target.
 room.sessions.invite({sessionId:accepted.id,to:[accepted.id],medium:'text-chat',intent:'synthetic',capabilities:{}},{timeoutMs:1000,operationId});
 // @ts-expect-error Incoming source metadata is immutable.
 hint.source.correlationId=operationId;
}
await incoming.close();
void [recovered,topology,foreignId,foreignResult,forged];

const connection=room.sessions.byId(sdkId<'session'>('session')).connections.byId(sdkId<'connection'>('connection'));
const stream=connection.streams.byId(sdkId<'stream'>('source'));
// @ts-expect-error Stream resource revisions cannot authorize a native Connection end.
connection.end({reasonCode:200,reason:'finished',expectedStreamRevision:sdkRevision<'stream-resource'>(1)},{timeoutMs:1000,operationId});
// @ts-expect-error Connection identity cannot name a Stream.
connection.streams.byId(connection.id);
// @ts-expect-error Stream observations cannot grant local source custody.
const source:'local-source'=(await stream.refresh({timeoutMs:1000})).ownership;
void source;

const role:'browser'=client.role;
// @ts-expect-error The browser runtime role cannot be changed by application code.
client.role='node';
void role;

const capture=await connection.capture({audio:true},{timeoutMs:1000});
// @ts-expect-error Ordinary capture handles expose no protocol media sender.
capture.connection;
const renderer=await connection.render({audio:false},{timeoutMs:1000});
// @ts-expect-error Ordinary render handles expose no borrowed RTP/frame callback API.
renderer.video;
void [capture,renderer];

// Local UI helpers preserve the original Operation's invariant result type.
import { BrowserOperationProgress, BrowserSourcePicker, BrowserCaptureControls, browserInputDevices } from "../typescript/dist/browser.js";
export function browserUiTypes(operation: import("../typescript/dist/browser.js").BrowserOperation<import("../typescript/dist/browser.js").BrowserConnection>, connection: import("../typescript/dist/browser.js").BrowserConnection) {
    const picker = new BrowserSourcePicker({audio:true,video:["camera","screen"]});
    const capture = new BrowserCaptureControls(connection,picker.selection(),{timeoutMs:1000});
    const progress = new BrowserOperationProgress(operation,{timeoutMs:1000});
    const completion: Promise<import("../typescript/dist/browser.js").BrowserCompletion<import("../typescript/dist/browser.js").BrowserConnection>> = progress.observe();
    // @ts-expect-error Local source selection cannot grant a Connection or mutation.
    picker.selection().offer();
    // @ts-expect-error UI media owner hides RTP and protocol access.
    capture.start().then(owner => owner.sendRtp({}));
    return {completion,devices:browserInputDevices({timeoutMs:1000})};
}

import type { BrowserConnectionUpdateReceipt, BrowserStreamSelectionReceipt } from "../typescript/dist/browser.js";
export function browserNativeControlTypes(connection: import("../typescript/dist/browser.js").BrowserConnection, stream: import("../typescript/dist/browser.js").StreamId, revision: import("../typescript/dist/browser.js").SdkRevision<"stream">, mutation: import("../typescript/dist/browser.js").BrowserSdkMutationOptions) {
    const update: Promise<import("../typescript/dist/browser.js").BrowserOperation<BrowserConnectionUpdateReceipt>> = connection.update({action:"mute",streams:{first:stream,rest:[]}},revision,mutation);
    const select: Promise<import("../typescript/dist/browser.js").BrowserOperation<BrowserStreamSelectionReceipt>> = connection.streams.subscribe({selectors:{first:{stream},rest:[]},expectedStreamRevision:revision},mutation);
    // @ts-expect-error Resource revisions cannot assert a negotiated native revision.
    connection.update({action:"resume"},{} as import("../typescript/dist/browser.js").SdkRevision<"connection-resource">,mutation);
    // @ts-expect-error A logical source selector is not a renderer or capture owner.
    select.then(async op => (await op.wait({timeoutMs:1000})).value.bindings?.[0].encodeVideo({}));
    return {update,select};
}

import type {BrowserSessionUpdate,BrowserSessionMedium} from '../typescript/dist/browser.js';
export function browserSessionControls(session: BrowserSession, participant: import('../typescript/dist/browser.js').ParticipantId, revision: SdkRevision<'topology'>, options: import('../typescript/dist/browser.js').BrowserSdkMutationOptions) {
 const update: BrowserSessionUpdate={action:'role-change',participantId:participant,role:'observer'};
 const mutation:Promise<BrowserOperation<BrowserSession>>=session.update(update,revision,options);
 const medium:BrowserSessionMedium='mixed';
 session.inviteParticipants({to:{first:participant,rest:[]},medium,intent:'join',capabilities:{},expectedTopologyRevision:revision},options);
 // @ts-expect-error A participant-leave cannot silently carry a role change.
 session.update({action:'participant-leave',participantId:participant,role:'observer'},revision,options);
 // @ts-expect-error Session resource revisions cannot authorize topology controls.
 session.update(update,sdkRevision<'session'>(1),options);
 // @ts-expect-error Existing Session invitations require a nonempty target set.
 session.inviteParticipants({to:[],medium,intent:'join',capabilities:{},expectedTopologyRevision:revision},options);
 session.connections.offer({connectionId:sdkId<'connection'>('new'),purpose:'handoff-preparation',streams:{first:{id:sdkId<'stream'>('audio'),kind:'audio',direction:'sendrecv',codecs:{first:'opus',rest:[]}},rest:[]}},options);
 // @ts-expect-error Offered streams use the frozen NonEmpty shape, not an array.
 session.connections.offer({connectionId:sdkId<'connection'>('new'),streams:[{id:sdkId<'stream'>('audio'),kind:'audio',direction:'sendrecv',codecs:['opus']}]},options);
 return mutation;
}

client.conversations.create({policy:'persistent',participants:[sdkId<'identity'>('recipient')]},{timeoutMs:1000,operationId});
// @ts-expect-error Conversation creation takes Identity IDs, not uncontracted role objects.
client.conversations.create({policy:'persistent',participants:[{identityId:sdkId<'identity'>('recipient'),role:'participant'}]},{timeoutMs:1000,operationId});

import {browserCursor} from '../typescript/dist/browser.js';
const textSend:Promise<BrowserOperation<import('../typescript/dist/browser.js').BrowserMessageAcceptance>>=room.send({messageId:sdkId<'message'>('message'),to:{first:sdkId<'participant'>('recipient'),rest:[]},text:'hello'},{timeoutMs:1000,operationId});
const journal=room.events({after:browserCursor('saved-cursor')},{timeoutMs:1000});
const item:import('../typescript/dist/browser.js').BrowserJournalEvent|undefined=(await journal.next()).value;
// @ts-expect-error Acceptance is not a completed recipient delivery.
const delivered:'delivered'=(await (await textSend).wait({timeoutMs:1000})).value.boundary;
// @ts-expect-error A Message ID cannot replace a Participant recipient.
room.send({messageId:sdkId<'message'>('message'),to:{first:sdkId<'message'>('message'),rest:[]},text:'hello'},{timeoutMs:1000,operationId});
// @ts-expect-error Journal cursors cannot be manufactured by passing an ordinary string.
room.events({after:'raw-cursor'},{timeoutMs:1000});
void [item,delivered];
await journal.close();

const added=room.participants.add({identityId:sdkId<'identity'>('recipient'),kind:'human',role:'reviewer',expectedRevision:sdkRevision<'conversation'>(1)},{timeoutMs:1000,operationId});
const participant=(await (await added).wait({timeoutMs:1000})).value;
participant.update({role:'observer',expectedConversationRevision:sdkRevision<'conversation'>(2)},{timeoutMs:1000,operationId});
participant.remove({expectedConversationRevision:sdkRevision<'conversation'>(3)},{timeoutMs:1000,operationId});
// @ts-expect-error Topology revisions cannot assert Conversation membership state.
participant.remove({expectedConversationRevision:sdkRevision<'topology'>(3)},{timeoutMs:1000,operationId});
// @ts-expect-error Participant admission requires an Identity, not a Participant ID.
room.participants.add({identityId:sdkId<'participant'>('recipient'),kind:'human',role:'reviewer',expectedRevision:sdkRevision<'conversation'>(1)},{timeoutMs:1000,operationId});
// @ts-expect-error Identity kinds are the frozen closed union.
room.participants.add({identityId:sdkId<'identity'>('recipient'),kind:'robot',role:'reviewer',expectedRevision:sdkRevision<'conversation'>(1)},{timeoutMs:1000,operationId});

const delivery:readonly import('../typescript/dist/browser.js').BrowserDeliveryObservation[]=await room.receipts(sdkId<'message'>('message'),{timeoutMs:1000});
// @ts-expect-error A Participant ID cannot replace the Message whose receipts are read.
room.receipts(sdkId<'participant'>('recipient'),{timeoutMs:1000});
// @ts-expect-error Delivery evidence is immutable.
delivery[0].state='read';

const cancelled:BrowserOperation<import('../typescript/dist/browser.js').BrowserDecisionReceipt>=await (await textSend).cancel({timeoutMs:1000,operationId:sdkId<'operation'>('decision')});
const reconciled:BrowserOperation<import('../typescript/dist/browser.js').BrowserDecisionReceipt>=await (await textSend).reconcile({outcome:'completed',evidenceId:'attributed-observation'},{timeoutMs:1000,operationId:sdkId<'operation'>('reconcile')});
const target:import('../typescript/dist/browser.js').OperationId=(await cancelled.wait({timeoutMs:1000})).value.target;
// @ts-expect-error Cancellation returns a decision receipt, not a message acceptance.
const acceptance:import('../typescript/dist/browser.js').BrowserMessageAcceptance=(await cancelled.wait({timeoutMs:1000})).value;
// @ts-expect-error Reconciliation admits only terminal outcomes and an evidence reference.
(await textSend).reconcile({outcome:'ready',evidenceId:'e'},{timeoutMs:1000,operationId});
void [reconciled,target,acceptance];

const localSupport:import('../typescript/dist/browser.js').BrowserSdkCapabilitySnapshot=await client.capabilities.refresh({timeoutMs:1000});
const browserRole:'browser'=localSupport.runtimeRole;
await client.capabilities.require('original-id-operations',{timeoutMs:1000});
// @ts-expect-error A support snapshot cannot grant a native Rust role.
const nativeRole:'rust'=localSupport.runtimeRole;
// @ts-expect-error The local feature set has no mutation API.
localSupport.features.add('decoded-media-ready');
// @ts-expect-error Selected transport is an immutable observation.
localSupport.selectedTransport='webtransport';
void [browserRole,nativeRole];

const consulting=await room.sessions.byId(sdkId<'session'>('s')).consult({via:sdkId<'connection'>('primary'),retain:sdkId<'connection'>('remote'),with:{first:sdkId<'participant'>('one'),rest:[sdkId<'participant'>('two')]},hold:'configured-media',expectedTopologyRevision:sdkRevision<'topology'>(2)},{timeoutMs:1000,operationId});
const consultation:import('../typescript/dist/browser.js').BrowserConsultationResult=(await consulting.wait({timeoutMs:1000})).value;
const returning=await consulting.returnToRetained({expectedTopologyRevision:sdkRevision<'topology'>(3)},{timeoutMs:1000,operationId:sdkId<'operation'>('return')});
const returned:BrowserSession=(await returning.wait({timeoutMs:1000})).value;
const hostRecord:import('../typescript/dist/browser.js').BrowserOperationRecordId|undefined=returned.receiptConsultationReturn?.consultationRecordId;
// @ts-expect-error Returning requires a consultation result, not a Message operation.
(await textSend).returnToRetained({expectedTopologyRevision:sdkRevision<'topology'>(3)},{timeoutMs:1000,operationId});
// @ts-expect-error A resource revision cannot assert the consultation topology.
consulting.returnToRetained({expectedTopologyRevision:sdkRevision<'session'>(3)},{timeoutMs:1000,operationId});
// @ts-expect-error Receipt topology does not expose a mutable admitted list.
consultation.admitted.push(consultation.retained[0]);
// @ts-expect-error A logical mutation ID is not a host operation-record identity.
const falseRecord:import('../typescript/dist/browser.js').BrowserOperationRecordId=operationId;
void [hostRecord,falseRecord];

const transfer=await room.sessions.byId(sdkId<'session'>('s')).handoff({via:sdkId<'connection'>('primary'),from:sdkId<'participant'>('from'),to:sdkId<'participant'>('to'),retain:sdkId<'connection'>('remote'),outgoing:'leave',role:{scope:'conversation',expectedConversationRevision:sdkRevision<'conversation'>(7)},expectedTopologyRevision:sdkRevision<'topology'>(3),consultation:consulting,policyRef:'target-offer-record'},{timeoutMs:1000,operationId});
const transferred:import('../typescript/dist/browser.js').BrowserHandoffResult=(await transfer.wait({timeoutMs:1000})).value;
const moved:import('../typescript/dist/browser.js').BrowserMoveResult=(await(await connection.replaceDevice({retain:sdkId<'connection'>('remote'),target:{endpoint:sdkId<'endpoint'>('new-endpoint')},expectedTopologyRevision:sdkRevision<'topology'>(3)},{timeoutMs:1000,operationId})).wait({timeoutMs:1000})).value;
const endpoint:import('../typescript/dist/browser.js').EndpointId=moved.binding.endpointId;
// @ts-expect-error Endpoint targets cannot be manufactured from Connection IDs.
connection.replaceDevice({retain:sdkId<'connection'>('remote'),target:{endpoint:sdkId<'connection'>('candidate')},expectedTopologyRevision:sdkRevision<'topology'>(3)},{timeoutMs:1000,operationId});
// @ts-expect-error Device movement must choose one target kind.
connection.replaceDevice({retain:sdkId<'connection'>('remote'),target:{endpoint:sdkId<'endpoint'>('new'),candidate:sdkId<'connection'>('candidate')},expectedTopologyRevision:sdkRevision<'topology'>(3)},{timeoutMs:1000,operationId});
// @ts-expect-error A Message operation cannot supply a consultation continuation.
room.sessions.byId(sdkId<'session'>('s')).handoff({via:sdkId<'connection'>('primary'),from:sdkId<'participant'>('from'),to:sdkId<'participant'>('to'),retain:sdkId<'connection'>('remote'),outgoing:'observe',role:{scope:'session'},expectedTopologyRevision:sdkRevision<'topology'>(3),consultation:await textSend},{timeoutMs:1000,operationId});
// @ts-expect-error Captured binding metadata is immutable.
moved.binding.generation=3;
// @ts-expect-error Captured role metadata is immutable.
if(transferred.roleChange)transferred.roleChange.cleanup='pending';
void [transferred,endpoint];

const validated:import('../typescript/dist/browser.js').BrowserValidatedEnvelope=client.raw.validate({v:2,id:'validation-only',ts:'2026-10-29T00:00:00Z',type:'operation.get',payload:{operation_id:operationId}});
const replies:readonly import('../typescript/dist/browser.js').BrowserValidatedEnvelope[]=await client.raw.request(validated,{timeoutMs:1000});
// @ts-expect-error A caller shape cannot manufacture a validated envelope witness.
client.raw.request({value:{}},{timeoutMs:1000});
// @ts-expect-error Validated original envelopes are immutable observations.
validated.value={};
// @ts-expect-error The bounded correlated response array cannot be mutated.
replies.push(validated);
void replies;

const flatConsultation:import('../typescript/dist/browser.js').BrowserTopologyChange=consultation;
const flatHandoff:import('../typescript/dist/browser.js').BrowserTopologyChange=transferred;
const flatMove:import('../typescript/dist/browser.js').BrowserTopologyChange=moved;
const originalConsultation:import('../typescript/dist/browser.js').BrowserConsultationOperation=consulting;
// @ts-expect-error The frozen TypeScript result exposes direct topology, with no nested compatibility alias.
consultation.topology;
// @ts-expect-error Return uses the frozen explicit input shape, never a positional revision alias.
consulting.returnToRetained(sdkRevision<'topology'>(3),{timeoutMs:1000,operationId});
void [flatConsultation,flatHandoff,flatMove,originalConsultation];

const boundedClose: (options: BrowserSdkRequestOptions) => Promise<void> = client.close.bind(client);
void boundedClose;
// @ts-expect-error Shutdown requires a bounded wait.
await client.close();

// @ts-expect-error Preparation selects a supported purpose, never an injected ready claim.
room.sessions.byId(sdkId<'session'>('session')).connections.offer({connectionId:sdkId<'connection'>('prepared'),purpose:'ready',streams:{first:{id:sdkId<'stream'>('audio'),kind:'audio',direction:'sendrecv',codecs:{first:'opus',rest:[]}},rest:[]}},{timeoutMs:1000,operationId});
