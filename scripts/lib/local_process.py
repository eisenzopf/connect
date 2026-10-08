"""Bounded, task-owned local CLI process helpers. No secrets in command arguments."""
import json, os, pathlib, signal, socket, subprocess, tempfile, time
ROOT=pathlib.Path(__file__).resolve().parents[2]
BINARY=pathlib.Path(os.environ.get('CONNECT_BINARY', ROOT/'target/debug/connect')).resolve()

def run(*args,value=None,success=True,binary=BINARY,timeout=15):
    result=subprocess.run([str(binary),*map(str,args)],input=None if value is None else json.dumps(value),text=True,capture_output=True,timeout=timeout)
    if success and result.returncode:raise RuntimeError('CLI command failed: '+str(args[0])+' '+result.stderr)
    if not success:assert result.returncode!=0
    return result

def port():
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
        sock.bind(('127.0.0.1',0));return sock.getsockname()[1]

class Server:
    def __init__(self,state,work=None,binary=BINARY,env=None,expected_exit=0,extra_args=()):
        self.state=state;self.binary=binary;self.env=env;self.expected_exit=expected_exit;self.extra_args=extra_args
    def __enter__(self):
        self.log=tempfile.TemporaryFile(mode='w+')
        self.process=subprocess.Popen([str(self.binary),'serve','--config',str(self.state/'server.toml'),*map(str,self.extra_args)],stdout=self.log,stderr=self.log,env=self.env)
        deadline=time.monotonic()+10
        try:
            while time.monotonic()<deadline:
                self.log.seek(0)
                if 'ready transport=quic' in self.log.read():return self
                if self.process.poll() is not None:raise RuntimeError('Server exited before readiness')
                time.sleep(.02)
            raise TimeoutError('Server readiness timed out')
        except BaseException:
            self.stop();self.log.close();raise
    def stop(self):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:self.process.wait(timeout=8)
            except subprocess.TimeoutExpired:self.process.kill();self.process.wait();raise
    def __exit__(self,*_):
        self.stop();self.log.seek(0);output=self.log.read();self.log.close()
        assert self.process.returncode==self.expected_exit,(self.process.returncode,self.expected_exit)
        if self.expected_exit==0:assert 'stopped' in output

def request(state,name,value,binary=BINARY):
    return json.loads(run('request','--config',state/(name+'.toml'),value=value,binary=binary).stdout)

def config_port(state,new_port):
    import re
    for name in ['server.toml','operator.toml','reviews.toml','agent.toml']:
        path=state/name
        if path.exists():path.write_text(re.sub(r'127\.0\.0\.1:\d+',f'127.0.0.1:{new_port}',path.read_text()))
