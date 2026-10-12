import sys,os,pathlib,tempfile,json,tomllib,subprocess,socket,time,hashlib
root=pathlib.Path.cwd();work=root/'.local/gate-1-audit';os.umask(0o077)
binary=root/'.local/releases/connect-0.1.0-alpha.1-macos-arm64-263fe1592ceb/bin/connect'
os.environ['CONNECT_BINARY']=str(binary);os.environ.pop('RUST_LOG',None)
sys.path.insert(0,str(root/'scripts'))
from lib.local_process import run,Server,port,request
result={'scope':'Additional Gate 1 audit proof using unchanged archived binaries and fresh isolated state','candidate':'263fe1592ceb5099985e58566ae3e3e35a34bc07','result':'running'}
try:
 with tempfile.TemporaryDirectory(prefix='connect-gate1-audit-') as td:
  state=pathlib.Path(td)/'state';run('init','--directory',state,'--example','review-loop','--bind',f'127.0.0.1:{port()}')
  with socket.socket() as sock:sock.bind(('127.0.0.1',0));wsport=sock.getsockname()[1]
  config=state/'server.toml';config.write_text(f'websocket_bind = "127.0.0.1:{wsport}"\n'+config.read_text())
  client=tomllib.loads((state/'reviews.toml').read_text());ready=pathlib.Path(td)/'ready';out=pathlib.Path(td)/'observed.json';cfg=pathlib.Path(td)/'observer.json'
  cfg.write_text(json.dumps({'credential':client['credential'],'url':f'ws://127.0.0.1:{wsport}','ready':str(ready),'output':str(out)}))
  with Server(state):
   with (work/'combined-observer.log').open('w') as log:
    observer=subprocess.Popen(['node',str(work/'observer.mjs'),str(cfg)],stdout=log,stderr=log)
    try:
     end=time.monotonic()+8
     while not ready.exists():
      if observer.poll() is not None or time.monotonic()>end:raise RuntimeError('observer startup failed')
      time.sleep(.02)
     demo=json.loads(run('demo','verify','--config',state/'operator.toml').stdout)
     assert demo['result']=='passed' and demo['independent_examples']==2 and demo['transport']=='quic'
     assert observer.wait(timeout=12)==0
     observed=json.loads(out.read_text());assert observed['conversations']==2 and observed['messages']==4
     for cid,history in observed['histories'].items():
      native=request(state,'reviews',{'op':'history','conversation_id':cid,'after':None,'limit':100})
      assert native==history,'WS and native history differ'
     result.update(result='passed',independent_native_examples=2,authenticated_ws_observers=1,conversations=2,messages=4,history='Native QUIC CLI and WS observer return identical history',cleanup='All audit processes stopped; temporary state removed')
    finally:
     if observer.poll() is None:observer.kill();observer.wait(timeout=5)
 with socket.socket() as sock:
  sock.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
  sock.bind(('127.0.0.1',wsport))
  sock.listen(1)
 result['ws_port_released']=True
except BaseException as e:
 result.update(result='failed',error=type(e).__name__);raise
finally:
 result['observer_harness_sha256']=hashlib.sha256((work/'observer.mjs').read_bytes()).hexdigest()
 result['runner_sha256']=hashlib.sha256((work/'check-combined.py').read_bytes()).hexdigest()
 (work/'combined-result.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(result))
