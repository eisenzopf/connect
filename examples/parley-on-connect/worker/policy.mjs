import { createHash } from "node:crypto";
import { stateJson } from "./state.mjs";
const fail = code => { const error = Error(code); error.code = code; throw error; };
const identifier = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
const revision = value => Number.isSafeInteger(value) && value >= 0;
const exact = (value, keys) => value && typeof value === "object" && !Array.isArray(value) &&
    Object.keys(value).every(key => keys.includes(key));
const frozen = value => { if(value && typeof value === "object") {Object.values(value).forEach(frozen);Object.freeze(value);}return value; };
export const digest = value => createHash("sha256").update(stateJson(value)).digest("hex");

/** Model output has no resource IDs, revisions, approval, executable code or
 * provider calls. Targets and public SDK inputs belong to reviewed app bindings. */
export function proposal(value, bindings) {
    if (!exact(value,["action","text"]) || !identifier(value.action) ||
        !Object.hasOwn(bindings,value.action)) fail("invalid-model-action");
    const binding=bindings[value.action];
    if (binding.text === true) {
        if(typeof value.text!=="string" || !value.text || Buffer.byteLength(value.text)>4096 ||
            /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/u.test(value.text))
            fail("invalid-model-text");
        for(const scalar of value.text) {
            const cp=scalar.codePointAt(0);if(cp>=0xd800 && cp<=0xdfff) fail("invalid-model-text");
        }
    } else if(value.text!==undefined) fail("invalid-model-action");
    return frozen(JSON.parse(stateJson(value)));
}
export function contextVersion(context) {
    if(!exact(context,["conversationId","stage","planId","planVersion","sessionId","sessionRevision",
        "sessionState","speech","approvers"]) || !identifier(context.conversationId) ||
        !identifier(context.stage) || !identifier(context.planId) || !revision(context.planVersion) ||
        !Array.isArray(context.approvers) || context.approvers.length>8) fail("invalid-current-context");
    const hasSession=context.sessionId!==undefined;
    if(hasSession && (!identifier(context.sessionId) || !revision(context.sessionRevision) ||
        !["active","ended"].includes(context.sessionState))) fail("invalid-current-context");
    if(!hasSession && [context.sessionRevision,context.sessionState].some(x=>x!==undefined)) fail("invalid-current-context");
    if(context.speech!==null && (!exact(context.speech,["sessionId","sourceId","complete"]) ||
        !identifier(context.speech.sessionId) || !identifier(context.speech.sourceId) ||
        typeof context.speech.complete!=="boolean")) fail("invalid-current-context");
    const seen=new Set();
    for(const member of context.approvers) {
        if(!exact(member,["participantId","membershipEpoch","active"]) ||
            !identifier(member.participantId) || !revision(member.membershipEpoch) || member.membershipEpoch<1 ||
            typeof member.active!=="boolean" || seen.has(member.participantId)) fail("invalid-current-context");
        seen.add(member.participantId);
    }
    return digest({...context,approvers:[...context.approvers].sort((a,b)=>a.participantId<b.participantId?-1:a.participantId>b.participantId?1:0)});
}
const commands = new Set(["message.send","session.invite","session.end",
    "session.consult","session.consult.return","session.handoff","connection.replace"]);
/** Binding is trusted compiled application code. This function is pure; no SDK
 * mutation runs until the resulting original intent is persisted and approved. */
export function intentFor({workerId, eventId, context, selected, binding}) {
    contextVersion(context);
    const newInvite=binding.command==="session.invite" && binding.inviteKind==="new";
    if(!identifier(workerId) || !identifier(eventId) || !identifier(binding.slot) ||
        !commands.has(binding.command) || typeof binding.buildInput!=="function" ||
        !exact(binding.scope,["conversationId","sessionId","connectionId"]) ||
        binding.scope.conversationId!==context.conversationId ||
        (binding.scope.sessionId!==undefined && !identifier(binding.scope.sessionId)) ||
        (binding.scope.connectionId!==undefined && !identifier(binding.scope.connectionId)) ||
        (newInvite ? binding.sourceSessionId!==context.sessionId || binding.scope.sessionId===context.sessionId :
            binding.scope.sessionId!==undefined && binding.scope.sessionId!==context.sessionId))
        fail("invalid-action-binding");
    if(binding.command==="session.invite" && !["new","participants"].includes(binding.inviteKind)) fail("invalid-action-binding");
    if(binding.command==="session.handoff" && (binding.afterSpeech!==true || binding.activeSession!==true || binding.approval?.purpose!=="takeover")) fail("invalid-action-binding");
    if(binding.command==="connection.replace" && (binding.activeSession!==true || binding.approval?.purpose!=="device-move")) fail("invalid-action-binding");
    if(binding.afterSpeech===true && !context.speech?.complete) fail("speech-in-progress");
    if(binding.afterSessionEnd===true && context.sessionState!=="ended") fail("session-still-active");
    if(binding.activeSession===true && context.sessionState!=="active") fail("stale-session");
    if(binding.afterSpeech===true && binding.speechSessionId!==context.speech.sessionId) fail("stale-speech-source");
    const identity={workerId,conversationId:context.conversationId,planId:context.planId,
        planVersion:context.planVersion,stage:context.stage,slot:binding.slot};
    // Slot identity is independent of model text. A duplicate notification or
    // changed model answer cannot hide the original logical mutation.
    const operationId="parley-"+digest(identity).slice(0,48);
    const body=binding.buildInput(selected,structuredClone(context),operationId);
    const intent={profile:"parley-worker-intent/1",...identity,eventId,
        command:binding.command,...(binding.command==="session.invite"?{inviteKind:binding.inviteKind}:{}),
        operationId,scope:structuredClone(binding.scope),input:body,
        contextVersion:contextVersion(context)};
    return frozen(JSON.parse(stateJson(intent)));
}
/** Call only with a fresh result from the application's authenticated decision
 * reader. Incoming event bodies and model output never call this reader or
 * become decisions merely by containing these same fields. */
export function approve(intent, current, binding, decision) {
    if(contextVersion(current)!==intent.contextVersion) fail("stale-action-context");
    if(binding.approval===undefined) return;
    const required=binding.approval;
    if(!exact(required,["participantId","purpose"]) || !identifier(required.participantId) ||
        !identifier(required.purpose)) fail("invalid-approval-policy");
    const actor=current.approvers.find(x=>x.participantId===required.participantId);
    if(!actor?.active) fail("approval-actor-not-current");
    if(!exact(decision,["source","recordId","participantId","membershipEpoch","conversationId",
        "planId","planVersion","purpose","intentDigest","decision"]) ||
        decision.source!=="authenticated-app-decision" || !identifier(decision.recordId) ||
        decision.participantId!==actor.participantId || decision.membershipEpoch!==actor.membershipEpoch ||
        decision.conversationId!==intent.conversationId || decision.planId!==intent.planId ||
        decision.planVersion!==intent.planVersion || decision.purpose!==required.purpose ||
        decision.intentDigest!==digest(intent) || decision.decision!=="approve")
        fail("approval-required");
}
