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
