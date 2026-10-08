// Public TypeScript SDK used from Node. Persist the request file before dialing.
import {NativeClient} from '../../sdk/typescript/dist/index.js';
import fs from 'node:fs';
import {randomUUID} from 'node:crypto';
import path from 'node:path';
const [action, config, statePath, conversation, route, destination] = process.argv.slice(2);
if(!['start','status','end'].includes(action)||!config||!statePath)throw new Error('Usage: voice.mjs start|status|end CONFIG PRIVATE_STATE [CONVERSATION ROUTE DESTINATION]');
function save(value,newFile=false){const target=newFile?statePath:statePath+'.pending';const fd=fs.openSync(target,'wx',0o600);try{fs.writeFileSync(fd,JSON.stringify(value,null,2));fs.fsyncSync(fd);}finally{fs.closeSync(fd);}if(!newFile)fs.renameSync(target,statePath);const dir=fs.openSync(path.dirname(statePath),'r');try{fs.fsyncSync(dir);}finally{fs.closeSync(dir);}}
const identity=fs.readFileSync(config,'utf8').match(/^identity_id\s*=\s*"([^"]+)"/m)?.[1];if(!identity)throw new Error('Missing client identity');
let state;
if(fs.existsSync(statePath)){state=JSON.parse(fs.readFileSync(statePath,'utf8'));if(state.identity!==identity)throw new Error('Receipt belongs to another identity');for(const [key,value] of Object.entries({conversation,route,destination})){if(value&&state[key]!==value)throw new Error('Existing request inputs cannot change');}}
else{if(action!=='start'||!conversation||!route||!destination)throw new Error('First start requires conversation, route and destination');state={identity,conversation,route,destination,startId:'voice-'+randomUUID(),endId:'end-'+randomUUID(),callId:null};save(state,true);}
const client=new NativeClient(config,process.env.CONNECT_EXECUTABLE||'connect');
try{let result;if(action==='start'||!state.callId){result=await client.callStart(state.startId,state.conversation,state.route,state.destination);state.callId=result.id;save(state);}if(action==='status')result=await client.callStatus(state.conversation,state.callId);else if(action==='end')result=await client.callEnd(state.endId,state.conversation,state.callId);console.log(JSON.stringify({callId:result.id,state:result.state,providerId:result.provider_id,endedReason:result.ended_reason}));}
finally{await client.close();}
