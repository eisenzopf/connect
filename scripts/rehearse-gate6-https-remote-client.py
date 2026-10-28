#!/usr/bin/env python3
"""Exercise a frozen Mac host from an owned local Linux/Lima HTTPS client.

The supplied server binary must match its recorded build SHA256. This uses only
synthetic state, loopback listeners and the existing local image, with no model,
provider or named external host. Linux server installation is a separate scope.
"""
import argparse
import base64
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
HOST = "host.lima.internal"
SURFACES = ("application", "administration", "callbacks", "health")
CLIENT = r'''
import hashlib,http.client,json,socket,ssl
from pathlib import Path
v=json.loads(Path('/probe/fixture.json').read_text());context=ssl.create_default_context(cafile='/probe/root.pem');cases=[];leaves=set()
def call(role,path,status,headers=None,body=None,method='GET',case=None):
 c=http.client.HTTPSConnection('host.lima.internal',v['ports'][role],context=context,timeout=5)
 try:
  c.connect();leaves.add(hashlib.sha256(c.sock.getpeercert(binary_form=True)).hexdigest());c.request(method,path,body=body,headers=headers or {});r=c.getresponse();b=r.read(1048577)
  assert len(b)<=1048576 and r.status==status,(case,r.status)
  assert r.getheader('cache-control')=='no-store'
  cases.append({'case':case,'status':r.status,'body_sha256':hashlib.sha256(b).hexdigest()})
  return b,{k.lower():value for k,value in r.getheaders()}
 finally:c.close()
auth={'Authorization':'Bearer '+v['client']};admin={'Authorization':'Bearer '+v['operator'],'Content-Type':'application/json'};origin='https://ui.fixture.invalid'
b,_=call('health','/healthz',200,case='minimal-health');assert json.loads(b)=={'profile':'connect/2','ready':True}
call('application','/v2/discovery',401,case='no-bearer')
b,h=call('application','/v2/discovery',200,{**auth,'Origin':origin},case='authenticated-allowed-origin');assert h['access-control-allow-origin']==origin
for foreign in ['https://foreign.fixture.invalid','null']:
 call('application','/v2/discovery',403,{**auth,'Origin':foreign},case='foreign-origin-'+foreign)
call('application','/v2/discovery',403,{**auth,'Host':'foreign.fixture.invalid'},case='wrong-authority')
call('application','/v2/discovery',403,{**auth,'X-Authenticated-User':'operator'},case='forged-identity')
call('application','/v2/discovery',403,{**auth,'X-Forwarded-Proto':'https','X-Forwarded-Host':'host.lima.internal:'+str(v['ports']['application']),'X-Forwarded-For':'203.0.113.7'},case='untrusted-forwarding')
call('administration','/v2/identities',401,body='malformed',method='POST',case='unauthenticated-admin-before-json')
call('administration','/v2/identities',403,auth,'malformed','POST','non-admin-before-json')
call('administration','/v2/identities',403,{'X-Authenticated-User':'operator'},'malformed','POST','asserted-admin-refused')
call('callbacks','/webhooks/telnyx',401,{'Content-Type':'application/json'},'{}','POST','unsigned-callback')
command={'type':'identity','operation_id':'remote-client-original','alias':'remote-client-original','kind':'human','display_name':None}
for role in ['administration','callbacks','health']:
 call(role,'/v2/discovery',404,auth,case=role+'-no-app-route')
for role in ['application','callbacks','health']:
 call(role,'/v2/identities',404,admin,json.dumps(command),'POST',role+'-no-admin-route')
for _ in range(2):
 b,_=call('administration','/v2/identities',200,admin,json.dumps(command),'POST','admin-same-original-id');assert 'Ok' in json.loads(b)['result']
plain={**command,'operation_id':'remote-client-plaintext','alias':'remote-client-plaintext'};body=json.dumps(plain).encode()
with socket.create_connection(('host.lima.internal',v['ports']['administration']),timeout=5) as c:
 c.sendall(('POST /v2/identities HTTP/1.1\r\nHost: host.lima.internal:'+str(v['ports']['administration'])+'\r\nAuthorization: Bearer '+v['operator']+'\r\nContent-Type: application/json\r\nContent-Length: '+str(len(body))+'\r\n\r\n').replace('\\r\\n','\r\n').encode()+body)
 try:reply=c.recv(256)
 except ConnectionResetError:reply=b''
 assert not reply.startswith(b'HTTP/1.')
cases.append({'case':'plaintext-real-synthetic-operator-refused','passed':True})
for name,ctx,server_name in [('unconfigured-root',ssl.create_default_context(),'host.lima.internal'),('wrong-name',context,'wrong.fixture.invalid')]:
 try:
  with socket.create_connection(('host.lima.internal',v['ports']['health']),timeout=5) as raw:
   with ctx.wrap_socket(raw,server_hostname=server_name):raise AssertionError('Invalid TLS identity accepted')
 except ssl.SSLCertVerificationError:cases.append({'case':name,'certificate_verification_refused':True})
revoke={'type':'revoke_credential','operation_id':'remote-client-revoke','credential_id':v['credential_id']}
b,_=call('administration','/v2/credentials/'+v['credential_id']+'/revoke',200,admin,json.dumps(revoke),'POST','admin-revoke');assert 'Ok' in json.loads(b)['result']
call('application','/v2/discovery',401,auth,case='current-revocation')
assert leaves=={v['leaf_sha256']}
print(json.dumps({'passed':True,'cases':cases,'normal_ca_and_name_validation':True,'wire_leaf_sha256':sorted(leaves),'openssl_version':ssl.OPENSSL_VERSION}),flush=True)
'''


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument("--image", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.output.resolve()
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        parser.error("This fixture requires the declared Mac arm64 server profile")
    if out.exists() or not out.is_relative_to(ROOT / ".local/gate-6"):
        parser.error("Output must be a fresh directory under .local/gate-6")
    if sha(binary) != args.binary_sha256 or not re.fullmatch(r"sha256:[0-9a-f]{64}", args.image):
        parser.error("Supply the recorded binary hash and existing pinned image ID")
    image = json.loads(subprocess.check_output(["docker", "image", "inspect", args.image], timeout=10))[0]
    assert image["Os"] == "linux" and image["Architecture"] == "amd64"
    out.mkdir(parents=True, mode=0o700)
    probe = out / "probe"
    probe.mkdir(mode=0o700)
    state = out / "state"
    name = "connect-g6-https-client-" + uuid.uuid4().hex[:12]
    report = {"task": "G6-053", "passed": False, "binary_sha256": args.binary_sha256,
              "runner_sha256": sha(__file__), "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "server_platform": platform.platform(), "client_image": args.image,
              "scope": "Owned Mac arm64 Remote-policy HTTPS host, Linux amd64 client in the local arm64 Lima VM, normal TLS via host.lima.internal. Every server listener binds loopback. No Ubuntu server, named external remote, browser/media, performance, provider/model or whole-gate qualification.",
              "external_provider_model_calls": 0}
    server = None
    start = time.monotonic()
    openssl = shutil.which("openssl")
    assert openssl
    with (out / "driver.log").open("wb") as log:
        def run(command, timeout=20):
            return subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)

        try:
            run([str(binary), "init", "--directory", str(state), "--bind", "127.0.0.1:0", "--example", "review-loop"])
            run([openssl, "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(out / "root.key"),
                 "-out", str(probe / "root.pem"), "-days", "2", "-subj", "/CN=Connect owned remote-client fixture",
                 "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign"])
            run([openssl, "req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", str(state / "host.key"),
                 "-out", str(out / "host.csr"), "-subj", "/CN=" + HOST])
            ext = out / "leaf.ext"
            ext.write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:" + HOST + ",DNS:localhost\n")
            run([openssl, "x509", "-req", "-in", str(out / "host.csr"), "-CA", str(probe / "root.pem"),
                 "-CAkey", str(out / "root.key"), "-set_serial", "1", "-days", "1", "-extfile", str(ext), "-out", str(state / "host.pem")])
            reservations = []
            ports = {}
            for role in SURFACES:
                reservation = socket.socket()
                reservation.bind(("127.0.0.1", 0))
                reservations.append(reservation)
                ports[role] = reservation.getsockname()[1]
            config_path = state / "server.toml"
            config = config_path.read_text()
            for field, value in (("network_profile", "remote"), ("certificate", "host.pem"), ("private_key", "host.key")):
                config, count = re.subn(rf'^{field} = "[^"]*"$', f'{field} = "{value}"', config, flags=re.M)
                assert count == 1
            for role, port in ports.items():
                config += f'\n[[https]]\nbind = "127.0.0.1:{port}"\nsurface = "{role}"\npublic_origin = "https://{HOST}:{port}"\n'
                if role == "application":
                    config += 'allowed_origins = ["https://ui.fixture.invalid"]\n'
                if role == "callbacks":
                    verifier = base64.b64encode((state / "server-signing.pub").read_bytes()).decode()
                    config += f'[https.telnyx_callbacks]\npublic_key = "{verifier}"\nroutes = ["synthetic-remote-client-only"]\n'
            config_path.write_text(config)
            operator = tomllib.loads((state / "operator.toml").read_text())
            client = tomllib.loads((state / "reviews.toml").read_text())
            with closing(sqlite3.connect(f"file:{state / 'state.sqlite3'}?mode=ro", uri=True)) as database:
                rows = database.execute("SELECT id FROM credentials WHERE identity_id=? AND revoked=0", (client["identity_id"],)).fetchall()
                assert len(rows) == 1
            fixture = {"ports": ports, "operator": operator["credential"], "client": client["credential"],
                       "credential_id": rows[0][0], "leaf_sha256": hashlib.sha256(__import__("ssl").PEM_cert_to_DER_cert((state / "host.pem").read_text())).hexdigest()}
            (probe / "fixture.json").write_text(json.dumps(fixture))
            (probe / "client.py").write_text(CLIENT)
            for path in out.rglob("*"):
                path.chmod(0o700 if path.is_dir() else 0o600)
            for reservation in reservations:
                reservation.close()
            with (out / "server.stdout").open("wb") as stdout, (out / "server.stderr").open("wb") as stderr:
                server = subprocess.Popen([str(binary), "serve", "--config", str(config_path)], stdout=stdout, stderr=stderr, start_new_session=True)
                deadline = time.monotonic() + 10
                while (out / "server.stdout").read_bytes().count(b"https_surface=") != 4:
                    assert server.poll() is None and time.monotonic() < deadline, "Owned Mac server readiness deadline"
                    time.sleep(0.05)
                with (out / "client.log").open("wb") as client_log:
                    child = subprocess.run(["docker", "run", "--rm", "--pull", "never", "--name", name,
                                            "--label", "connect.fixture=gate6-https-client", "--read-only", "--platform", "linux/amd64",
                                            "--network", "bridge", "--cpus", "1", "--memory", "128m", "--pids-limit", "32",
                                            "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--user", f"{os.getuid()}:{os.getgid()}",
                                            "--mount", f"type=bind,source={probe},target=/probe,readonly",
                                            args.image, "python3", "/probe/client.py"], stdout=client_log, stderr=subprocess.STDOUT, timeout=90)
                report["client_exit_code"] = child.returncode
                assert child.returncode == 0, "Owned Linux client failed; inspect retained private client.log"
                observations = [json.loads(line) for line in (out / "client.log").read_text().splitlines() if line.startswith("{")]
                assert len(observations) == 1 and observations[0]["passed"]
                report["client"] = observations[0]
                with closing(sqlite3.connect(f"file:{state / 'state.sqlite3'}?mode=ro", uri=True)) as database:
                    original = database.execute("SELECT count(*) FROM identities WHERE alias='remote-client-original'").fetchone()[0]
                    plaintext = database.execute("SELECT count(*) FROM identities WHERE alias='remote-client-plaintext'").fetchone()[0]
                    assert original == 1 and plaintext == 0
                    report["identity_effects"] = {"same_original_id_requests": 2, "original_id_rows": original, "plaintext_rows": plaintext}
                report["behavior_passed"] = True
        finally:
            if server is not None:
                if server.poll() is None:
                    server.send_signal(signal.SIGTERM)
                try:
                    report["server_exit_code"] = server.wait(timeout=15)
                    report["forced_server_cleanup"] = False
                except subprocess.TimeoutExpired:
                    os.killpg(server.pid, signal.SIGKILL)
                    server.wait(timeout=5)
                    report["forced_server_cleanup"] = True
                logs = (out / "server.stdout").read_bytes() + (out / "server.stderr").read_bytes()
                report["joined_stop_observed"] = b"stopped" in logs.splitlines()
                report["panic_observed"] = b"panicked at" in logs
                report["credential_bytes_in_server_logs"] = any(c["credential"].encode() in logs for c in (operator, client))
            found = subprocess.run(["docker", "inspect", name], capture_output=True, timeout=10)
            if found.returncode == 0:
                metadata = json.loads(found.stdout)[0]
                assert metadata["Config"]["Labels"].get("connect.fixture") == "gate6-https-client"
                run(["docker", "stop", "--time", "5", name])
                report["forced_client_cleanup"] = True
            else:
                report["forced_client_cleanup"] = False
            report["binary_unchanged"] = sha(binary) == args.binary_sha256
            report["runner_unchanged"] = sha(__file__) == report["runner_sha256"]
            report["seconds"] = time.monotonic() - start
            report["passed"] = (report.get("behavior_passed", False) and report.get("server_exit_code") == 0
                                and report.get("joined_stop_observed", False) and not report.get("forced_server_cleanup", True)
                                and not report.get("forced_client_cleanup", True) and not report.get("panic_observed", True)
                                and not report.get("credential_bytes_in_server_logs", True) and report["binary_unchanged"] and report["runner_unchanged"])
            (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
