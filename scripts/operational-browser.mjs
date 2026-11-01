// Local operational fixture only: ordinary browser HTTP, no media or providers.
import fs from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import {pathToFileURL} from 'node:url';
import {launchOwnedChromium} from './video/owned_chromium.mjs';
let stage='input';
async function main(){
const input=JSON.parse(await fs.readFile(process.argv[2],'utf8'));
stage='browser-sdk';
const dist=await fs.realpath(input.dist);
stage='browser-runtime';
const {chromium}=await import(pathToFileURL(process.env.CONNECT_PLAYWRIGHT_MODULE).href);
const server=http.createServer(async(req,res)=>{
    try{
        const pathname=new URL(req.url,'http://127.0.0.1').pathname;
        if(pathname==='/'){res.writeHead(200,{'Content-Type':'text/html'});res.end('<!doctype html><title>Owned operational fixture</title>');return;}
        if(!/^\/[a-z0-9_]+\.js$/.test(pathname))throw Error('denied');
        const file=await fs.realpath(path.join(dist,pathname.slice(1)));
        if(!file.startsWith(dist+path.sep))throw Error('denied');
        res.writeHead(200,{'Content-Type':'text/javascript'});res.end(await fs.readFile(file));
    }catch{res.writeHead(404);res.end();}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
let owned;
try{
    stage='browser-launch';
    owned=await launchOwnedChromium(chromium,process.env.CONNECT_GATE6_CHROME,input.profile);
    const page=await owned.browser.newPage();
    stage='page-ready';
    await page.goto('http://127.0.0.1:'+server.address().port,{timeout:5000});
    process.stdout.write(JSON.stringify({phase:'ready'})+'\n');
    stage='health-reads';
    const result=await page.evaluate(async({endpoint,credential})=>{
        const {BrowserHttpClient}=await import('/browser_http.js');
        const client=new BrowserHttpClient(endpoint,credential,4000);
        let reads=0;
        for(let i=0;i<45;i++){
            const value=await client.health();
            if(!value.operational)throw Error('Missing administrator health');
            const operational=value.operational;
            if(!operational.resource_pools?.some(p=>p.kind==='native-quic'&&p.usage?.used>=1))
                throw Error('Original helper peer not observed');
            // Keep outcomes inside the page. Return only fixed codes/counts.
            if(JSON.stringify(operational).includes(credential))throw Error('Credential in diagnostics');
            reads++;
            await new Promise(resolve=>setTimeout(resolve,333));
        }
        return {health_reads:reads,administrator_health:true,helper_peer_observed:true,credential_absent:true};
    },{endpoint:input.endpoint,credential:input.credential});
    stage='browser-drain';
    const joined=await owned.close();owned=null;
    if(!joined.original_process_joined||joined.forced_kill)throw Error('Browser drain not confirmed');
    process.stdout.write(JSON.stringify({phase:'complete',...result,original_browser_joined:true})+'\n');
}finally{
    if(owned)await owned.close();
    server.closeAllConnections();
    await new Promise(resolve=>server.close(resolve));
}
}
try{await main();}catch(error){
    // Never return exception messages, stacks, paths, request bodies or bearer input.
    const name=['Error','TypeError','SyntaxError','TimeoutError'].includes(error?.name)?error.name:'Error';
    const code=['ENOENT','EACCES','EADDRINUSE'].includes(error?.code)?error.code:null;
    process.stdout.write(JSON.stringify({phase:'failed',stage,name,code})+'\n');
    process.exitCode=1;
}
