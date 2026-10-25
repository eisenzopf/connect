#!/usr/bin/env node
// Explicit isolated installed-Chrome primitive checkpoint; no personal browser state.
import {createServer} from 'node:http';
import {readFile,writeFile,mkdir,stat} from 'node:fs/promises';
import {resolve,dirname,relative} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const directory=resolve(root,process.argv[2]??'');
if(!directory.startsWith(root+'/.local/'))throw Error('Provide a fresh output directory under .local/');
await mkdir(dirname(directory),{recursive:true});await mkdir(directory);await mkdir(resolve(directory,'profile'));
const fixture='crates/connect-server/tests/fixtures/gate6-webtransport-browser/';
const names=['browser_codecs.js','browser_rtp.js','uctp_wire.js','uctp_json.js','errors.js'];
const inputs=[...names.map(n=>'sdk/typescript/dist/'+n),'sdk/typescript/src/browser_codecs.ts','sdk/typescript/src/browser_rtp.ts','scripts/rehearse-gate6-browser-codecs.mjs',fixture+'sdk-codecs.mjs',fixture+'run-browser.mjs','dependencies/source-lock.json'];
const digest=b=>createHash('sha256').update(b).digest('hex');
const snapshot=async()=>Object.fromEntries(await Promise.all(inputs.map(async name=>[name,digest(await readFile(resolve(root,name)))])));
const before=await snapshot();const report={passed:false,started_at:new Date().toISOString(),source_inputs_sha256:before,scope:'Explicit local short synthetic SDK WebCodecs/RTP/canvas primitive checkpoint; no capture, network/native, 60s qualification, complete G6-050, remote/provider, acceptance or publication claim'};
const routes=new Map(await Promise.all(names.map(async n=>['/sdk/'+n,await readFile(resolve(root,'sdk/typescript/dist',n))])));
routes.set('/sdk-codecs.mjs',await readFile(resolve(root,fixture+'sdk-codecs.mjs')));
routes.set('/',Buffer.from("<!doctype html><meta charset=utf-8><script type=module>import {run} from '/sdk-codecs.mjs';globalThis.runBrowserFixture=run;</script>"));
const server=createServer((req,res)=>{const body=routes.get(req.url);res.writeHead(body?200:404,{'Content-Type':req.url==='/'?'text/html':'text/javascript','Cache-Control':'no-store','Connection':'close'});res.end(body??'unavailable');});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const chromePath=process.env.CONNECT_GATE6_CHROME??'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
let chrome,driver;
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const run=(command,args)=>new Promise((resolve,reject)=>{
  driver=spawn(command,args,{cwd:root,stdio:['ignore','pipe','pipe']});let out='',err='';
  const timer=setTimeout(()=>{driver.kill('SIGKILL');reject(Error('owned browser codec driver deadline'));},30000);
  driver.stdout.on('data',b=>{out+=b;if(out.length>1048576)driver.kill('SIGKILL');});driver.stderr.on('data',b=>{err+=b;if(err.length>1048576)driver.kill('SIGKILL');});
  driver.on('error',e=>{clearTimeout(timer);reject(e);});driver.on('exit',code=>{clearTimeout(timer);code===0?resolve(out):reject(Error(err||'browser codec driver failed'));});
});
try{
  await stat(chromePath);
  chrome=spawn(chromePath,['--headless=new','--remote-debugging-address=127.0.0.1','--remote-debugging-port=0','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-sync','--disable-breakpad','--disable-crash-reporter','--user-data-dir='+resolve(directory,'profile'),'about:blank'],{detached:true,stdio:['ignore','ignore','pipe']});
  let log='';chrome.stderr.on('data',b=>{if(log.length<1048576)log+=b;});
  const until=Date.now()+10000;let port;
  for(;;){try{port=Number((await readFile(resolve(directory,'profile/DevToolsActivePort'),'utf8')).split('\n')[0]);if(port>0)break;}catch{}if(chrome.exitCode!==null||Date.now()>until)throw Error('owned Chrome startup deadline');await wait(50);}
  const config=resolve(directory,'config.json');await writeFile(config,JSON.stringify({debug_port:port,page_url:'http://127.0.0.1:'+server.address().port+'/',run:{actors:[]}}));
  const out=await run(process.execPath,[resolve(root,fixture+'run-browser.mjs'),config]);
  const result=JSON.parse(out);report.browser=result.browser;report.result=result.result;
  await writeFile(resolve(directory,'chrome-stderr.log'),log);
  report.source_inputs_unchanged=JSON.stringify(before)===JSON.stringify(await snapshot());
  report.passed=result.result.passed===true&&report.source_inputs_unchanged;
}catch(e){report.failure=String(e);}
finally{
  driver?.kill('SIGKILL');if(chrome?.pid){try{process.kill(-chrome.pid,'SIGKILL');}catch{}}
  server.closeAllConnections();await new Promise(r=>server.close(r));
  report.finished_at=new Date().toISOString();await writeFile(resolve(directory,'summary.json'),JSON.stringify(report,null,2)+'\n');
}
console.log(JSON.stringify({passed:report.passed,summary:relative(root,resolve(directory,'summary.json')),failure:report.failure}));
process.exitCode=report.passed?0:1;
