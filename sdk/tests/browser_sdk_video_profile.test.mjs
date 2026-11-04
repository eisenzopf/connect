import test from 'node:test';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
import {fixture} from './fixtures/browser_control_peer.mjs';
const base=process.env.CONNECT_GATE6_TS_DIST?pathToFileURL(path.resolve(process.env.CONNECT_GATE6_TS_DIST)+'/').href:new URL('../typescript/dist/',import.meta.url).href;
const {BrowserSdkClient,sdkId}=await import(base+'browser.js');
const {BrowserMediaPeer}=await import(base+'browser_media.js');
const one=first=>({first,rest:[]});
test('public browser SDK freezes explicit native feedback intent before asynchronous enrollment and preserves default off',async()=>{
 const original=BrowserMediaPeer.prototype.beginOfferConnection;
 const captured=[];
 BrowserMediaPeer.prototype.beginOfferConnection=function(input){captured.push(input);return{accepted:Promise.resolve(),ready:Promise.resolve({id:input.connectionId}),submissionAttempted:false};};
 try{
  for(const enabled of [undefined,true])await fixture(async f=>{
   let release;const gate=new Promise(resolve=>release=resolve);
   const options={tenantId:'tnt_default',identityId:f.options.identityId,deviceId:f.options.device.id,endpoints:{webtransport:'https://fixture.invalid/connect'},preferredTransport:'webtransport',timeoutMs:1000,resolveEnrollment:async()=>{await gate;return f.options;},...(enabled?{nativeVideoFeedback:true}:{})};
   const connecting=BrowserSdkClient.connect(options);options.nativeVideoFeedback=!enabled;release();const client=await connecting;
   try{
    f.transports.at(-1).datagrams.maxDatagramSize=1200;
    const session=client.conversations.byId(sdkId('room')).sessions.byId(sdkId('session'));
    await session.connections.offer({connectionId:sdkId('connection'),streams:one({id:sdkId('data'),kind:'data',direction:'sendrecv',codecs:one('binary')})},{operationId:sdkId('offer'),timeoutMs:500});
    assert.equal(captured.at(-1).nativeVideoFeedback,enabled===true);
    assert.equal(f.sent.filter(v=>v.type==='connection.offer').length,0,'this checkpoint captures the backend entry, not a negotiated media pass');
   }finally{await client.close({timeoutMs:500});}
  },{wtMode:'ok'});
  await assert.rejects(()=>BrowserSdkClient.connect({tenantId:'tnt_default',nativeVideoFeedback:'yes'}),{kind:'validation',certainty:'not-submitted'});
  assert.equal(captured.length,2);
 }finally{BrowserMediaPeer.prototype.beginOfferConnection=original;}
});
