"""Candidate normal Chrome156 profile schema proof; no production pin change."""
from pathlib import Path
import argparse,hashlib,importlib.util,json,os,re,shutil,sqlite3,ssl,subprocess,threading
from http.server import HTTPServer,BaseHTTPRequestHandler

root=Path.cwd();parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);selection=parser.add_mutually_exclusive_group(required=True);selection.add_argument('--binary',type=Path);selection.add_argument('--image-browser',type=Path);parser.add_argument('--renderer-manifest',type=Path);parser.add_argument('--playwright-module',type=Path,required=True);args=parser.parse_args()
out=args.output.absolute();assert out.is_relative_to(root/'.local');out.mkdir(parents=True,mode=0o700,exist_ok=False)
if args.image_browser:
 image=json.loads(args.image_browser.read_text());assert image['architecture']=='x64' and len(image['candidates'])==1
 binary=Path(image['candidates'][0]);assert str(binary).startswith('/ms-playwright/chromium-')
else:binary=args.binary.absolute()
module_path=args.playwright_module.absolute()
version=subprocess.check_output([str(binary),'--version'],text=True).strip()
assert re.fullmatch(r'Google Chrome(?: for Testing)? 156\.0\.8078\.(?:4|48)',version),version
sha=lambda p:hashlib.file_digest(Path(p).open('rb'),'sha256').hexdigest()
sources={str(p):sha(p) for p in [Path(__file__).absolute(),root/'scripts/video/browser_profile.py',module_path,module_path.parent/'package.json',binary,Path(shutil.which('node')).resolve()]}
if args.renderer_manifest:
 expected=json.loads(args.renderer_manifest.read_text());assert all(sha(p)==d for p,d in expected.items());sources.update(expected);sources[str(args.renderer_manifest.absolute())]=sha(args.renderer_manifest)
def command(values):
 with (out/'openssl.log').open('ab') as log:subprocess.run(values,stdout=log,stderr=log,check=True)
command(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=Connect owned candidate proof root','-addext','basicConstraints=critical,CA:TRUE','-addext','keyUsage=critical,keyCertSign,cRLSign','-keyout',str(out/'root.key'),'-out',str(out/'root.pem')])
command(['openssl','x509','-in',str(out/'root.pem'),'-outform','DER','-out',str(out/'root.der')])
for name,san in [('leaf','DNS:localhost,IP:127.0.0.1'),('wrong-host','DNS:wrong-host.invalid')]:
 command(['openssl','req','-new','-newkey','rsa:2048','-nodes','-subj','/CN='+('localhost' if name=='leaf' else 'wrong-host.invalid'),'-keyout',str(out/(name+'.key')),'-out',str(out/(name+'.csr'))])
 (out/(name+'.ext')).write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName='+san+'\n')
 command(['openssl','x509','-req','-in',str(out/(name+'.csr')),'-CA',str(out/'root.pem'),'-CAkey',str(out/'root.key'),'-CAcreateserial','-days','1','-extfile',str(out/(name+'.ext')),'-out',str(out/(name+'.pem'))])
spec=importlib.util.spec_from_file_location('profile_candidate',root/'scripts/video/browser_profile.py');helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
baseline_pin=helper.CHROME_VERSION
# Candidate in memory only. The actual executable label/version was independently
# checked above; the tracked helper stays unchanged until this schema proof.
helper.CHROME_VERSION=version.removeprefix('Google Chrome ')
helper.create_profile('chrome',binary,out/'trusted',out/'root.der')
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*values):pass
 def do_GET(self):
  self.send_response(200);self.end_headers();self.wfile.write(b'owned-normal-tls-proof')
servers=[];threads=[]
for name in ['leaf','wrong-host']:
 server=HTTPServer(('127.0.0.1',0),Handler);tls=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);tls.load_cert_chain(out/(name+'.pem'),out/(name+'.key'));server.socket=tls.wrap_socket(server.socket,server_side=True);thread=threading.Thread(target=server.serve_forever);servers.append(server);threads.append(thread);thread.start()
driver=out/'driver.mjs';driver.write_text('''import fs from 'node:fs/promises';import{pathToFileURL}from'node:url';
const spec=JSON.parse(await fs.readFile(process.argv[2],'utf8'));
const{chromium}=await import(pathToFileURL(spec.module).href),results=[];let closed=0;
for(const trusted of [false,true]){let context;try{
 context=await chromium.launchPersistentContext(spec.root+'/'+(trusted?'trusted':'untrusted'),{executablePath:spec.binary,headless:true,ignoreHTTPSErrors:false,timeout:10000,args:['--disable-background-networking','--disable-sync','--disable-breakpad','--no-first-run']});
 const page=context.pages()[0]??await context.newPage();
 const check=async(url,expected)=>{let response,error;try{response=await page.goto(url,{timeout:5000});}catch(e){error=/net::(ERR_[A-Z_]+)/.exec(e.message)?.[1]??'unclassified';}
  if(expected==='accepted'){if(error||response?.status()!==200||await page.textContent('body')!=='owned-normal-tls-proof')throw Error('Trusted normal TLS failed');}
  else if(error!==expected)throw Error('Normal TLS refusal mismatch');
  results.push({trusted,expected,error:error??null,status:response?.status()??null});};
 await check(spec.good,trusted?'accepted':'ERR_CERT_AUTHORITY_INVALID');
 if(trusted)await check(spec.wrong,'ERR_CERT_COMMON_NAME_INVALID');
}finally{if(context){await context.close();closed++;}}}
console.log(JSON.stringify({passed:true,results,normal_certificate_verification:true,original_contexts_closed:closed}));
''')
(out/'input.json').write_text(json.dumps(dict(root=str(out),binary=str(binary),module=str(module_path),good=f'https://127.0.0.1:{servers[0].server_port}/',wrong=f'https://127.0.0.1:{servers[1].server_port}/')))
result=None;timed_out=False
try:
 with (out/'result.json').open('w') as stdout,(out/'driver.log').open('w') as stderr:result=subprocess.run(['node',str(driver),str(out/'input.json')],stdout=stdout,stderr=stderr,timeout=40)
except subprocess.TimeoutExpired:timed_out=True
finally:
 for server in servers:server.shutdown();server.server_close()
 for thread in threads:thread.join()
original_root_retained=False
if result and result.returncode==0:
 with sqlite3.connect(out/'trusted/Default/ServerCertificate') as db:
  row=db.execute('SELECT der_cert,trust_settings FROM certificates WHERE sha256hash_hex=?',(sha(out/'root.der'),)).fetchone()
  original_root_retained=bool(row and row[0]==(out/'root.der').read_bytes() and row[1]==bytes.fromhex('0a020803'))
changed=[p for p,d in sources.items() if sha(p)!=d];observations=json.loads((out/'result.json').read_text()) if result and result.returncode==0 else None
report=dict(scope='Candidate Chrome156 normal private profile on owned loopback HTTPS: untrusted CA refusal, trusted CA acceptance and trusted wrong-host refusal. No RTC/Core/media/remote/account qualification.',candidate_only=True,browser_version=version,original_helper_pin=baseline_pin,helper_file_unchanged=sha(root/'scripts/video/browser_profile.py')==sources[str(root/'scripts/video/browser_profile.py')],source_inputs_sha256=sources,sources_unchanged=not changed,changed_inputs=changed,driver_sha256=sha(driver),root_der_sha256=sha(out/'root.der'),observations=observations,original_servers_joined=len(threads),exit_code=result.returncode if result else None,timed_out=timed_out,original_root_retained=original_root_retained,passed=bool(result and result.returncode==0 and not changed and original_root_retained))
(out/'summary.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));raise SystemExit(not report['passed'])
