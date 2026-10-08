import fs from 'node:fs';
import {randomUUID} from 'node:crypto';
const cfg=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
const socket=new WebSocket(cfg.url);
const pending=new Map();
const deadline=setTimeout(()=>{socket.close();process.exit(2)},15000);
socket.addEventListener('message',e=>{
 const value=JSON.parse(e.data);const match=pending.get(value.in_reply_to);
 if(match){pending.delete(value.in_reply_to);match(value);}
});
await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',()=>reject(new Error('connection failed')),{once:true});});
function exchange(type,payload){
 const id='env_'+randomUUID().replaceAll('-','');
 const response=new Promise(resolve=>pending.set(id,resolve));
 socket.send(JSON.stringify({v:1,type,id,ts:new Date().toISOString(),payload}));return response;
}
async function request(op){
 const r=await exchange('connect.request',{profile:'connect-control/1',...op});
 if(r.type!=='connect.result'||r.payload.profile!=='connect-control/1'||!r.payload.result.Ok)throw new Error('operation failed');
 return r.payload.result.Ok;
}
const challenge=await exchange('auth.hello',{device:{id:'gate1-audit-observer',kind:'desktop',platform:'node',sdk_version:'audit'},auth_methods:['bearer'],capabilities:{}});
if(challenge.type!=='auth.challenge'||!challenge.payload.server_capabilities.application_profiles.includes('connect-control/1'))throw new Error('profile not advertised');
if((await exchange('auth.response',{method:'bearer',credential:cfg.credential})).type!=='auth.session')throw new Error('auth failed');
fs.writeFileSync(cfg.ready,'ready',{mode:0o600});
let cursor=null;const messages=new Map();const conversations=new Set();
while(messages.size<4){
 const page=await request({op:'inbox',cursor,limit:100,wait_ms:500});
 if(page.kind!=='inbox')throw new Error('wrong outcome');
 for(const event of page.data.events){conversations.add(event.conversation_id);if(event.message)messages.set(event.message.id,event.message);}
 cursor=page.data.cursor;
}
if(conversations.size!==2)throw new Error('expected two independently initiated conversations');
const histories={};
for(const id of conversations){
 const h=await request({op:'history',conversation_id:id,after:null,limit:100});
 if(h.kind!=='history'||h.data.messages.length<2)throw new Error('history incomplete');
 histories[id]=h;
}
fs.writeFileSync(cfg.output,JSON.stringify({messages:messages.size,conversations:conversations.size,histories}),{mode:0o600});
socket.close();clearTimeout(deadline);
console.log('PASS authenticated WS observer: two native conversations and four messages');
