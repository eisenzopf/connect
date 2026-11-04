// Actual signed production listener, WebTransport and Chromium VP8. No fixture
// signaling or codec substitutions. Observers preserve every original datagram.
import {BrowserClient} from '/sdk/browser_client.js';
import {BrowserMediaPeer} from '/sdk/browser_media.js';
import {decodeNativeRtpDatagram,decodeNativeVideoEpoch,decodeNativeVideoFeedback} from '/sdk/browser_video_wire.js';
import {rtpTimestampFromMicroseconds} from '/sdk/browser_rtcp.js';
import {unbase64url} from '/sdk/uctp_signing.js';
const check=(ok,message)=>{if(!ok)throw Error(message);};
const wait=ms=>new Promise(r=>setTimeout(r,ms));
async function options(a){
 const signing=a.algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
 const server=a.server_algorithm==='EdDSA'?{name:'Ed25519'}:{name:'ECDSA',namedCurve:'P-256'};
 return {url:a.url,credential:a.credential,identityId:a.identity_id,device:a.device,
 signing:{key:await crypto.subtle.importKey('jwk',a.private_jwk,signing,false,['sign']),keyId:a.keyid,algorithm:a.algorithm},
 server:{key:await crypto.subtle.importKey('raw',unbase64url(a.server_public_raw),server,false,['verify']),keyId:a.server_keyid,algorithm:a.server_algorithm,notBefore:a.server_not_before,expiresAt:a.server_expires_at},
 serverCertificateHashes:[{algorithm:'sha-256',value:unbase64url(a.certificate_sha256)}],timeoutMs:5000};
}
function observed(client,receive,send){
 const raw=client.datagrams,reader=raw.readable.getReader(),writer=raw.writable.getWriter();let readReleased=false,writeReleased=false;
 const releaseRead=()=>{if(!readReleased){readReleased=true;reader.releaseLock();}},releaseWrite=()=>{if(!writeReleased){writeReleased=true;writer.releaseLock();}};
 const readable=new ReadableStream({async pull(controller){try{const {value,done}=await reader.read();if(done){releaseRead();controller.close();}else{receive(value);controller.enqueue(value);}}catch(error){releaseRead();controller.error(error);}},async cancel(reason){try{await reader.cancel(reason);}finally{releaseRead();}}});
 const writable=new WritableStream({async write(bytes){send(bytes);await writer.write(bytes);},async abort(reason){try{await writer.abort(reason);}finally{releaseWrite();}},async close(){try{await writer.close();}finally{releaseWrite();}}});
 const proxy={readable,writable};for(const key of ['maxDatagramSize','incomingMaxAge','outgoingMaxAge','incomingHighWaterMark','outgoingHighWaterMark'])Object.defineProperty(proxy,key,{get:()=>raw[key],...(key==='maxDatagramSize'?{}:{set:v=>raw[key]=v})});
 Object.defineProperty(client,'datagrams',{value:proxy});
 return ()=>readReleased&&writeReleased&&!readable.locked&&!writable.locked;
}
export async function runRecovery(input){
 const clients=[],peers=[],unlocked=[],stops=[],errors=[],sent=[],received=[],hints=[],notices=[],decoded=[],control=[];
 const result={passed:false,same_ssrc:false,new_epoch:false,fresh_frame:false,shutdown_joined:false,feedback_packets:0};
 let phase='connect';globalThis.videoRecoveryPhase=phase;
 const setPhase=v=>globalThis.videoRecoveryPhase=phase=v;
 const until=async(fn,label,ms=5000)=>{const end=performance.now()+ms;while(!fn()){check(!errors.length,'owned media error: '+errors.join(', '));check(performance.now()<end,label+' deadline; phase='+phase);await wait(5);}};
 try{
  for(const actor of input.actors){const client=await BrowserClient.open(await options(actor));clients.push(client);
   const actorIndex=clients.length-1,record=row=>{check(control.length<512,'control trace bound');control.push({actor:actorIndex,at:performance.now(),...row});};
   stops.push(client.onClose(e=>record({closed:e?.message,code:e?.code})));
   const begin=client.begin.bind(client);client.begin=(type,payload,scope,options)=>{record({request:type,operation:scope?.operation_id,revision:scope?.expected_revision});const original=begin(type,payload,scope,options);void original.response.then(frames=>record({response:type,frames:frames.map(f=>({type:f.type,code:f.payload.code,state:f.payload.state}))}),e=>record({refused:type,code:e?.code,message:e?.message}));return original;};
   stops.push(client.onEvent(frame=>record({event:frame.type,operation:frame.operation_id,revision:frame.payload.substrate_setup?.revision})));
   stops.push(client.onEvent(frame=>{if(frame.type==='stream.epoch'){check(notices.length<64,'notice bound');notices.push({cid:frame.cid,sid:frame.sid,connid:frame.connid,...frame.payload});}}));}
  unlocked.push(observed(clients[0],bytes=>{if(bytes.length===48&&bytes[9]===206){check(hints.length<32,'hint bound');hints.push({...decodeNativeVideoFeedback(bytes),at:performance.now()});}},bytes=>{
   if(bytes[1]!==0||(bytes[9]>=192&&bytes[9]<=223))return;const p=decodeNativeRtpDatagram(bytes);
   if(p.payload[0]&0x10){check(sent.length<16,'source frame bound');sent.push({timestamp:p.timestamp,ssrc:p.ssrc,key:(p.payload[1]&1)===0,sequence:p.sequence,at:performance.now()});}
  }));
  unlocked.push(observed(clients[1],bytes=>{if(bytes[1]===1){const p=decodeNativeVideoEpoch(bytes);check(received.length<256,'receive packet bound');received.push({epoch:p.epoch,ssrc:p.packet.ssrc,timestamp:p.packet.timestamp,sequence:p.packet.datagramSequence});}},()=>{}));
  const pixels=new OffscreenCanvas(640,360),pixelContext=pixels.getContext('2d',{willReadFrequently:true});
  peers.push(new BrowserMediaPeer(clients[0],{error:e=>errors.push(e.message)}));
  peers.push(new BrowserMediaPeer(clients[1],{error:e=>errors.push(e.message),video:(binding,frame)=>{
   check(decoded.length<16,'decoded frame bound');pixelContext.drawImage(frame,0,0);const rgba=pixelContext.getImageData(8,8,16,16).data;let red=0;for(let i=0;i<rgba.length;i+=4)red+=rgba[i];decoded.push({stream:binding.streamId,red:red/256,timestamp:frame.timestamp,at:performance.now()});
  }}));
  setPhase('offers');
  const scope={conversationId:input.cid,sessionId:input.sid,nativeVideoFeedback:true};
  const source=await peers[0].offerConnection({...scope,connectionId:'browser-feedback-source',operationId:'browser-feedback-source-offer',video:'camera'},{timeoutMs:5000});
  const target=await peers[1].offerConnection({...scope,connectionId:'browser-feedback-target',operationId:'browser-feedback-target-offer',video:'monitor'},{timeoutMs:5000});
  await target.subscribe('browser-feedback-select',input.source_participant,['video'],{timeoutMs:5000});
  check(target.streams.some(s=>s.streamId==='camera'),'actual selected receiver resource');
  const canvas=new OffscreenCanvas(640,360),ctx=canvas.getContext('2d');const origin=performance.now();
  const capture=red=>{ctx.fillStyle='rgb(30,60,120)';ctx.fillRect(0,0,640,360);ctx.fillStyle=`rgb(${red},40,60)`;ctx.fillRect(0,0,48,48);const timestamp=Math.round((performance.now()-origin)*1000);check(source.encodeVideo('camera',new VideoFrame(canvas,{timestamp})),'fresh source frame accepted');return {timestamp,rtp:rtpTimestampFromMicroseconds(timestamp,90000)};};
  setPhase('initial-key');const first=capture(40);await until(()=>decoded.some(f=>f.stream==='camera'&&Math.abs(f.red-40)<15),'initial decoded marker');
  const firstPacket=received.find(p=>p.timestamp===first.rtp);check(firstPacket,'initial original RTP timestamp observed');
  // Let any initial bootstrap hint expire; do not encode a stale source frame.
  await wait(210);
  setPhase('unselect');await peers[1].beginControl(target.id,'browser-feedback-unselect','stream.unsubscribe',{streams:['camera']},target.revision,{timeoutMs:5000}).ready;
  setPhase('reselect');await target.subscribe('browser-feedback-reselect',input.source_participant,['video'],{timeoutMs:5000});
  const before=hints.length;setPhase('new-history-delta');const delta=capture(110);await until(()=>sent.some(f=>f.timestamp===delta.rtp),'new source frame');
  const sentDelta=sent.find(f=>f.timestamp===delta.rtp);check(!sentDelta.key,'new history must encounter an actual interframe before recovery');
  await until(()=>received.some(p=>p.timestamp===delta.rtp),'new history packet');
  const changed=received.find(p=>p.timestamp===delta.rtp);check(changed.epoch!==firstPacket.epoch,'same source must have new receiver epoch');check(changed.ssrc===firstPacket.ssrc,'same source SSRC retained');
  await until(()=>hints.length>before,'actual receiver feedback reaches source');
  check(!decoded.some(f=>Math.abs(f.red-110)<15),'interframe cannot initialize the new decoder history');
  setPhase('fresh-feedback-key');const fresh=capture(210);await until(()=>decoded.some(f=>f.stream==='camera'&&Math.abs(f.red-210)<15),'fresh original decoded marker');
  const sentFresh=sent.find(f=>f.timestamp===fresh.rtp);check(sentFresh?.key,'feedback forces fresh original keyframe');
  check(received.some(p=>p.timestamp===fresh.rtp&&p.epoch===changed.epoch),'fresh key stays in current epoch at original RTP timestamp');
  check(notices.some(n=>n.direction==='receiver'&&n.epoch===changed.epoch&&n.connid===target.id&&n.strm_id==='camera'),'packet history joins actual signed receiver notice');
  const hint=hints.at(-1);check(notices.some(n=>n.direction==='publisher'&&n.epoch===hint.epoch&&n.ssrc===hint.mediaSsrc&&n.connid===source.id),'feedback joins actual signed publisher notice');
  Object.assign(result,{passed:true,same_ssrc:true,new_epoch:true,fresh_frame:true,feedback_packets:hints.length,source_frames:sent,decoded_frames:decoded,original_timestamps:{first,delta,fresh},receiver_epoch_change:[firstPacket.epoch,changed.epoch],route_diagnostics:peers.map(p=>p.routeDiagnostics),scope:'One local EdDSA Chromium/WebTransport recovery, actual production listener and WebCodecs; not full frozen workload/network/transport cell.'});
 }catch(error){throw Error('phase='+phase+'; '+error.message+'; observed='+JSON.stringify({sent,received:received.slice(-4),hints,decoded,errors,control}));}
 finally{setPhase('shutdown');for(const stop of stops)stop();for(const peer of peers)await peer.shutdown();for(const client of clients)client.close();check(unlocked.every(f=>f()),'observer and native owner stream locks joined');result.shutdown_joined=true;}
 return result;
}
