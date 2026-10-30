import test from "node:test";
import assert from "node:assert/strict";
import {proposal,contextVersion,intentFor,approve,digest} from "./policy.mjs";
const context=()=>({conversationId:"conversation-1",stage:"takeover",planId:"business-plan",
    planVersion:1,sessionId:"session-1",sessionRevision:4,sessionState:"active",
    speech:{sessionId:"session-1",sourceId:"owned-speech-1",complete:true},
    approvers:[{participantId:"owner",membershipEpoch:3,active:true}]});
const binding=()=>({slot:"takeover",command:"session.handoff",
    scope:{conversationId:"conversation-1",sessionId:"session-1",connectionId:"original-primary"},
    activeSession:true,afterSpeech:true,speechSessionId:"session-1",
    approval:{participantId:"owner",purpose:"takeover"},
    buildInput:(_model,current)=>({via:"original-primary",retain:"original-remote",to:"owner",expectedTopologyRevision:current.sessionRevision})});
const accepted=(c=context(),b=binding(),eventId="event-1")=>intentFor({workerId:"worker-1",eventId,context:c,selected:proposal({action:"takeover"},{takeover:b}),binding:b});
const decision=(intent,c=context())=>({source:"authenticated-app-decision",recordId:"actual-owner-decision",
    participantId:"owner",membershipEpoch:c.approvers[0].membershipEpoch,conversationId:c.conversationId,
    planId:c.planId,planVersion:c.planVersion,purpose:"takeover",intentDigest:digest(intent),decision:"approve"});
test("model resource, approval, role and execution fields refuse before an app binding can run",()=>{
    const b=binding();let builds=0;b.buildInput=()=>{builds++;return {};};
    for(const extra of [{approved:true},{role:"owner"},{sessionId:"foreign"},{expectedRevision:99},
        {raw:{type:"session.handoff"}},{execute:"shell"},{operationId:"replacement"}]) {
        assert.throws(()=>proposal({action:"takeover",...extra},{takeover:b}),{code:"invalid-model-action"});
    }
    assert.equal(builds,0);assert.throws(()=>proposal({action:"unknown"},{takeover:b}),{code:"invalid-model-action"});
});
test("a completed owned speech source and current exact session fence takeover",()=>{
    const c=context();c.speech.complete=false;
    assert.throws(()=>accepted(c),{code:"speech-in-progress"});
    c.speech.complete=true;c.speech.sessionId="old-session";
    assert.throws(()=>accepted(c),{code:"stale-speech-source"});
    c.speech.sessionId="session-1";c.sessionState="ended";
    assert.throws(()=>accepted(c),{code:"stale-session"});
    c.sessionState="active";c.sessionId="new-session";
    assert.throws(()=>accepted(c),{code:"invalid-action-binding"});
});
test("booker contact requires both owned closing completion and authoritative Session end",()=>{
    const c=context();const b=binding();b.command="session.invite";b.slot="call-booker";b.activeSession=false;b.afterSessionEnd=true;b.inviteKind="new";b.sourceSessionId="session-1";b.scope={conversationId:"conversation-1",sessionId:"booker-session"};delete b.approval;b.buildInput=()=>({sessionId:"booker-session",to:["booker"],medium:"voice",intent:"call",capabilities:{}});
    assert.throws(()=>accepted(c,b),{code:"session-still-active"});
    c.sessionState="ended";assert.equal(accepted(c,b).command,"session.invite");
    c.speech.complete=false;assert.throws(()=>accepted(c,b),{code:"speech-in-progress"});
});
test("fresh exact owner decision approves only its current proposal/actor/epoch/scope",()=>{
    const c=context(),b=binding(),intent=accepted(c,b),good=decision(intent,c);
    approve(intent,c,b,good);
    for(const change of [{participantId:"assistant"},{membershipEpoch:2},{intentDigest:"copied"},
        {planId:"previous-proposal"},{planVersion:0},{conversationId:"other"},{purpose:"other"},
        {decision:"decline"},{source:"model-output"},{source:"incoming-text"},{recordId:""}]) {
        assert.throws(()=>approve(intent,c,b,{...good,...change}),{code:"approval-required"});
    }
    assert.throws(()=>approve(intent,c,b,null),{code:"approval-required"});
});
test("a later current Session or owner membership change invalidates the pending approval",()=>{
    const c=context(),b=binding(),intent=accepted(c,b),good=decision(intent,c);
    c.sessionRevision++;assert.throws(()=>approve(intent,c,b,good),{code:"stale-action-context"});
    const removed=context();removed.approvers[0].active=false;
    assert.throws(()=>approve(intent,removed,b,good),{code:"stale-action-context"});
});
test("slot mutation identity survives repeated events and changed model text",()=>{
    const c=context(),b={...binding(),command:"message.send",slot:"owner-followup",text:true,
        buildInput:(model,_current,id)=>({messageId:"message-"+id,text:model.text,to:{first:"owner",rest:[]}})};
    const first=intentFor({workerId:"worker-1",eventId:"event-1",context:c,selected:proposal({action:"notify",text:"first"},{notify:b}),binding:b});
    const second=intentFor({workerId:"worker-1",eventId:"duplicate-hint",context:c,selected:proposal({action:"notify",text:"changed"},{notify:b}),binding:b});
    assert.equal(first.operationId,second.operationId);assert.notEqual(digest(first),digest(second));
    assert.throws(()=>approve(second,c,b,decision(first,c)),{code:"approval-required"});
    assert.equal(Object.isFrozen(first.input),true);
});
test("bounded text accepts ordinary speech but rejects malformed or overlong model data",()=>{
    const bindings={message:{text:true}};
    assert.equal(proposal({action:"message",text:"Sandbox reply\nwith a second line"},bindings).action,"message");
    for(const text of ["","x".repeat(4097),"hidden\0control","\ud800"]) {
        assert.throws(()=>proposal({action:"message",text},bindings),{code:"invalid-model-text"});
    }
    assert.throws(()=>proposal({action:"message",text:"ok",approved:true},bindings),{code:"invalid-model-action"});
});
test("current-context metadata rejects cross-scope ambiguity and duplicate approvers",()=>{
    const c=context();c.approvers.push({...c.approvers[0]});assert.throws(()=>contextVersion(c),{code:"invalid-current-context"});
    const missing=context();delete missing.sessionId;assert.throws(()=>contextVersion(missing),{code:"invalid-current-context"});
});

test("handoff and device movement bindings cannot omit the approved business gates",()=>{
 const c=context();for(const changed of [{approval:undefined},{afterSpeech:false},{activeSession:false}]) {
  assert.throws(()=>accepted(c,{...binding(),...changed}),{code:"invalid-action-binding"});
 }
 const move={...binding(),command:"connection.replace",approval:{participantId:"owner",purpose:"takeover"}};
 assert.throws(()=>accepted(c,move),{code:"invalid-action-binding"});
});
test("ordinary chat needs no fabricated speech source and approver ordering is semantic",()=>{
 const c=context();c.speech=null;c.approvers.push({participantId:"organizer",membershipEpoch:1,active:true});
 assert.equal(contextVersion(c),contextVersion({...c,approvers:[...c.approvers].reverse()}));
 const b={...binding(),command:"message.send",afterSpeech:false,approval:undefined,slot:"chat",buildInput:()=>({text:"hello"})};
 const intent=accepted(c,b);assert.equal(intent.command,"message.send");
});
