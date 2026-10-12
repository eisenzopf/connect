#!/usr/bin/env python3
"""Repeat the identity journey with independently launched local CLI processes."""
import pathlib, socket, subprocess, tempfile, time, signal, tomllib
root=pathlib.Path(__file__).resolve().parents[1]
subprocess.run(['cargo','build','--locked','-p','connect-server','--bin','connect'],cwd=root,check=True)
binary=root/'target/debug/connect'
def run(*args):
    return subprocess.run([str(binary),*map(str,args)],check=True,text=True,capture_output=True,timeout=15).stdout
with tempfile.TemporaryDirectory(prefix='connect-identity-') as work:
    directory=pathlib.Path(work)/'state'
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
        sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    run('init','--directory',directory,'--example','review-loop','--bind',f'127.0.0.1:{port}')
    before=(directory/'agent.toml').read_bytes()
    run('init','--directory',directory,'--example','review-loop')
    assert (directory/'agent.toml').read_bytes()==before
    for iteration in range(2):
        with (pathlib.Path(work)/f'server-{iteration}.log').open('w+') as log:
            server=subprocess.Popen([str(binary),'serve','--config',str(directory/'server.toml')],stdout=log,stderr=log)
            try:
                deadline=time.monotonic()+10
                while True:
                    log.seek(0)
                    if 'ready transport=quic' in log.read():break
                    if server.poll() is not None or time.monotonic()>deadline:raise RuntimeError('Server failed to start')
                    time.sleep(.03)
                assert 'installation_id' in run('health','--config',directory/'operator.toml')
                assert '"alias": "agent"' in run('resolve','--config',directory/'reviews.toml','connect:agent@local')
                assert '"alias": "reviews"' in run('resolve','--config',directory/'agent.toml','connect:reviews@local')
            finally:
                if server.poll() is None:
                    server.send_signal(signal.SIGTERM)
                    try:server.wait(timeout=10)
                    except subprocess.TimeoutExpired:server.kill();server.wait();raise
                log.seek(0);assert server.returncode==0 and 'stopped' in log.read()
    print('PASS separate CLI processes: init preservation; health; scoped two-way resolution; server restart; SIGTERM shutdown')
