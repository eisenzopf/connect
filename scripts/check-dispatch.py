#!/usr/bin/env python3
"""Launch separate native server and client processes on an ephemeral loopback port."""
import pathlib, signal, subprocess, tempfile, time, sys
root = pathlib.Path(__file__).resolve().parents[1]
subprocess.run(['cargo', 'build', '--locked', '-p', 'connect-server', '--example', 'quic_dispatch'], cwd=root, check=True)
binary = root / 'target/debug/examples/quic_dispatch'
with tempfile.TemporaryDirectory(prefix='connect-dispatch-') as path:
    directory = pathlib.Path(path)
    with (directory / 'server.log').open('w+') as log:
        server = subprocess.Popen([str(binary), 'server', path], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while not (directory / 'address').exists():
                if server.poll() is not None or time.monotonic() > deadline:
                    log.seek(0); raise RuntimeError(log.read())
                time.sleep(0.03)
            subprocess.run([str(binary), 'client', path], check=True, timeout=15)
        finally:
            if server.poll() is None:
                server.send_signal(signal.SIGINT)
                try: server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill();server.wait();raise
            log.seek(0)
            output=log.read()
            print(output)
            if server.returncode != 0 or 'executions=1' not in output:
                raise SystemExit('Native dispatch server failed or executed unexpected mutations')
