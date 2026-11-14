import {test} from 'node:test';
import assert from 'node:assert/strict';
import {installOriginalCompletionGate} from '../lib/browser_matrix_completion_gate.mjs';
// Only listener/timer mechanics. These controls cannot prove signed Root/media.
class Socket extends EventTarget {close(){this.dispatchEvent(new Event('close'));}}
const root={intent:{source:0,ordinal:0,operation:'original-op'},cid:'c',sid:'s',old:'old',retained:'keep'};
const body=(reply='request')=>JSON.stringify({type:'operation.updated',id:'original-envelope',in_reply_to:reply,cid:'c',sid:'s',
    connid:'candidate',payload:{record_id:'original-record',operation_id:'original-op',command_type:'connection.replace',state:'completed',
        result:{replaces:'old',connection_id:'candidate',activation_state:'ready',retained_connection_ids:['keep']}}});
const send=(socket,data)=>{const event=new MessageEvent('message',{data});socket.dispatchEvent(event);return event;};
async function fixture(holdMs,work){
    const native=globalThis.WebSocket;globalThis.WebSocket=Socket;
    let gate,socket;
    try{gate=installOriginalCompletionGate({holdMs,source:0,ordinal:0});socket=new WebSocket();await work(gate,socket);}
    finally{gate?.close();socket?.close();globalThis.WebSocket=native;}
}
test('original completion and correlated ACK wait while unrelated control keeps flowing, with identical events',()=>fixture(30,async(gate,socket)=>{
    const seen=[];socket.addEventListener('message',e=>seen.push(e));gate.arm({...root,until:performance.now()+1000});
    const original=send(socket,body());const ack=send(socket,JSON.stringify({type:'ack',in_reply_to:'request',payload:{reply_complete:true}}));
    const ice=send(socket,JSON.stringify({type:'connection.ice-candidate',in_reply_to:'other'}));
    assert.deepEqual(seen,[ice]);gate.target('candidate');
    await new Promise(resolve=>setTimeout(resolve,60));assert.deepEqual(seen,[ice,original,ack]);
    assert.equal(seen[1].data,body());assert.equal(gate.report().held_events_remaining,0);
    gate.validated({operationId:'original-op',old:'old',target:'candidate'});
}));
test('original cutoff ends hold without renewing it; listener removal prevents delayed callback',()=>fixture(500,async(gate,socket)=>{
    const seen=[];const listen=e=>seen.push(e);socket.addEventListener('message',listen);
    gate.arm({...root,until:performance.now()+20});send(socket,body());socket.removeEventListener('message',listen);
    await new Promise(resolve=>setTimeout(resolve,60));assert.equal(seen.length,0);assert.equal(gate.report().expired,true);
    assert.equal(gate.report().held_events_remaining,0);
}));
test('cleanup releases original pending events and later traffic without creating or closing transport owners',()=>fixture(500,async(gate,socket)=>{
    const seen=[];socket.addEventListener('message',e=>seen.push(e));gate.arm({...root,until:performance.now()+1000});
    const original=send(socket,body());assert.equal(seen.length,0);gate.close();assert.deepEqual(seen,[original]);
    const after=send(socket,'after');assert.deepEqual(seen,[original,after]);assert.equal(gate.report().sockets_closed,0);
    assert.equal(gate.report().hook_closed,true);
}));
test('terminal-looking foreign root is not held and malformed input reaches original reader unchanged',()=>fixture(500,async(gate,socket)=>{
    const seen=[];socket.addEventListener('message',e=>seen.push(e));gate.arm({...root,until:performance.now()+1000});
    const foreign=send(socket,body().replace('original-op','foreign-op'));const malformed=send(socket,'{');
    assert.deepEqual(seen,[foreign,malformed]);assert.equal(gate.report().held_request_groups,0);
    assert.throws(()=>gate.validated({operationId:'foreign-op',old:'old',target:'candidate'}));
}));
test('once callback can re-register the same listener immediately after original dispatch',()=>fixture(30,async(gate,socket)=>{
    const seen=[];const listen=e=>seen.push(e);
    socket.addEventListener('message',listen,{once:true});const first=send(socket,'first');
    socket.addEventListener('message',listen,{once:true});const second=send(socket,'second');
    assert.deepEqual(seen,[first,second]);
}));
test('held once invocation remains original while the same callback registers as a fresh listener',()=>fixture(30,async(gate,socket)=>{
    const seen=[];const listen=e=>seen.push(e);gate.arm({...root,until:performance.now()+1000});
    socket.addEventListener('message',listen,{once:true});const first=send(socket,body());
    socket.addEventListener('message',listen,{once:true});const second=send(socket,'other');
    assert.deepEqual(seen,[second]);await new Promise(resolve=>setTimeout(resolve,60));
    assert.deepEqual(seen,[second,first]);
}));
test('aborted registrations permit the same listener again and discard only their own held invocation',()=>fixture(30,async(gate,socket)=>{
    const seen=[];const listen=e=>seen.push(e),already=new AbortController();already.abort();
    socket.addEventListener('message',listen,{signal:already.signal});
    socket.addEventListener('message',listen,{once:true});const first=send(socket,'first');assert.deepEqual(seen,[first]);
    const aborted=new AbortController();socket.addEventListener('message',listen,{signal:aborted.signal});
    gate.arm({...root,until:performance.now()+1000});send(socket,body());aborted.abort();
    socket.addEventListener('message',listen);const second=send(socket,'second');
    await new Promise(resolve=>setTimeout(resolve,60));assert.deepEqual(seen,[first,second]);
}));
test('same reply identifier on a different original socket is unrelated and passes immediately',()=>fixture(30,async(gate,socket)=>{
    const second=new WebSocket(),seen=[];
    try{
        const firstSeen=[];socket.addEventListener('message',e=>firstSeen.push(e));second.addEventListener('message',e=>seen.push(e));
        gate.arm({...root,until:performance.now()+1000});const original=send(socket,body());
        const other=send(second,JSON.stringify({type:'ack',in_reply_to:'request',payload:{}}));
        assert.deepEqual(seen,[other]);assert.deepEqual(firstSeen,[]);
        await new Promise(resolve=>setTimeout(resolve,60));assert.deepEqual(firstSeen,[original]);
    }finally{second.close();}
}));

test('same reply ID never holds retirement, auth, ICE or unrelated control types',()=>fixture(30,async(gate,socket)=>{
    const seen=[];socket.addEventListener('message',e=>seen.push(e));gate.arm({...root,until:performance.now()+1000});
    const original=send(socket,body());
    const unrelated=['connection.end','auth.bye','connection.ice-candidate','conversation.event','connection.update']
        .map(type=>send(socket,JSON.stringify({type,in_reply_to:'request',payload:{}})));
    assert.deepEqual(seen,unrelated);
    await new Promise(resolve=>setTimeout(resolve,60));assert.deepEqual(seen,[...unrelated,original]);
}));
