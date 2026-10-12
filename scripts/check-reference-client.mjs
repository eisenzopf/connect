/** Isolated, offline Chromium proof of the actual reference UI and public HTTP client.
 * Supply CONNECT_PLAYWRIGHT_MODULE / CONNECT_CHROME_BIN when not installed locally.
 * No provider worker, microphone, real credentials, or retained installation is used.
 */
import {createServer} from 'node:http';
import {once} from 'node:events';
import fs from 'node:fs';
import path from 'node:path';
import {spawn,execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const scratch=fs.mkdtempSync(path.join(root,'.local/pre-gate-4-fixes/browser-'));
fs.chmodSync(scratch,0o700);
const executable=path.join(root,'target/debug/connect'), state=path.join(scratch,'state');
const {chromium}=await import(process.env.CONNECT_PLAYWRIGHT_MODULE || 'playwright');
const {HttpClient}=await import('../sdk/typescript/dist/index.js');
execFileSync(executable,['init','--directory',state,'--example','review-loop','--bind','127.0.0.1:0'],{stdio:'pipe'});
const cfg=JSON.parse(execFileSync('python3',['-c','import json,tomllib,sys;print(json.dumps(tomllib.load(open(sys.argv[1],"rb"))))',path.join(state,'reviews.toml')],{encoding:'utf8'}));
const peer=JSON.parse(execFileSync('python3',['-c','import json,tomllib,sys;print(json.dumps(tomllib.load(open(sys.argv[1],"rb"))))',path.join(state,'agent.toml')],{encoding:'utf8'}));
const reserve=createServer();reserve.listen(0,'127.0.0.1');await once(reserve,'listening');const port=reserve.address().port;await new Promise(resolve=>reserve.close(resolve));
const log=fs.openSync(path.join(scratch,'server.log'),'w',0o600);
const child=spawn(executable,['serve','--config',path.join(state,'server.toml'),'--http-bind',`127.0.0.1:${port}`],{stdio:['ignore',log,log]});
let browser,staticServer;
try {
 const url=`http://127.0.0.1:${port}`,client=new HttpClient(url,cfg.credential);
 let ready=false;for(let i=0;i<100;i++){try{await client.health();ready=true;break;}catch{await new Promise(r=>setTimeout(r,100));}}
 assert(ready,'local server did not become ready');
 const conversation=await client.create('browser-check',[peer.identity_id]);
 for(let i=0;i<101;i++)await client.send(`seed-${i}`,conversation.id,`Seed message ${i}`);
 staticServer=createServer((req,res)=>{
  const requested=new URL(req.url,'http://localhost').pathname;
  if(!['/examples/conversation-client/','/sdk/typescript/dist/'].some(prefix=>requested.startsWith(prefix)) || requested.includes('..')){res.writeHead(404);res.end();return;}
  const file=path.join(root,requested.endsWith('/')?requested+'index.html':requested);
  try{res.setHeader('Content-Type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));}catch{res.writeHead(404);res.end();}
 });staticServer.listen(0,'127.0.0.1');await once(staticServer,'listening');
 browser=await chromium.launch({headless:true,...(process.env.CONNECT_CHROME_BIN?{executablePath:process.env.CONNECT_CHROME_BIN}:{})});
 const page=await browser.newPage({viewport:{width:1400,height:1000}});
 const errors=[];page.on('pageerror',error=>errors.push(error.message));
 await page.addInitScript(()=>{globalThis.microphoneRequests=0;navigator.mediaDevices.getUserMedia=async()=>{globalThis.microphoneRequests++;throw Error('fixture forbids recording');};});
 await page.goto(`http://127.0.0.1:${staticServer.address().port}/examples/conversation-client/`);
 async function connect(){for(const [id,value] of Object.entries({http:url,credential:cfg.credential,identity:cfg.identity_id,participant:peer.identity_id,conversation:conversation.id}))await page.locator('#'+id).fill(value);await page.locator('#connect').click();await page.waitForFunction(()=>document.querySelector('#state').textContent==='Connected'&&document.querySelector('#detail').textContent==='Complete.');}
 await connect();await page.locator('#refresh').click();await page.waitForFunction(()=>document.querySelectorAll('#messages>li').length===100);
 await page.locator('#more-history').click();await page.waitForFunction(()=>document.querySelectorAll('#messages>li').length===101);
 assert.match(await page.locator('#messages').innerText(),/Seed message 100/);
 let lost=false,firstId;
 await page.route(url+'/v1/operations',async route=>{
  const op=route.request().postDataJSON();
  if(op.op==='send'&&op.body==='Lost acknowledgement proof'&&!lost){lost=true;firstId=op.request_id;await route.fetch();await route.abort('failed');return;}
  if(op.op==='send'&&op.body==='Lost acknowledgement proof')assert.equal(op.request_id,firstId);
  await route.continue();
 });
 await page.locator('#message').fill('Lost acknowledgement proof');await page.locator('#message-form').evaluate(form=>form.requestSubmit());
 await page.waitForFunction(()=>document.querySelector('#state').textContent==='Action failed');
 await page.reload();await connect();await page.locator('#retry').click();
 await page.waitForFunction(()=>document.querySelector('#state').textContent==='Saved action recovered'&&document.querySelector('#detail').textContent==='Complete.');
 const first=await client.history(conversation.id,null,100);
 const second=await client.history(conversation.id,first.next,100);
 assert.equal([...first.messages,...second.messages].filter(m=>m.body==='Lost acknowledgement proof').length,1);
 await page.locator('#join').click();await page.waitForFunction(()=>document.querySelector('#state').textContent==='Action failed');
 assert.equal(await page.evaluate(()=>globalThis.microphoneRequests),0);
 await page.locator('#credential').fill('');
 await page.screenshot({path:path.join(scratch,'reference-client.png'),fullPage:true});
 assert.deepEqual(errors,[]);
 const result={result:'passed',scope:'offline real Chromium and local public HTTP interface',checks:['101-message history paging','lost acknowledgement followed by reload and same-ID replay','exactly one committed message','voice refusal before microphone acquisition','no page errors'],screenshot:'reference-client.png'};
 fs.writeFileSync(path.join(scratch,'result.json'),JSON.stringify(result,null,2)+'\n',{mode:0o600});
 console.log(JSON.stringify({...result,evidence_directory:scratch}));
} finally {
 if(browser)await browser.close();if(staticServer)await new Promise(resolve=>staticServer.close(resolve));
 child.kill('SIGINT');await Promise.race([once(child,'exit'),new Promise(resolve=>setTimeout(resolve,5000))]);if(child.exitCode===null)child.kill('SIGKILL');fs.closeSync(log);
}
