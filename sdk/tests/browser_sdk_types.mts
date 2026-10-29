// Compile-only public browser workflow; no Node helper types or protocol frames.
import {BrowserSdkClient,browserOperationKinds,sdkId,sdkRevision,type BrowserOperation,type BrowserConversation,type BrowserSession,type BrowserSdkOperationReference} from '../typescript/dist/browser.js';
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
