// Public Node/browser HTTP and native clients with only the intended task bearer.
import assert from 'node:assert/strict';
import {NativeClient, HttpClient, ConnectError} from '../typescript/dist/index.js';
import {BrowserHttpClient} from '../typescript/dist/browser.js';
const [config,url,executable,mode]=process.argv.slice(2);
let input=''; for await (const chunk of process.stdin) input+=chunk;
const {token,task,other,cid}=JSON.parse(input);
const open=()=>mode==='native'?new NativeClient(config,executable):mode==='browser'?new BrowserHttpClient(url,token):new HttpClient(url,token);
async function denied(fn,code) { await assert.rejects(fn,e=>e instanceof ConnectError && e.code===code); }
const command={action:'invoke',task_id:task,request_id:'typescript-'+mode,step_id:'read',operation:{op:'send',request_id:'typescript-'+mode,conversation_id:cid,message:{to:'all',content_type:'text/plain',body:'Task-scoped SDK message',body_encoding:'utf8',label:'connect-messages',attachments:[],in_reply_to_msg:null,msg_id:null}},reply_to:null,wait_for_reply:false};
let client=open(), outcome;
try {
    await denied(()=>client.task({action:'get',task_id:task}),'unsupported-profile');
    await denied(()=>client.negotiateTask(other),'forbidden');
    await denied(()=>client.task({action:"get",task_id:task}),"unsupported-profile");
    await denied(()=>client.negotiateTask(""),"invalid-request");
    assert.equal((await client.negotiateTask(task)).id,task);
    await denied(()=>client.task({action:'get',task_id:other}),'forbidden');
    await denied(()=>client.send('primitive-'+mode,cid,'not allowed'),'forbidden');
    outcome=await client.task(command);
    assert.equal(outcome.kind,'tool'); assert.equal(outcome.data.status,'completed');
    assert.deepEqual(await client.task(command),outcome);
    const changed=structuredClone(command); changed.operation.body="Changed intent";
    await denied(()=>client.task(changed),"request-conflict");
} finally { if(mode==='native') await client.close(); }
client=open();
try { await client.negotiateTask(task); assert.deepEqual(await client.task(command),outcome); }
finally { if(mode==='native') await client.close(); }
process.stdout.write(JSON.stringify({status:'ok',mode})+'\n');
