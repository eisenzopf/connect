"""Explicit isolated browser SDP/capture probe, never a WSS/media qualification.

Run from the source root with a selected Chrome/Firefox binary and a unique label.
Only synthetic canvas/WebAudio sources are used. Canonical client.request is an
in-process fixture that records the offer and refuses before remote submission.
Raw SDP, browser profiles and logs remain under ignored .local/. No account,
provider, personal profile, camera or microphone is accessed.
"""
from pathlib import Path
import http.server,threading,subprocess,os,signal,json,time,hashlib,uuid,sys,urllib.parse,re
if len(sys.argv) not in [4,5] or not re.fullmatch(r'[a-z0-9][a-z0-9._-]{0,63}',sys.argv[3]):raise ValueError('Use browser-name browser-binary unique-private-label [offer|resize]')
mode=sys.argv[4] if len(sys.argv)==5 else 'offer'
if mode not in ['offer','resize']:raise ValueError('unknown probe mode')
root=Path.cwd(); name=sys.argv[1];binary=Path(sys.argv[2]).resolve(strict=True);out=root/'.local/gate-6/browser-offer-profile'/sys.argv[3];out.mkdir(mode=0o700)
nonce=uuid.uuid4().hex;done=threading.Event();rows={};assets=root/'sdk/typescript/dist'
page='''<!doctype html><meta charset="utf-8"><title>Synthetic canonical offer profile</title><script type="module">
import {BrowserVideoSession,ConnectError} from '/sdk/browser.js';
const report={mode:'PROBE_MODE',browser:navigator.userAgent,secureContext:isSecureContext,encodedTransform:typeof RTCRtpScriptTransform==='function'};
let audio,oscillator,tracks=[],canvasTracks=[];
const nativeCanvasCapture=HTMLCanvasElement.prototype.captureStream;
report.canvasCaptureRates=[];
HTMLCanvasElement.prototype.captureStream=function(...args){const stream=nativeCanvasCapture.apply(this,args);report.canvasCaptureRates.push(args[0]);canvasTracks.push(...stream.getTracks());return stream;};
report.trackStopCounts={audio:0,video:0};const nativeStop=MediaStreamTrack.prototype.stop;MediaStreamTrack.prototype.stop=function(){if(this.kind in report.trackStopCounts)report.trackStopCounts[this.kind]++;return nativeStop.call(this);};
try {
 const source=document.createElement('canvas');source.width=640;source.height=360;const paint=source.getContext('2d');paint.fillStyle='red';paint.fillRect(0,0,640,360);
 audio=new AudioContext({sampleRate:48000});await audio.resume();oscillator=audio.createOscillator();const destination=audio.createMediaStreamDestination();destination.channelCount=1;oscillator.connect(destination);oscillator.start();
 const stream=new MediaStream(destination.stream.getAudioTracks());tracks=stream.getTracks();report.capture=tracks.map(t=>({kind:t.kind,settings:t.getSettings(),constraints:t.getConstraints(),capabilities:t.getCapabilities?.()}));
 for(const track of tracks) {
   const constrain=track.applyConstraints.bind(track);track.applyConstraints=async constraints=>{try{return await constrain(constraints);}catch(error){report.nativeConstraintError={name:error.name,message:error.message,constraint:error.constraint,requested:constraints};throw error;}};
 }
 const peerCreate=RTCPeerConnection.prototype.createOffer;RTCPeerConnection.prototype.createOffer=async function(...args){try{return await peerCreate.apply(this,args);}catch(error){report.nativeOfferError={name:error.name,message:error.message};throw error;}};

 const client={controlTransport:'websocket',closed:false,onClose(){return()=>{};},onEvent(){return()=>{};},async request(type,payload,scope){
   await fetch('/NONCE/offer',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({type,payload,scope})});
   if(report.mode==='resize')return await new Promise(()=>{});
   throw new ConnectError('fixture-offer-observed','Isolated synthetic offer captured before any remote submission');
 }};
 if(report.mode==='resize')setTimeout(()=>{source.width=1280;},50);
 await BrowserVideoSession.open(client,{conversationId:'synthetic-conversation',sessionId:'synthetic-session',connectionId:'synthetic-connection',operationId:'synthetic-original-offer',participantId:'synthetic-participant',audioStreamId:'synthetic-audio',videoStreamId:'synthetic-video',expectedRevision:0,inputStream:stream,videoCanvas:source},{timeoutMs:15000});
 report.result='unexpected-success';
} catch(error){report.result=report.mode==='resize'&&error.code==='unsupported-media-profile'?'dimension-retired':error.code==='fixture-offer-observed'?'local-offer-observed':'failed';report.errorCode=error.code;report.errorName=error.name;report.errorMessage=error.message;report.originalSubmissionAttempted=error.savedOffer?.submissionAttempted;}
finally {
 report.canvasTracksAfterRelease=canvasTracks.map(t=>({kind:t.kind,state:t.readyState}));
 report.tracksAfterRelease=tracks.map(t=>({kind:t.kind,state:t.readyState}));for(const track of tracks)if(track.readyState==='live')track.stop();try{oscillator?.stop();}catch{}await audio?.close();
 await fetch('/NONCE/done',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(report)});
}
</script>'''.replace('NONCE',nonce).replace('PROBE_MODE',mode).encode()
class Handler(http.server.BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_GET(self):
  path=urllib.parse.urlparse(self.path).path
  if path=='/':data=page;mime='text/html'
  elif path.startswith('/sdk/'):
   target=(assets/urllib.parse.unquote(path[5:])).resolve()
   if not target.is_relative_to(assets) or not target.is_file():self.send_error(404);return
   data=target.read_bytes();mime='text/javascript'
  else:self.send_error(404);return
  self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 def do_POST(self):
  label=self.path.removeprefix('/'+nonce+'/')
  if self.path!='/'+nonce+'/'+label or label not in ['offer','done']:self.send_error(404);return
  length=int(self.headers.get('Content-Length','0'))
  if not 0<length<=1024*1024:self.send_error(413);return
  value=json.loads(self.rfile.read(length));rows[label]=value;(out/(label+'.json')).write_text(json.dumps(value,indent=2)+'\n')
  self.send_response(204);self.end_headers()
  if label=='done':done.set()
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start();url=f'http://127.0.0.1:{server.server_port}/';profile=out/'profile'
if name=='chrome':args=[str(binary),'--headless=new','--disable-background-networking','--disable-component-update','--disable-default-apps','--no-first-run','--no-default-browser-check','--autoplay-policy=no-user-gesture-required','--user-data-dir='+str(profile),url]
elif name=='firefox':
 profile.mkdir();(profile/'user.js').write_text('user_pref("media.autoplay.default", 0);\nuser_pref("media.autoplay.blocking_policy", 0);\nuser_pref("toolkit.telemetry.enabled", false);\nuser_pref("browser.shell.checkDefaultBrowser", false);\nuser_pref("browser.startup.homepage_override.mstone", "ignore");\n')
 args=[str(binary),'--headless','--no-remote','--profile',str(profile),url]
else:raise ValueError('unknown browser')
inputs={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for folder in ['sdk/typescript/src','sdk/typescript/dist'] for p in (root/folder).glob('*')if p.is_file()}
inputs[str(Path(__file__).resolve().relative_to(root))]=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
start=time.monotonic();log=(out/'browser.log').open('wb');process=subprocess.Popen(args,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
try:
 observed=done.wait(30)
finally:
 try:os.killpg(process.pid,signal.SIGTERM)
 except ProcessLookupError:pass
 try:process.wait(timeout=5)
 except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);process.wait(timeout=5)
 log.close();server.shutdown();server.server_close();thread.join(timeout=2)
result={'browser':name,'binary':str(binary),'seconds':time.monotonic()-start,'observed':observed,'process_exit_code':process.returncode,'process_joined':process.poll() is not None,'input_hashes':inputs,'changed_inputs':[name for name,h in inputs.items()if hashlib.sha256((root/name).read_bytes()).hexdigest()!=h],'report':rows.get('done')}
(out/'summary.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'browser':name,'seconds':result['seconds'],'observed':observed,'joined':result['process_joined'],'changed_inputs':result['changed_inputs'],'result':rows.get('done',{}).get('result'),'errorCode':rows.get('done',{}).get('errorCode'),'trackStopCounts':rows.get('done',{}).get('trackStopCounts'),'tracksAfterRelease':rows.get('done',{}).get('tracksAfterRelease'),'canvasTracksAfterRelease':rows.get('done',{}).get('canvasTracksAfterRelease'),'canvasCaptureRates':rows.get('done',{}).get('canvasCaptureRates')}))
if result['changed_inputs'] or not observed or rows['done'].get('result')!=('dimension-retired'if mode=='resize'else'local-offer-observed') or any(t['state']!='ended'for t in rows['done']['tracksAfterRelease']+rows['done']['canvasTracksAfterRelease']) or rows['done']['canvasCaptureRates'] != [30]:raise SystemExit(1)
