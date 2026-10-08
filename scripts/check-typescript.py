#!/usr/bin/env python3
"""Measure a TypeScript-hosted native helper against a separate fixture server."""
import pathlib, subprocess, tempfile, signal, time, shutil
root=pathlib.Path(__file__).resolve().parents[1]
subprocess.run(['cargo','build','--locked','-p','connect-server','--example','quic_dispatch'],cwd=root,check=True)
binary=root/'target/debug/examples/quic_dispatch'
with tempfile.TemporaryDirectory(prefix='connect-ts-') as work:
    path=pathlib.Path(work)
    helper=path/'native-helper'
    probe=path/'probe.ts'
    shutil.copy2(binary,helper)
    shutil.copy2(root/'spikes/typescript-transport/probe.ts',probe)
    with (path/'server.log').open('w+') as log:
        server=subprocess.Popen([str(binary),'server',work],stdout=log,stderr=log)
        try:
            deadline=time.monotonic()+10
            while not (path/'address').exists():
                if server.poll() is not None or time.monotonic()>deadline:raise RuntimeError('Server did not start')
                time.sleep(.03)
            subprocess.run(['node',str(probe),str(helper),work],check=True,timeout=20)
        finally:
            if server.poll() is None:
                server.send_signal(signal.SIGINT)
                try:server.wait(timeout=10)
                except subprocess.TimeoutExpired:server.kill();server.wait();raise
            log.seek(0);output=log.read()
            assert server.returncode==0 and 'executions=1' in output,output
