from pathlib import Path
import hashlib,json,os,shutil,subprocess,sys,tempfile,time
os.umask(0o077);root=Path.cwd();archive=Path(sys.argv[1]).resolve();output=Path(sys.argv[2]).resolve();output.mkdir(parents=True,mode=0o700,exist_ok=False);assert os.getuid()>0
node=shutil.which('node');assert node;results=[];began=time.monotonic()
with tempfile.TemporaryDirectory(prefix='connect-first-refusals-',dir='/tmp') as temp:
 temp=Path(temp).resolve();assert not temp.is_relative_to(root.resolve());project=temp/'consumer';project.mkdir(mode=0o700);home=temp/'home';home.mkdir(mode=0o700);env=dict(PATH=os.environ['PATH'],HOME=str(home),TMPDIR=str(temp),LANG='C.UTF-8')
 def run(name,args,expected_success):
  p=subprocess.run(args,cwd=project,env=env,capture_output=True,timeout=120);(output/(name+'.log')).write_bytes(p.stdout+p.stderr);assert (p.returncode==0)==expected_success,(name,p.returncode);results.append(dict(name=name,exit_code=p.returncode));return p
 run('extract',['tar','-xzf',str(archive),'--strip-components=1'],True)
 run('missing-sdk',[node,'app/run.mjs'],False);assert not (project/'.local/parley').exists()
 deps=sorted(str(p.relative_to(project)) for p in (project/'sdk/dependencies').glob('*.tgz'));assert len(deps)==2
 run('install',['npm','install','--offline','--ignore-scripts','--no-audit','--no-fund','--no-save','--cache','.local/npm-cache','./sdk/rudeless-connect-0.1.0.tgz',*['./'+n for n in deps]],True)
 env['PATH']=str(project/'runtime/bin')+':'+env['PATH']
 run('missing-runtime',[node,'app/run.mjs','--connect',str(project/'no-such-connect')],False);assert not (project/'.local/parley').exists()
 bad=project/'.local/not-private';bad.mkdir(mode=0o755);bad.chmod(0o755)
 run('non-private-state',[node,'app/run.mjs','--state','.local/not-private'],False);assert list(bad.iterdir())==[]
 readonly=project/'.local/read-only';readonly.mkdir(mode=0o700);readonly.chmod(0o500)
 try:run('unwritable-state',[node,'app/run.mjs','--state','.local/read-only'],False)
 finally:readonly.chmod(0o700)
 assert not list((project/'.local').rglob('server.toml')),'negative setup reached live bootstrap'
 target=project/'app/ui/app.css';target.write_text(target.read_text()+'\n/* explicit owned corruption probe */\n')
 run('corrupt-kit',[node,'verify.mjs'],False)
proof=dict(format='connect/first-conversation-refusal-probes/1',passed=True,platform=sys.platform,ordinary_uid=os.getuid(),consumer_outside_source_checkout=True,negative_setup_created_server_state=False,checks=results,archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),elapsed_seconds=time.monotonic()-began)
(output/'summary.json').write_text(json.dumps(proof,indent=2)+'\n');print(json.dumps(proof))
