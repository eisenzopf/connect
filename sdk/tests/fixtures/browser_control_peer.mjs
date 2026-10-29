import assert from 'node:assert/strict';
import {canonicalWireJson,parseWireJson} from '../../typescript/dist/uctp_json.js';
import {signEnvelope,verifyEnvelope} from '../../typescript/dist/uctp_signing.js';
import {envelope,encodeEnvelope,EnvelopeReader} from '../../typescript/dist/uctp_wire.js';
import {ConnectError} from '../../typescript/dist/errors.js';
const bytes=text=>new TextEncoder().encode(text);
// Signed control boundary fixture only: no TLS, network, media or provider proof.
export async function fixture(run,{algorithm='EdDSA',lostCreateAck=false,tamper=false,raw=false,handler,wtMode,wsMode}={}) {
  const keyOptions=algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
  const clientPair=await crypto.subtle.generateKey(keyOptions,false,['sign','verify']);
  const serverPair=await crypto.subtle.generateKey(keyOptions,false,['sign','verify']);
  const signing={key:clientPair.privateKey,keyId:'wss-client-key',algorithm};
  const clientTrust={key:clientPair.publicKey,keyId:signing.keyId,algorithm,notBefore:Date.now()/1000-30,expiresAt:Date.now()/1000+300};
  const serverSigning={key:serverPair.privateKey,keyId:'wss-server-key',algorithm};
  const server={key:serverPair.publicKey,keyId:serverSigning.keyId,algorithm,notBefore:Date.now()/1000-30,expiresAt:Date.now()/1000+300};
  const sent=[],strings=[],tasks=[],receipts=new Map(),sockets=[],transports=[],wtCalls=[];
  let socket,activeCredential='fixture-only';
  const originals=['WebSocket','WebTransport','isSecureContext'].map(key=>[key,Object.getOwnPropertyDescriptor(globalThis,key)]);
  const accept=(peer,frame)=>{
    sent.push(frame);assert.ok(tasks.length<2048,'bounded fixture requests');
    const task=(async()=>{
      if(frame.type!=='auth.hello')await verifyEnvelope(frame,clientTrust);
      if(frame.type==='auth.hello')return peer.emit('auth.challenge',{accepted_methods:['bearer'],server_capabilities:{application_profiles:['connect/2']}},frame.id);
      if(frame.type==='auth.response'){
        if(peer.mode==='auth-timeout')return;
        if(peer.mode==='auth-refused'||frame.payload.credential!==activeCredential)return peer.emit('error',{code:401,category:'auth',reason:'authentication_failed'},frame.id);
        peer.credential=frame.payload.credential;
        return peer.emit('auth.session',{identity_id:'wss_identity',participant_id:'wss_participant',session_token:'fixture-session',expires_at:new Date(Date.now()+60000).toISOString(),application_profile:'connect/2'},frame.id);
      }
      if(peer.credential!==activeCredential)return peer.emit('error',{code:401,category:'auth',reason:'authentication_failed'},frame.id,frame);
      if(handler&&await handler({peer,frame,receipts}))return;
      if(frame.type==='conversation.create'){
        const receipt={operation_id:frame.operation_id,conversation_id:'wss_conversation',state:'completed'};receipts.set(frame.operation_id,receipt);
        if(!lostCreateAck)return peer.emit('ack',{details:{reply_complete:true,conversation:{id:'wss_conversation',receipt}}},frame.id,frame);
      }else{
        if(frame.type==='operation.get'){
          const value=receipts.get(frame.payload.operation_id);
          if(!value)return peer.emit('error',{code:404,category:'state',reason:'resource_not_found'},frame.id,frame);
          await peer.emit('operation.updated',{record_id:'opr_'+frame.payload.operation_id,command_type:'create',revision:1,...value},frame.id,frame);
        }
        await peer.emit('ack',{},frame.id,frame);
      }
    })();tasks.push(task);void task.catch(()=>{});return task;
  };
  const signed=async(type,payload,in_reply_to,scope={},extensions={})=>{
    const frame=await signEnvelope({...envelope(type,payload,{...scope,in_reply_to}),...extensions},serverSigning);
    if(tamper&&type==='auth.session')frame.payload.identity_id='forged';return frame;
  };
  class Socket extends EventTarget {
    static CONNECTING=0;static OPEN=1;static CLOSED=3;
    readyState=0;bufferedAmount=0;closeCount=0;path='wss-webrtc';mode=wsMode;
    constructor(){super();socket=this;sockets.push(this);queueMicrotask(()=>{if(this.readyState!==0)return;if(this.mode==='transport-refused'){this.dispatchEvent(new Event('error'));return;}if(this.mode==='timeout')return;this.readyState=1;this.dispatchEvent(new Event('open'));});}
    text(value){if(this.readyState!==3)this.dispatchEvent(new MessageEvent('message',{data:value}));}
    async emit(type,payload,in_reply_to,scope,extensions){const frame=await signed(type,payload,in_reply_to,scope,extensions);this.text(canonicalWireJson(frame));return frame;}
    send(text){assert.equal(typeof text,'string');strings.push(text);if(!raw)accept(this,parseWireJson(bytes(text)));}
    close(){if(this.readyState===3)return;this.readyState=3;this.closeCount++;this.dispatchEvent(new Event('close'));}
  }
  const installWT=mode=>{
    if(mode===undefined){Object.defineProperty(globalThis,'WebTransport',{configurable:true,value:undefined});return;}
    class Transport {
      path='webtransport';didClose=false;mode=mode;datagrams={readable:new ReadableStream(),writable:new WritableStream()};
      constructor(url,config){
        wtCalls.push({url,config});
        if(mode==='security-refused')throw new DOMException('fixture TLS policy refusal','SecurityError');
        this.closed=new Promise(resolve=>{this.finish=resolve;});
        this.output=new ReadableStream({start:c=>{this.controller=c;}});
        this.ready=mode==='timeout'?new Promise(()=>{}):mode==='transport-refused'?Promise.reject(new ConnectError('transport','fixture physical failure')):Promise.resolve();
        transports.push(this);
      }
      async emit(type,payload,in_reply_to,scope,extensions){const frame=await signed(type,payload,in_reply_to,scope,extensions);if(!this.didClose)this.controller.enqueue(encodeEnvelope(frame));return frame;}
      async createBidirectionalStream(){const decoder=new EnvelopeReader();return{readable:this.output,writable:new WritableStream({write:async chunk=>{for(const frame of decoder.push(chunk))await accept(this,frame);}})};}
      close(){if(this.didClose)return;this.didClose=true;this.finish();try{this.controller.close();}catch{}}
    }
    Object.defineProperty(globalThis,'WebTransport',{configurable:true,value:Transport});
  };
  Object.defineProperty(globalThis,'WebSocket',{configurable:true,value:Socket});installWT(wtMode);
  Object.defineProperty(globalThis,'isSecureContext',{configurable:true,value:true});
  const options={transport:'websocket',url:'wss://fixture.invalid/connect',credential:'fixture-only',identityId:'wss_identity',device:{id:'wss_device',kind:'browser',platform:'fixture'},signing,server,timeoutMs:500};
  try{await run({options,sent,strings,socket:()=>socket,receipts,sockets,transports,wtCalls,installWT,setCredential:value=>{activeCredential=value;}});await Promise.all(tasks);}
  finally{for(const peer of sockets)peer.close();for(const peer of transports)peer.close();for(const[key,descriptor]of originals)if(descriptor)Object.defineProperty(globalThis,key,descriptor);else delete globalThis[key];}
}
