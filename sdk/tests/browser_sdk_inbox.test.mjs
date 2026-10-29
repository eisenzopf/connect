import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserInvitationInbox} from '../typescript/dist/browser_sdk_inbox.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
// Internal metadata bounds only. Signature/physical/current-authority coverage
// belongs to the signed SDK tests and installed Chrome fixture.
const invite=(source,extra={})=>envelope('session.invite',{from:'sender',to:['recipient'],medium:'voice',intent:'synthetic',capabilities_offer:{audio:['opus']},...extra},{cid:'room',sid:'session',operation_id:source});
const cancel=frame=>envelope('session.cancel',{by:'sender',reason_code:487,reason:'Cancelled'},{cid:'room',sid:'session',operation_id:frame.operation_id,in_reply_to:frame.id});
test('invitation consumed-source history has an explicit 256-source horizon',()=>{
 const inbox=new BrowserInvitationInbox();for(let i=0;i<257;i++){assert(inbox.record(invite('source-'+i),1));assert(inbox.consume(inbox.peek()));}
 assert.equal(inbox.record(invite('source-1'),1),false);assert(inbox.record(invite('source-0'),1));assert.equal(inbox.peek().frame.operation_id,'source-0');
});
test('invitation consumed payload history has a byte horizon independent of source count',()=>{
 const inbox=new BrowserInvitationInbox();for(let i=0;i<18;i++){assert(inbox.record(invite('source-'+i,{extension:'x'.repeat(60000)}),1));assert(inbox.consume(inbox.peek()));}
 assert.equal(inbox.record(invite('source-1',{extension:'x'.repeat(60000)}),1),false);assert(inbox.record(invite('source-0',{extension:'x'.repeat(60000)}),1));
});
test('queued source remains pinned when consumed/cancelled history evicts its dedup entry',()=>{
 const inbox=new BrowserInvitationInbox(),original=invite('pinned');inbox.record(original,1);
 for(let i=0;i<257;i++){const next=invite('source-'+i);inbox.record(next,1);assert(inbox.record(cancel(next),1));}
 assert.equal(inbox.record(invite('pinned'),1),false);assert.equal(inbox.peek().frame.operation_id,'pinned');assert(inbox.consume(inbox.peek()));assert.equal(inbox.peek(),undefined);
});
test('correlated invitation cancellation follows only the four most recent wire IDs',()=>{
 const inbox=new BrowserInvitationInbox(),old=invite('source');inbox.record(old,1);let latest;
 for(let i=0;i<5;i++){latest=invite('source');assert.equal(inbox.record(latest,1),false);}
 assert.equal(inbox.record(cancel(old),1),false);assert(inbox.peek());assert(inbox.record(cancel(latest),1));assert.equal(inbox.peek(),undefined);assert.equal(inbox.record(invite('source'),1),false);
});
test('invitation capture excludes envelope extensions, isolates payload and enforces UTF8 bounds',()=>{
 const inbox=new BrowserInvitationInbox(),original=invite('source');original.unretained_extension='x'.repeat(70000);inbox.record(original,1);original.payload.intent='changed';const hint=inbox.peek();assert.equal(hint.frame.payload.intent,'synthetic');assert.equal(hint.frame.unretained_extension,undefined);assert.throws(()=>inbox.record(invite('oversize',{extension:'😀'.repeat(20000)}),1),{code:'protocol'});inbox.clear();assert.equal(inbox.peek(),undefined);
});
