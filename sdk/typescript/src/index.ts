import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import { randomUUID } from 'node:crypto';
export type Operation =
 | {op:'resolve';address:string}
 | {op:'create';request_id:string;participants:string[]}
 | {op:'send';request_id:string;conversation_id:string;body:string}
 | {op:'history';conversation_id:string;after:string|null;limit:number}
 | {op:'inbox';cursor:string|null;limit:number;wait_ms:number}
 | {op:'request_status';request_id:string}
 | {op:'provider_status';conversation_id:string}
 | {op:'close';request_id:string;conversation_id:string}
 | {op:'call_start';request_id:string;conversation_id:string;route_id:string;destination:string}
 | {op:'call_status';conversation_id:string;call_id:string}
 | {op:'call_end';request_id:string;conversation_id:string;call_id:string}
 | {op:'health'};
export interface Outcome {kind:string;data: Record<string, unknown>}
export class ConnectError extends Error {
 constructor(public code:string,message:string) { super(message); }
}
function outcome(response: any): Outcome {
 if(response?.profile!=='connect-control/1') throw new ConnectError('protocol','Unexpected Connect profile');
 if(response.result.Err) throw new ConnectError(response.result.Err.code,response.result.Err.message);
 if(!response.result.Ok) throw new ConnectError('protocol','Missing result');
 return response.result.Ok;
}
abstract class Operations {
 abstract request(operation:Operation):Promise<Outcome>;
 async resolve(address:string){return (await this.request({op:'resolve',address})).data;}
 async create(request_id:string,participants:string[]){return (await this.request({op:'create',request_id,participants})).data;}
 async send(request_id:string,conversation_id:string,body:string){return (await this.request({op:'send',request_id,conversation_id,body})).data;}
 async history(conversation_id:string,after:string|null=null,limit=100){return (await this.request({op:'history',conversation_id,after,limit})).data;}
 async inbox(cursor:string|null=null,limit=100,wait_ms=0){return (await this.request({op:'inbox',cursor,limit,wait_ms})).data;}
 async requestStatus(request_id:string){return (await this.request({op:'request_status',request_id})).data;}
 async providerStatus(conversation_id:string){return (await this.request({op:'provider_status',conversation_id})).data;}
 async closeConversation(request_id:string,conversation_id:string){return (await this.request({op:'close',request_id,conversation_id})).data;}
 async callStart(request_id:string,conversation_id:string,route_id:string,destination:string){return (await this.request({op:'call_start',request_id,conversation_id,route_id,destination})).data;}
 async callStatus(conversation_id:string,call_id:string){return (await this.request({op:'call_status',conversation_id,call_id})).data;}
 async callEnd(request_id:string,conversation_id:string,call_id:string){return (await this.request({op:'call_end',request_id,conversation_id,call_id})).data;}
 async health(){return (await this.request({op:'health'})).data;}
}
/** Rust helper owns TLS/auth/UCTP/QUIC. No automatic mutation retries. */
export class NativeClient extends Operations {
 private child:ChildProcessWithoutNullStreams;
 private pending?:{id:string;resolve:(v:Outcome)=>void;reject:(e:Error)=>void;timer:NodeJS.Timeout};
 private closed=false;
 private chain:Promise<unknown>=Promise.resolve();
 constructor(config:string,executable='connect',private timeoutMs=15000){
  super();this.child=spawn(executable,['stdio','--config',config],{stdio:['pipe','pipe','pipe']});
  this.child.stderr.pipe(process.stderr);
  const lines=createInterface({input:this.child.stdout});
  lines.on('line',line=>{const pending=this.pending;if(!pending)return;this.pending=undefined;clearTimeout(pending.timer);
   try {if(Buffer.byteLength(line)>600000)throw new ConnectError('protocol','Response exceeds bounds');const value=JSON.parse(line);if(value.id!==pending.id)throw new ConnectError('protocol','Exchange ID mismatch');pending.resolve(outcome(value.response));}catch(e){pending.reject(e as Error);}
  });
  this.child.on('error',()=>this.fail());this.child.stdin.on('error',()=>this.fail());this.child.on('exit',()=>this.fail());
 }
 private fail(){this.closed=true;if(this.pending){clearTimeout(this.pending.timer);this.pending.reject(new ConnectError('transport-uncertain','Helper closed; preserve logical request ID'));this.pending=undefined;}}
 request(operation:Operation):Promise<Outcome>{
  const next=this.chain.then(()=>new Promise<Outcome>((resolve,reject)=>{
   if(this.closed){reject(new ConnectError('transport-uncertain','Client closed'));return;}
   const id=randomUUID();const line=JSON.stringify({id,operation})+'\n';
   if(Buffer.byteLength(line)>65537){reject(new ConnectError('invalid-request','Operation exceeds 64 KiB'));return;}
   const timer=setTimeout(()=>{this.fail();this.child.kill();},this.timeoutMs);
   this.pending={id,resolve,reject,timer};this.child.stdin.write(line);
  }));this.chain=next.catch(()=>{});return next;
 }
 async close(){if(this.closed)return;this.closed=true;this.child.stdin.end();await new Promise<void>(resolve=>{const timer=setTimeout(()=>{this.child.kill();resolve();},3000);this.child.once('exit',()=>{clearTimeout(timer);resolve();});});}
}
/** HTTP facade only; it does not advertise native QUIC. */
export class HttpClient extends Operations {
 private url:string;
 constructor(url:string,private credential:string,private timeoutMs=12000){super();const parsed=new URL(url);if(parsed.protocol!=='http:'||!['127.0.0.1','[::1]','localhost'].includes(parsed.hostname)||parsed.username||parsed.password||parsed.search||parsed.hash)throw new Error('Use a loopback HTTP URL');this.url=url.replace(/\/$/,'')+'/v1/operations';}
 async request(operation:Operation):Promise<Outcome>{
  try {const response=await fetch(this.url,{method:'POST',redirect:'error',headers:{'content-type':'application/json','authorization':'Bearer '+this.credential},body:JSON.stringify(operation),signal:AbortSignal.timeout(this.timeoutMs)});return outcome(await response.json());}
  catch(e){if(e instanceof ConnectError)throw e;throw new ConnectError('transport-uncertain','HTTP failed; preserve logical request ID');}
 }
}
