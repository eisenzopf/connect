// Independent browser peer for the canonical Connect listener fixture.
export function canonical(value) {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return '['+value.map(canonical).join(',')+']';
  return '{'+Object.keys(value).sort().map(key=>JSON.stringify(key)+':'+canonical(value[key])).join(',')+'}';
}
export function unbase64(value) {
  return Uint8Array.from(atob(value.replaceAll('-','+').replaceAll('_','/')), c=>c.charCodeAt(0));
}
export function base64url(value) {return btoa(String.fromCharCode(...new Uint8Array(value))).replaceAll('+','-').replaceAll('/','_').replaceAll('=','');}
export const algorithm=name=>name==='ES256'?{name:'ECDSA',namedCurve:'P-256'}:{name:'Ed25519'};
const signing=name=>name==='ES256'?{name:'ECDSA',hash:'SHA-256'}:{name:'Ed25519'};
const check=(test,label)=>{if(!test)throw new Error(label);};
const bytes=new TextEncoder();
const deadline=(promise,ms,label)=>new Promise((resolve,reject)=>{
  const timer=setTimeout(()=>reject(new Error(label)),ms);
  Promise.resolve(promise).then(value=>{clearTimeout(timer);resolve(value);},error=>{clearTimeout(timer);reject(error);});
});
export async function signFrame(frame,key,keyid,alg) {
  const bare={...frame};delete bare.signature;
  const sig=await crypto.subtle.sign(signing(alg),key,bytes.encode(canonical(bare)));
  return {...bare,signature:{keyid,alg,sig:base64url(sig)}};
}
function envelope(type,payload,scope={}){return {v:2,type,id:'env_'+crypto.randomUUID().replaceAll('-',''),ts:new Date().toISOString(),...scope,payload};}
export class Peer {
  static async open(config) {
    const peer=new Peer();peer.config=config;peer.seen=new Set();peer.buffer=new Uint8Array();peer.verified=0;
    peer.key=await crypto.subtle.importKey('jwk',config.private_jwk,algorithm(config.algorithm),false,['sign']);
    peer.serverKey=await crypto.subtle.importKey('raw',unbase64(config.server_public_raw),algorithm(config.server_algorithm),false,['verify']);
    peer.transport=new WebTransport(config.url,{allowPooling:false,serverCertificateHashes:[{algorithm:'sha-256',value:unbase64(config.certificate_sha256)}]});
    await deadline(peer.transport.ready,8000,'browser WebTransport handshake deadline');
    const stream=await deadline(peer.transport.createBidirectionalStream(),5000,'browser signaling stream deadline');
    peer.writer=stream.writable.getWriter();peer.reader=stream.readable.getReader();
    peer.datagramWriter=peer.transport.datagrams.writable.getWriter();peer.datagramReader=peer.transport.datagrams.readable.getReader();
    const hello=await peer.send(envelope('auth.hello',{device:{...config.device,sdk_version:'independent-chrome-fixture'},auth_methods:['bearer'],application_profiles:['connect/2']}),false);
    const challenge=await peer.read();check(challenge.type==='auth.challenge'&&challenge.in_reply_to===hello,'browser challenge correlation');
    check(challenge.payload.server_capabilities.application_profiles.includes('connect/2'),'browser canonical profile offer');
    const response=envelope('auth.response',{method:'bearer',credential:config.credential},{in_reply_to:challenge.id});
    const id=await peer.send(response,true);const authenticated=await peer.read();
    check(authenticated.type==='auth.session'&&authenticated.in_reply_to===id,'browser authentication correlation');
    check(authenticated.payload.application_profile==='connect/2'&&authenticated.payload.identity_id===config.identity_id,'browser authenticated identity/profile');
    return peer;
  }
  async send(frame,signed=true){
    if(signed)frame=await signFrame(frame,this.key,this.config.keyid,this.config.algorithm);
    const value=bytes.encode(JSON.stringify(frame));check(value.length>0&&value.length<=1048576,'browser envelope write bound');
    const packet=new Uint8Array(4+value.length);new DataView(packet.buffer).setUint32(0,value.length);packet.set(value,4);
    await deadline(this.writer.write(packet),5000,'browser signaling write deadline');return frame.id;
  }
  async read(){
    const frame=await deadline((async()=>{for(;;){
      if(this.buffer.length>=4){const size=new DataView(this.buffer.buffer,this.buffer.byteOffset,this.buffer.byteLength).getUint32(0);check(size>0&&size<=1048576,'browser envelope length');
        if(this.buffer.length>=size+4){const raw=this.buffer.slice(4,size+4);this.buffer=this.buffer.slice(size+4);return JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(raw));}}
      const {value,done}=await this.reader.read();check(!done,'browser unexpected signaling EOF');check(this.buffer.length+value.length<=1114116,'browser envelope buffer bound');
      const next=new Uint8Array(this.buffer.length+value.length);next.set(this.buffer);next.set(value,this.buffer.length);this.buffer=next;
    }})(),5000,'browser canonical read deadline');
    check(frame.v===2&&typeof frame.id==='string'&&!this.seen.has(frame.id)&&this.seen.size<1024,'browser fresh envelope id');
    const sig=frame.signature;check(sig?.keyid===this.config.server_keyid&&sig?.alg===this.config.server_algorithm,'browser enrolled server signature identity');
    const now=Date.now()/1000;check(now>=this.config.server_not_before&&now<this.config.server_expires_at,'browser enrolled server key lifetime');
    check(Math.abs(Date.now()-Date.parse(frame.ts))<60000,'browser reply freshness');
    const bare={...frame};delete bare.signature;
    check(await crypto.subtle.verify(signing(sig.alg),this.serverKey,unbase64(sig.sig),bytes.encode(canonical(bare))),'browser server signature');
    this.seen.add(frame.id);this.verified++;return frame;
  }
  async create(participants){
    const id=await this.send(envelope('conversation.create',{tenant_id:'tnt_default',policy:'persistent',idle_close_secs:null,metadata:null,initial_participants:participants.map(identity_id=>({identity_id,role:'participant'}))},{operation_id:'browser-create'}));
    let cid;for(let count=0;count<128;count++){const frame=await this.read();check(frame.in_reply_to===id,'browser create reply correlation');check(frame.type!=='error','browser conversation create refused');if(frame.type==='conversation.opened')cid=frame.cid;if(frame.type==='ack'){check(typeof cid==='string','browser create durable projection');const details=frame.payload.details,receipt=details?.conversation?.receipt;check(details?.reply_complete===true&&details.conversation.id===cid&&receipt?.operation_id==='browser-create'&&receipt?.conversation_id===cid&&receipt?.state==='completed','browser durable create receipt');return cid;}}
    throw new Error('browser create reply count bound');
  }
  async completed(cid,operation){
    const until=Date.now()+5000;
    for(;;){const id=await this.send(envelope('operation.get',{operation_id:operation},{cid}));let state,done=false;
      for(let count=0;count<128;count++){const frame=await this.read();if(frame.in_reply_to!==id)continue;check(frame.type!=='error','browser current operation refused');if(frame.type==='operation.updated'){check(frame.payload.operation_id===operation,'browser operation target correlation');state=frame.payload.state;}if(frame.type==='ack'){done=true;break;}}
      check(done,'browser operation reply bound');if(state==='completed')return;check(Date.now()<until,'browser native operation completion deadline');await new Promise(r=>setTimeout(r,20));
    }
  }
  async offer(cid,index){
    const scope={cid,sid:'http3-media-session',connid:'browser-connection-'+index,operation_id:'browser-offer-'+index};
    const caps={audio_codecs:[{name:'pcmu',params:{sample_rate:8000,channels:1}}],max_streams_per_connection:8,identity_assurance_offered:'identified'};
    const id=await this.send(envelope('connection.offer',{substrate:'webtransport',capabilities:caps,streams_offered:[{id:'browser-voice-'+index,kind:'audio',direction:'sendrecv',codec_preferences:['g.711-mu']}]},{...scope,expected_revision:0}));
    let local;for(let count=0;count<128;count++){
      const frame=await this.read();check(frame.type!=='error','browser media offer refused');
      if(frame.type==='connection.answer'&&frame.in_reply_to===id){check(frame.payload.substrate==='webtransport','browser actual answer transport');local=frame.payload.substrate_setup.streams[0].stream_local_id;}
      if(frame.type==='connection.offer'){
        check(Number.isInteger(local)&&local>0&&local<=65535,'browser native answer before parameter confirmation');for(const field of ['cid','sid','connid','operation_id'])check(frame[field]===scope[field],'browser parameter resource binding');
        const p=frame.payload;check(p.substrate==='webtransport','browser parameter transport');
        const streams_answered=p.substrate_setup.streams.map(s=>({id:s.strm_id,kind:s.kind,direction:s.direction==='sendonly'?'recvonly':s.direction==='recvonly'?'sendonly':s.direction,codec:s.codec}));
        await this.send(envelope('connection.answer',{by_participant:p.by_participant,substrate:'webtransport',capabilities:caps,streams_answered,substrate_setup:p.substrate_setup},{...scope,in_reply_to:frame.id}));
        await this.completed(cid,scope.operation_id);return local;
      }
    }throw new Error('browser offer reply count bound');
  }
  async subscribe(cid,index,source,handle){
    const scope={cid,sid:'http3-media-session',connid:'browser-connection-'+index,operation_id:'browser-subscribe-'+index};
    const caps={audio_codecs:[{name:'pcmu',params:{sample_rate:8000,channels:1}}],max_streams_per_connection:8,identity_assurance_offered:'identified'};
    const id=await this.send(envelope('stream.subscribe',{subscriptions:[{from_participant:source,kinds:['audio']}]},{...scope,expected_revision:1}));
    let completed=false;
    for(let count=0;count<128;count++){
      const frame=await this.read();check(frame.type!=='error','browser signed subscription refused');
      if(frame.type==='connection.offer'){
        for(const field of ['cid','sid','connid','operation_id'])check(frame[field]===scope[field],'browser subscription parameter binding');
        const p=frame.payload;check(p.substrate==='webtransport'&&p.substrate_setup.offer_id===id,'browser subscription transport and offer binding');
        const audio=p.substrate_setup.streams.find(s=>s.kind==='audio');check(Number.isInteger(audio?.stream_local_id)&&audio.stream_local_id>0&&audio.stream_local_id<=65535,'browser subscription negotiated handle');handle=audio.stream_local_id;
        const streams_answered=p.substrate_setup.streams.map(s=>({id:s.strm_id,kind:s.kind,direction:s.direction==='sendonly'?'recvonly':s.direction==='recvonly'?'sendonly':s.direction,codec:s.codec}));
        await this.send(envelope('connection.answer',{by_participant:p.by_participant,substrate:'webtransport',capabilities:caps,streams_answered,substrate_setup:p.substrate_setup},{...scope,in_reply_to:frame.id}));
      }else if(frame.in_reply_to===id){
        if(frame.type==='operation.updated'){check(frame.payload.operation_id===scope.operation_id&&frame.payload.state==='completed','browser subscription completed operation');completed=true;}
        if(frame.type==='ack'){check(completed,'browser subscription completion before ACK');return handle;}
      }
    }throw new Error('browser subscription reply count bound');
  }
  async close(){this.transport.close({closeCode:0,reason:'owned browser fixture complete'});await deadline(this.transport.closed.catch(()=>{}),3000,'browser transport close deadline');}
}
export function datagram(handle,seq,period=16){
  const output=new Uint8Array(180);const view=new DataView(output.buffer);output[0]=1;view.setUint16(2,handle);view.setUint32(4,seq);output[8]=0x80;output[9]=0;view.setUint16(10,seq&65535);view.setUint32(12,seq*160);view.setUint32(16,0x47360049);
  for(let i=0;i<160;i++)output[20+i]=i%period<period/2?0x80:0x00;return output;
}
export function decodedPacket(packet,expected,period=16){
  check(packet.length>=20&&packet.length<=16404,'browser datagram bound');const v=new DataView(packet.buffer,packet.byteOffset,packet.byteLength);
  check(packet[0]===1&&v.getUint16(2)===expected&&packet[8]>>6===2&&(packet[9]&127)===0,'browser negotiated RTP binding');
  let begin=20+4*(packet[8]&15);if(packet[8]&16){check(begin+4<=packet.length,'browser RTP extension bound');begin+=4+v.getUint16(begin+2)*4;}
  const end=packet.length-(packet[8]&32?packet.at(-1):0);check(begin<=end,'browser RTP payload bound');
  let positive=false,negative=false,energetic=0,changes=0,previous;
  for(let i=begin;i<end;i++){const value=(~packet[i])&255;const size=(((value&15)*8+132)<<((value>>4)&7))-132;const pcm=value&128?-size:size;positive||=pcm>1000;negative||=pcm< -1000;if(Math.abs(pcm)>500){const sign=pcm>0;if(previous!==undefined&&previous!==sign)changes++;previous=sign;energetic++;}}
  const rate=changes/energetic,expectedRate=2/period;
  return positive&&negative&&energetic>=(end-begin)*0.85&&rate>=expectedRate*0.8&&rate<=expectedRate*1.2;
}
async function duplex(from,to,source,target,start,period){
  const sending=(async()=>{for(let seq=start;seq<start+80;seq++){await from.datagramWriter.write(datagram(source,seq,period));await new Promise(r=>setTimeout(r,20));}})();
  const receiving=deadline((async()=>{for(let n=0;n<256;n++){const result=await to.datagramReader.read();check(!result.done,'browser datagram EOF');if(decodedPacket(result.value,target,period))return true;}throw new Error('browser decoded receive frame bound');})(),5000,'browser actual decoded duplex deadline');
  const [,decoded]=await Promise.all([sending,receiving]);return decoded;
}
export async function run(config){
  check(globalThis.isSecureContext,'browser secure context');const peers=[];let first;
  try{
    first=await Peer.open(config.actors[0]);peers.push(first);const created=await first.create([config.actors[1].identity_id]);await first.close();peers.pop();
    first=await Peer.open(config.actors[0]);peers.push(first);check(await first.create([config.actors[1].identity_id])===created,'browser durable reconnect receipt');
    const second=await Peer.open(config.actors[1]);peers.push(second);let a=await first.offer(config.media_cid,0);let b=await second.offer(config.media_cid,1);
    a=await first.subscribe(config.media_cid,0,config.participants[1],a);b=await second.subscribe(config.media_cid,1,config.participants[0],b);
    const forward=await duplex(first,second,a,b,1,16);const reverse=await duplex(second,first,b,a,1001,32);
    return {passed:true,created_cid:created,decoded_forward:forward,decoded_reverse:reverse,verified_server_frames:first.verified+second.verified,algorithm:config.actors[0].algorithm};
  }finally{await Promise.all(peers.map(p=>p.close().catch(()=>{})));}
}
