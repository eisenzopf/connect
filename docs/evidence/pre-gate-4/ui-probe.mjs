import fs from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const source = fs.readFileSync('examples/conversation-client/app.js','utf8');
const refresh = source.slice(source.indexOf('async function refresh()'),source.indexOf('byId("message-form")'));
const element = () => ({children:[], append(...v){this.children.push(...v)}, appendChild(v){this.children.push(v)}, replaceChildren(){this.children=[]}});
const list = element(); const watermark = element();
const sandbox = {needConversation:()=> 'conversation', needClient:()=>({
 history:async()=>({messages:Array.from({length:100},(_,i)=>({id:`m${i}`,sender:'human',body:`message ${i}`})),next:'next-page'}),
 contextView:async()=>({watermark:101,omitted:1,items:[{kind:'message',source_id:'m100',body:'Newest message',sequence:101}],deliveries:[{state:'failed'}]})
}), byId:id=>id==='messages'?list:watermark, document:{createElement:element}, Set};
vm.createContext(sandbox);
await vm.runInContext(refresh+'\nrefresh()',sandbox);
assert.equal(list.children.length,100);
assert(!JSON.stringify(list).includes('Newest message'));
assert(!JSON.stringify(list).includes('failed'));
console.log('REPRODUCED: refresh renders first 100 messages, drops newer message context, and does not display failed context delivery. Pure-function fixture, not browser E2E.');
