import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createServer} from 'node:https';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
const root=fileURLToPath(new URL('../..',import.meta.url)),consumer=fileURLToPath(new URL('./fixtures/node_https_consumer.mjs',import.meta.url));
async function fixture(action,run) {
 const base=join(root,'.local/gate-6/node-https-unit');await mkdir(base,{recursive:true,mode:0o700});const dir=await mkdtemp(join(base,'tls-')),key=join(dir,'key.pem'),cert=join(dir,'cert.pem'),config=join(dir,'openssl.cnf');
 await writeFile(config,'[req]\ndistinguished_name=dn\nx509_extensions=tls\nprompt=no\n[dn]\nCN=Connect synthetic local HTTPS\n[tls]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyCertSign,cRLSign\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1\n',{mode:0o600});
 await promisify(execFile)('openssl',['genpkey','-algorithm','EC','-pkeyopt','ec_paramgen_curve:prime256v1','-out',key],{timeout:3000});
 await promisify(execFile)('openssl',['req','-new','-x509','-sha256','-days','1','-key',key,'-out',cert,'-config',config],{timeout:3000});
 const calls=[],server=createServer({key:await readFile(key),cert:await readFile(cert)},async(request,response)=>{
  let raw='';for await(const chunk of request)raw+=chunk;const value=JSON.parse(raw);calls.push({url:request.url,value,authorization:request.headers.authorization});await action(value,response,request);
 });
 server.on('tlsClientError',()=>{});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const endpoint=`https://127.0.0.1:${server.address().port}`;
 const child=async(spec,trust=true)=>{
  const environment={...process.env,CONNECT_HTTPS_FIXTURE:JSON.stringify({endpoint,...spec})};delete environment.NODE_TLS_REJECT_UNAUTHORIZED;delete environment.NODE_USE_SYSTEM_CA;delete environment.NODE_USE_ENV_PROXY;
  if(trust)environment.NODE_EXTRA_CA_CERTS=cert;else delete environment.NODE_EXTRA_CA_CERTS;
  const proc=spawn(process.execPath,[consumer],{env:environment,stdio:['ignore','pipe','pipe']});let out='',diagnostics='';proc.stdout.on('data',v=>out+=v);proc.stderr.on('data',v=>diagnostics+=v);const timer=setTimeout(()=>proc.kill('SIGKILL'),5000);
  const code=await new Promise((resolve,reject)=>{proc.once('error',reject);proc.once('close',resolve);});clearTimeout(timer);await writeFile(join(dir,'child-stderr.log'),diagnostics,{mode:0o600});assert.equal(code,0,'inspect private TLS fixture diagnostics');const result=JSON.parse(out);assert.equal(result.passed,true);assert.throws(()=>process.kill(proc.pid,0),e=>e.code==='ESRCH');return result;
 };
 try{await run(child,calls,endpoint);}finally{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));await rm(dir,{recursive:true,force:true});}
}
function reply(request,type='ack',payload={}) {return {v:2,id:'reply',ts:new Date().toISOString(),type,payload,in_reply_to:request.id,...Object.fromEntries(['cid','sid','connid','operation_id'].filter(k=>request[k]!==undefined).map(k=>[k,request[k]]))};}
const send=(response,value,status=200)=>{response.writeHead(status,{'content-type':'application/json'});response.end(JSON.stringify(value));};
test('explicit HTTPS uses verified TLS and the existing canonical/application routes with original intent and authority status',async()=>{
 await fixture((request,response,http)=>{
  assert.equal(http.headers.authorization,'Bearer synthetic-https-credential');
  if(http.url==='/v1/operations')return send(response,{profile:'connect-control/1',result:{Err:{code:'forbidden',message:'synthetic refusal'}}},403);
  assert.equal(http.url,'/v2/commands');if(request.type==='message.send'){assert.equal(request.payload.text,'original');assert.equal(request.operation_id,'original-https');}
  const frames=request.type==='operation.get'?[reply(request,'operation.updated',{record_id:'record-original'}),{...reply(request),id:'terminal'}]:[reply(request)];send(response,{profile:'connect/2',request_id:request.id,envelopes:frames});
 },async(child,calls)=>{await child({mode:'success'});assert.equal(calls.length,3);assert.equal(calls[0].value.operation_id,'original-https');assert.equal(calls[1].value.payload.operation_id,'original-https');});
});
test('explicit HTTPS rejects untrusted certificates and hostname mismatch before any HTTP credential or mutation reaches the server',async()=>{
 await fixture(()=>assert.fail('unverified TLS must not admit HTTP'),async(child,calls,endpoint)=>{
  await child({mode:'untrusted',kind:'transport-uncertain',certainty:'uncertain'},false);
  await child({mode:'hostname',endpoint:endpoint.replace('127.0.0.1','localhost'),kind:'transport-uncertain',certainty:'uncertain'});
  assert.equal(calls.length,0);
 });
});
test('explicit HTTPS refuses redirects and current authentication without resubmitting the logical mutation',async()=>{
 for(const mode of ['redirect','refused'])await fixture((request,response)=>{
  if(mode==='redirect'){response.writeHead(307,{location:'https://127.0.0.1:1/private-target'});response.end();}
  else send(response,{profile:'connect/2',problem:{code:403,category:'authorization',reason:'current-auth-refused'}},403);
 },async(child,calls)=>{await child({mode,kind:mode==='redirect'?'protocol':'authentication',certainty:mode==='redirect'?'uncertain':'refused'});assert.equal(calls.length,1);assert.equal(calls[0].value.operation_id,'original-https');});
});
test('explicit HTTPS whole-body deadlines and cancellation retain uncertainty and join requests without retry',async()=>{
 for(const mode of ['deadline','cancel'])await fixture((request,response)=>{response.writeHead(200,{'content-type':'application/json'});response.write('{');},async(child,calls)=>{
  await child({mode,kind:mode==='deadline'?'deadline':'cancelled',certainty:'uncertain'});assert.equal(calls.length,1);assert.equal(calls[0].value.operation_id,'original-https');
 });
});
