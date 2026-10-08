import { readFileSync } from 'node:fs';
import { NativeClient } from '../../sdk/typescript/dist/index.js';
// Usage: node chat.mjs <config> <presence.json> <saved-request-id> <body> <connect-binary>
const [config,manifest,requestId,body,executable='connect']=process.argv.slice(2);
if(!config||!manifest||!requestId||!body)throw new Error('Provide config, manifest, saved request ID, and body');
const {conversation_id}=JSON.parse(readFileSync(manifest,'utf8'));
const client=new NativeClient(config,executable);
try{
 const accepted=await client.send(requestId,conversation_id,body);
 console.log(JSON.stringify({accepted}));
 for(let i=0;i<60;i++){
  const {deliveries}=await client.providerStatus(conversation_id);
  if(deliveries.some(d=>d.message_id===accepted.message_id&&!['pending','sending'].includes(d.state))){console.log(JSON.stringify({history:await client.history(conversation_id),deliveries}));break;}
  await new Promise(resolve=>setTimeout(resolve,1000));
 }
}finally{await client.close();}
