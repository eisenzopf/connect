#!/usr/bin/env python3
"""Exercise real HTTPS/WSS trust in a fresh local qualification browser profile.

All certificate/key inputs and output belong under ignored .local/. Supply a
localhost-only leaf, a foreign-CA localhost leaf, and the first leaf's CA DER.
No certificate exceptions, verification bypasses or global trust edits are used.
"""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import ssl
import subprocess
import threading
import time

from browser_profile import create_profile

ROOT = Path(__file__).resolve().parents[2]


def run(arguments):
    output = arguments.output.absolute()
    private = (ROOT / '.local').resolve()
    for path in [output, arguments.root_der, arguments.leaf_cert, arguments.leaf_key,
                 arguments.foreign_cert, arguments.foreign_key]:
        if not path.resolve().is_relative_to(private):
            raise ValueError('Keep TLS fixture inputs and output under ignored .local/')
    sources = [Path(__file__).resolve(), ROOT / 'scripts/video/browser_profile.py']
    inputs = sources + [arguments.binary, arguments.root_der, arguments.leaf_cert,
                        arguments.leaf_key, arguments.foreign_cert, arguments.foreign_key]
    if arguments.browser == 'firefox':
        inputs += [arguments.binary.parent / name for name in
                   ['libmozglue.dylib', 'libnss3.dylib', 'libsoftokn3.dylib']]
    hashes = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs}
    output.mkdir(mode=0o700)
    profile = output / 'profile'
    if arguments.without_root:
        profile.mkdir(mode=0o700)
    else:
        metadata = create_profile(arguments.browser, arguments.binary, profile, arguments.root_der)
        (output / 'profile.json').write_text(json.dumps(metadata, indent=2) + '\n')
    rows, requests = {}, []
    done = threading.Event()
    servers, threads = [], []

    class TLS(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def do_GET(self):
            requests.append({'server': self.server.label, 'path': self.path,
                             'host': self.headers.get('host')})
            if self.path == '/ws':
                key = self.headers['Sec-WebSocket-Key']
                accept = base64.b64encode(hashlib.sha1(
                    (key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
                self.send_response(101)
                for name, value in [('Upgrade', 'websocket'), ('Connection', 'Upgrade'),
                                    ('Sec-WebSocket-Accept', accept)]:
                    self.send_header(name, value)
                self.end_headers()
                self.wfile.flush()
                self.connection.settimeout(5)
                try:
                    header = self.rfile.read(2)
                    if len(header) == 2 and header[0] == 0x88 and header[1] & 0x80:
                        length = header[1] & 127
                        if length < 126:
                            self.rfile.read(length + 4)
                            self.wfile.write(bytes([136, 0]))
                            self.wfile.flush()
                except OSError:
                    pass
                self.close_connection = True
            else:
                body = b'connect-tls-ok'
                self.send_response(200)
                self.send_header('Access-Control-Allow-Origin', '*')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        def log_message(self, *unused):
            pass

    def serve(server):
        servers.append(server)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        threads.append(thread)
        return server

    process = None
    started = time.monotonic()
    observed = False
    try:
        for label, cert, key in [('valid', arguments.leaf_cert, arguments.leaf_key),
                                 ('foreign', arguments.foreign_cert, arguments.foreign_key)]:
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), TLS)
            server.label = label
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(cert, key)
            server.socket = context.wrap_socket(server.socket, server_side=True)
            serve(server)
        targets = {'valid': f'localhost:{servers[0].server_port}',
                   'foreign': f'localhost:{servers[1].server_port}',
                   'wrong_name': f'127.0.0.1:{servers[0].server_port}'}
        page = '''<!doctype html><script>
const targets=TARGETS;
async function run(){
 const report={userAgent:navigator.userAgent,secureContext:isSecureContext,results:{}};
 for(const [name,host] of Object.entries(targets)){
  let https=false;
  try{https=(await(await fetch('https://'+host+'/',{
   signal:AbortSignal.timeout(5000)})).text())==='connect-tls-ok'}catch{}
  const wss=await new Promise(resolve=>{
   const socket=new WebSocket('wss://'+host+'/ws');
   const timer=setTimeout(()=>{socket.close();resolve(false)},5000);
   socket.onopen=()=>{clearTimeout(timer);socket.close();resolve(true)};
   socket.onerror=()=>{clearTimeout(timer);resolve(false)};
  });
  report.results[name]={https,wss};
 }
 await fetch('/done',{method:'POST',body:JSON.stringify(report)});
}
run();</script>'''.replace('TARGETS', json.dumps(targets)).encode()

        class HTTP(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Type', 'text/html')
                self.end_headers()
                self.wfile.write(page)

            def do_POST(self):
                if self.path != '/done' or int(self.headers.get('Content-Length', 0)) > 8192:
                    self.send_error(400)
                    return
                rows.update(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                self.send_response(200)
                self.end_headers()
                done.set()

            def log_message(self, *unused):
                pass

        assets = serve(http.server.ThreadingHTTPServer(('127.0.0.1', 0), HTTP))
        if arguments.browser == 'chrome':
            command = [str(arguments.binary), '--headless=new', '--disable-background-networking',
                       '--disable-component-update', '--disable-default-apps', '--no-first-run',
                       '--no-default-browser-check', '--user-data-dir=' + str(profile)]
        else:
            (profile / 'user.js').write_text('''user_pref("toolkit.telemetry.enabled", false);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
''')
            command = [str(arguments.binary), '--headless', '--no-remote', '--profile', str(profile)]
        command.append(f'http://127.0.0.1:{assets.server_port}/')
        with (output / 'browser.log').open('w') as log:
            process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
            observed = done.wait(35)
    finally:
        if process:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
        for server in servers:
            server.shutdown()
            server.server_close()
        for thread in threads:
            thread.join(timeout=2)
    trusted = not arguments.without_root
    expected = {'valid': {'https': trusted, 'wss': trusted},
                'foreign': {'https': False, 'wss': False},
                'wrong_name': {'https': False, 'wss': False}}
    changed = [path for path, digest in hashes.items()
               if hashlib.sha256(Path(path).read_bytes()).hexdigest() != digest]
    passed = (not changed and observed and process and process.poll() is not None and rows.get('secureContext')
              and rows.get('results') == expected and len(requests) == (2 if trusted else 0)
              and all(row['server'] == 'valid' and row['host'] == targets['valid']
                      for row in requests))
    report = {'browser': arguments.browser, 'trusted_profile': trusted, 'passed': bool(passed),
              'seconds': time.monotonic() - started, 'observed': observed,
              'process_joined': process is not None and process.poll() is not None,
              'exit_code': process.returncode if process else None, 'report': rows,
              'application_requests': requests, 'arguments': command,
              'sources': {str(path.relative_to(ROOT)): hashes[str(path)] for path in sources},
              'inputs': hashes, 'changed_inputs': changed}
    (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report[key] for key in ['browser', 'trusted_profile', 'passed',
                                                 'seconds', 'process_joined', 'report']}))
    if not passed:
        raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('browser', choices=['chrome', 'firefox'])
    for name in ['binary', 'root_der', 'leaf_cert', 'leaf_key', 'foreign_cert', 'foreign_key', 'output']:
        parser.add_argument(name, type=lambda value: Path(value).absolute())
    parser.add_argument('--without-root', action='store_true')
    run(parser.parse_args())


if __name__ == '__main__':
    main()
