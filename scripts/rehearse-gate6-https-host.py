#!/usr/bin/env python3
"""Exercise an owned, isolated Linux HTTPS host with synthetic installation state.

Supply a source-bound Linux amd64 connect binary and an existing pinned Ubuntu
image. No image pull, external endpoint, model, telephone or provider is used.
The output retains private state and raw logs under .local/gate-6.
"""
import argparse
import base64
import datetime
import hashlib
import http.client
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import sqlite3
import ssl
import subprocess
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
ORIGIN = "https://ui.fixture.invalid"
SURFACES = {"application": 8443, "administration": 8444, "callbacks": 8445, "health": 8446}


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument("--image", required=True, help="Existing sha256 image ID; never pulled")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--openssl", default=shutil.which("openssl"))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / ".local/gate-6") or output.exists():
        parser.error("--output must be a fresh directory under this workspace's .local/gate-6")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", args.image):
        parser.error("--image must be a pinned existing sha256 image ID")
    if digest(binary) != args.binary_sha256:
        parser.error("Binary SHA256 does not match")
    with binary.open("rb") as stream:
        header = stream.read(64)
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != 62:
        parser.error("Binary must be little-endian Linux ELF amd64")
    if not args.openssl:
        parser.error("OpenSSL is required for isolated fixture certificates")
    image = json.loads(subprocess.check_output(["docker", "image", "inspect", args.image], timeout=10))[0]
    if image["Os"] != "linux" or image["Architecture"] != "amd64":
        parser.error("The existing image must be Linux amd64")
    output.mkdir(parents=True, mode=0o700)
    state = output / "runtime"
    state.mkdir(mode=0o700)
    name = "connect-g6-https-" + uuid.uuid4().hex[:12]
    network = name + "-network"
    report = {
        "task": "G6-053", "passed": False,
        "scope": "Owned isolated Linux amd64 HTTPS server and host client across an internal container network and loopback-only published ports. Emulation is not native x86 performance, named remote, browser/media or complete installation qualification.",
        "client_platform": platform.platform(),
        "docker_architecture": subprocess.check_output(["docker", "info", "--format", "{{.Architecture}}"], text=True, timeout=10).strip(),
        "openssl_version": subprocess.check_output([args.openssl, "version"], text=True, timeout=5).strip(),
        "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=10).strip(),
        "runner_sha256": digest(__file__), "binary_sha256": args.binary_sha256,
        "image_id": args.image, "image_architecture": image["Architecture"],
        "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "cases": [], "container_created": False, "network_created": False,
        "external_provider_model_calls": 0,
    }
    start = time.monotonic()
    log = (output / "driver.log").open("wb")

    def run(command, timeout=20):
        return subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)

    def openssl(*command):
        run([args.openssl, *command])

    common = ["--pull", "never", "--read-only", "--platform", "linux/amd64", "--cpus", "1", "--memory", "512m",
              "--pids-limit", "128", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
              "--tmpfs", "/tmp:rw,noexec,nosuid,size=16m", "--user", f"{os.getuid()}:{os.getgid()}",
              "--mount", f"type=bind,source={binary},target=/app/connect,readonly",
              "--mount", f"type=bind,source={state},target=/state"]
    try:
        run(["docker", "run", "--rm", *common, "--name", name + "-init", "--label", "connect.fixture=gate6-https", "--network", "none", args.image,
             "/app/connect", "init", "--directory", "/state/server", "--example", "review-loop",
             "--bind", "127.0.0.1:0"], timeout=60)
        installation = state / "server"
        operator = tomllib.loads((installation / "operator.toml").read_text())
        client = tomllib.loads((installation / "reviews.toml").read_text())
        assert (installation / "operator.toml").stat().st_mode & 0o077 == 0
        # A private, short-lived CA and two independently keyed server leaves.
        openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(state / "root.key"),
                "-out", str(state / "root.pem"), "-subj", "/CN=Connect isolated hosting fixture", "-days", "2",
                "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        extensions = state / "leaf.ext"
        extensions.write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost\n")
        for revision in (1, 2):
            openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", str(state / f"leaf{revision}.key"),
                    "-out", str(state / f"leaf{revision}.csr"), "-subj", "/CN=localhost")
            openssl("x509", "-req", "-in", str(state / f"leaf{revision}.csr"), "-CA", str(state / "root.pem"),
                    "-CAkey", str(state / "root.key"), "-set_serial", str(revision), "-days", "1",
                    "-extfile", str(extensions), "-out", str(state / f"leaf{revision}.pem"))
        # Provider callback verifier is a separate synthetic public key only.
        openssl("genpkey", "-algorithm", "ED25519", "-out", str(state / "callback.key"))
        openssl("pkey", "-in", str(state / "callback.key"), "-pubout", "-outform", "DER", "-out", str(state / "callback.pub"))
        public = (state / "callback.pub").read_bytes()
        assert public[:12] == bytes.fromhex("302a300506032b6570032100") and len(public) == 44
        verifier = base64.b64encode(public[12:]).decode()
        certificate = installation / "host.pem"
        key = installation / "host.key"
        shutil.copyfile(state / "leaf1.pem", certificate)
        shutil.copyfile(state / "leaf1.key", key)
        config_path = installation / "server.toml"
        config = config_path.read_text()
        for field, value in (("network_profile", "remote"), ("certificate", "host.pem"), ("private_key", "host.key")):
            config, count = re.subn(rf'^{field} = "[^"]*"$', f'{field} = "{value}"', config, flags=re.M)
            assert count == 1
        for surface, port in SURFACES.items():
            config += f'\n[[https]]\nbind = "0.0.0.0:{port}"\nsurface = "{surface}"\npublic_origin = "https://localhost"\n'
            if surface == "application":
                config += f'allowed_origins = ["{ORIGIN}"]\n'
            if surface == "callbacks":
                config += f'[https.telnyx_callbacks]\npublic_key = "{verifier}"\nroutes = ["synthetic-hosting-only"]\n'
        config_path.write_text(config)
        for path in state.rglob("*"):
            path.chmod(0o700 if path.is_dir() else 0o600)
        run(["docker", "network", "create", "--internal", "--label", "connect.fixture=gate6-https", network])
        report["network_created"] = True
        command = ["docker", "create", *common, "--name", name, "--label", "connect.fixture=gate6-https",
                   "--network", network]
        for port in SURFACES.values():
            command += ["--publish", f"127.0.0.1::{port}/tcp"]
        # Docker allocates random host ports at start. Keep the process behind a
        # bounded local barrier until its exact public contacts are configured.
        command += [args.image, "/bin/sh", "-c",
                    'n=0; while [ ! -f /state/ready-to-serve ]; do n=$((n+1)); [ "$n" -lt 300 ] || exit 78; sleep 0.1; done; exec /app/connect serve --config /state/server/server.toml']
        run(command)
        report["container_created"] = True
        run(["docker", "start", name])
        metadata = json.loads(subprocess.check_output(["docker", "inspect", name], timeout=10))[0]
        mappings = metadata["NetworkSettings"]["Ports"]
        ports = {}
        for surface, port in SURFACES.items():
            mapping = mappings[f"{port}/tcp"]
            assert len(mapping) == 1 and mapping[0]["HostIp"] == "127.0.0.1"
            ports[surface] = int(mapping[0]["HostPort"])
        for surface, port in SURFACES.items():
            before = f'bind = "0.0.0.0:{port}"\nsurface = "{surface}"\npublic_origin = "https://localhost"'
            after = f'bind = "0.0.0.0:{port}"\nsurface = "{surface}"\npublic_origin = "https://localhost:{ports[surface]}"'
            assert config.count(before) == 1
            config = config.replace(before, after)
        config_path.write_text(config)
        (state / "ready-to-serve").touch(mode=0o600)
        report["published_contacts"] = {surface: f"https://localhost:{port}" for surface, port in ports.items()}
        network_metadata = json.loads(subprocess.check_output(["docker", "network", "inspect", network], timeout=10))[0]
        assert network_metadata["Internal"] and len(metadata["Mounts"]) == 2
        assert {m["Destination"]: m["RW"] for m in metadata["Mounts"]} == {"/app/connect": False, "/state": True}
        report["deployment"] = {"read_only_root": metadata["HostConfig"]["ReadonlyRootfs"],
                                "loopback_only_published_ports": len(ports), "network_internal": True,
                                "capabilities_dropped": metadata["HostConfig"]["CapDrop"],
                                "synthetic_state_only_writable_mount": True}
        assert report["deployment"]["read_only_root"]
        context = ssl.create_default_context(cafile=str(state / "root.pem"))

        def request(surface, path, status, headers=None, body=None, method="GET", case=None):
            connection = http.client.HTTPSConnection("localhost", ports[surface], context=context, timeout=5)
            try:
                connection.request(method, path, body=body, headers={"Host": f"localhost:{ports[surface]}", **(headers or {})})
                response = connection.getresponse()
                data = response.read(1_048_577)
                assert len(data) <= 1_048_576 and response.status == status, f"Unexpected response for {case or surface}: {response.status}"
                assert response.getheader("cache-control") == "no-store"
                if case:
                    report["cases"].append({"case": case, "status": response.status, "body_sha256": hashlib.sha256(data).hexdigest()})
                return data, {key.lower(): value for key, value in response.getheaders()}
            finally:
                connection.close()

        deadline = time.monotonic() + 45
        while True:
            try:
                data, _ = request("health", "/healthz", 200)
                assert json.loads(data) == {"profile": "connect/2", "ready": True}
                break
            except (OSError, http.client.HTTPException):
                assert time.monotonic() < deadline, "Isolated host readiness deadline"
                time.sleep(0.2)
        report["cases"].append({"case": "minimal-health", "status": 200})
        auth = {"Authorization": "Bearer " + client["credential"]}
        admin = {"Authorization": "Bearer " + operator["credential"], "Content-Type": "application/json"}
        request("application", "/v2/discovery", 401, case="no-bearer")
        _, headers = request("application", "/v2/discovery", 200, {**auth, "Origin": ORIGIN}, case="authenticated-allowed-origin")
        assert headers.get("access-control-allow-origin") == ORIGIN
        for origin in ("https://foreign.fixture.invalid", "null"):
            request("application", "/v2/discovery", 403, {**auth, "Origin": origin}, case="refused-origin-" + origin)
        request("application", "/v2/discovery", 403, {**auth, "Host": "other.fixture.invalid"}, case="wrong-published-authority")
        request("application", "/v2/discovery", 403, {**auth, "X-Authenticated-User": "operator"}, case="forged-identity")
        request("application", "/v2/discovery", 403,
                {**auth, "X-Forwarded-Proto": "https", "X-Forwarded-Host": f"localhost:{ports['application']}", "X-Forwarded-For": "203.0.113.7"},
                case="untrusted-forwarding")
        request("administration", "/v2/identities", 401, body="malformed", method="POST", case="unauthenticated-admin-before-json")
        request("administration", "/v2/identities", 403, auth, "malformed", "POST", "non-admin-before-json")
        request("callbacks", "/webhooks/telnyx", 401, {"Content-Type": "application/json"}, "{}", "POST", "unsigned-callback")
        for surface in ("administration", "callbacks", "health"):
            request(surface, "/v2/discovery", 404, auth, case=surface + "-no-app-route")
        command = {"type": "identity", "operation_id": "isolated-host-identity", "alias": "isolated-host-identity", "kind": "human", "display_name": None}
        for surface in ("application", "callbacks", "health"):
            request(surface, "/v2/identities", 404, admin, json.dumps(command), "POST", surface + "-no-admin-route")
        for _ in range(2):
            data, _ = request("administration", "/v2/identities", 200, admin, json.dumps(command), "POST", "admin-original-id")
            assert "Ok" in json.loads(data)["result"]
        with sqlite3.connect(f"file:{installation / 'state.sqlite3'}?mode=ro", uri=True) as database:
            assert database.execute("SELECT count(*) FROM identities WHERE alias=?", (command["alias"],)).fetchone()[0] == 1
            credential = database.execute("SELECT id FROM credentials WHERE identity_id=? AND revoked=0", (client["identity_id"],)).fetchall()
            assert len(credential) == 1
        # TLS must refuse a real synthetic operator credential before HTTP decode.
        plain_command = {**command, "operation_id": "plaintext-must-not-create", "alias": "plaintext-must-not-create"}
        body = json.dumps(plain_command).encode()
        with socket.create_connection(("127.0.0.1", ports["administration"]), timeout=5) as plain:
            plain.sendall((f"POST /v2/identities HTTP/1.1\r\nHost: localhost:{ports['administration']}\r\nAuthorization: Bearer {operator['credential']}\r\nContent-Type: application/json\r\nContent-Length: {len(body)}\r\n\r\n").encode() + body)
            try:
                response = plain.recv(256)
            except ConnectionResetError:
                response = b""
            assert not response.startswith(b"HTTP/1.")
        report["cases"].append({"case": "plaintext-operator-refused-before-http", "passed": True})
        # Fresh SSL sockets require normal CA/name validation for every witness.
        def certificate_sha():
            with socket.create_connection(("127.0.0.1", ports["health"]), timeout=5) as raw:
                with context.wrap_socket(raw, server_hostname="localhost") as tls:
                    return hashlib.sha256(tls.getpeercert(binary_form=True)).hexdigest()

        first = certificate_sha()
        assert first == hashlib.sha256(ssl.PEM_cert_to_DER_cert((state / "leaf1.pem").read_text())).hexdigest()
        for source, destination in ((state / "leaf2.pem", certificate), (state / "leaf2.key", key)):
            replacement = destination.with_suffix(destination.suffix + ".new")
            shutil.copyfile(source, replacement)
            replacement.chmod(0o600)
            replacement.replace(destination)
        run(["docker", "kill", "--signal", "HUP", name])
        expected = hashlib.sha256(ssl.PEM_cert_to_DER_cert((state / "leaf2.pem").read_text())).hexdigest()
        deadline = time.monotonic() + 15
        while certificate_sha() != expected:
            assert time.monotonic() < deadline, "TLS replacement deadline"
            time.sleep(0.1)
        assert first != expected
        key.write_bytes(b"invalid isolated replacement")
        run(["docker", "kill", "--signal", "HUP", name])
        deadline = time.monotonic() + 15
        while True:
            private_logs = subprocess.check_output(["docker", "logs", name], stderr=subprocess.STDOUT, timeout=10)
            if b"TLS reload refused; last valid identity retained" in private_logs:
                break
            assert time.monotonic() < deadline, "Invalid reload diagnostic deadline"
            time.sleep(0.1)
        assert certificate_sha() == expected
        request("application", "/v2/discovery", 200, auth, case="last-good-https-after-invalid-reload")
        report["tls_certificates"] = {"root_pem_sha256": digest(state / "root.pem"), "initial_sha256": first,
                                      "rotated_sha256": expected, "invalid_reload_retained_last_good": True}
        for label, roots, hostname in (("unconfigured-root", ssl.create_default_context(), "localhost"),
                                       ("wrong-name", context, "wrong.fixture.invalid")):
            try:
                with socket.create_connection(("127.0.0.1", ports["health"]), timeout=5) as raw:
                    with roots.wrap_socket(raw, server_hostname=hostname):
                        raise AssertionError("Invalid TLS identity accepted")
            except ssl.SSLCertVerificationError:
                report["cases"].append({"case": label, "certificate_verification_refused": True})
        revoke = {"type": "revoke_credential", "operation_id": "isolated-https-revoke", "credential_id": credential[0][0]}
        data, _ = request("administration", f"/v2/credentials/{credential[0][0]}/revoke", 200, admin, json.dumps(revoke), "POST", "admin-revoke")
        assert "Ok" in json.loads(data)["result"]
        request("application", "/v2/discovery", 401, auth, case="current-revocation")
        with sqlite3.connect(f"file:{installation / 'state.sqlite3'}?mode=ro", uri=True) as database:
            plaintext_count = database.execute("SELECT count(*) FROM identities WHERE alias=?", (plain_command["alias"],)).fetchone()[0]
            replay_count = database.execute("SELECT count(*) FROM identities WHERE alias=?", (command["alias"],)).fetchone()[0]
            assert plaintext_count == 0 and replay_count == 1
        report["identity_effects"] = {"same_original_id_requests": 2, "original_id_rows": replay_count,
                                      "plaintext_operator_request_rows": plaintext_count}
        report["behavior_passed"] = True
    finally:
        cleanup_errors = []
        # A timed-out Docker client is not proof that its create/run failed.
        # Recover only these unique names carrying our fixture label.
        try:
            for owned in (name + "-init", name):
                found = subprocess.run(["docker", "inspect", owned], capture_output=True, timeout=10)
                if found.returncode == 0:
                    metadata = json.loads(found.stdout)[0]
                    assert metadata["Config"]["Labels"].get("connect.fixture") == "gate6-https"
                    if owned == name:
                        report["container_created"] = True
                    else:
                        run(["docker", "stop", "--time", "10", owned])
                        # Successful --rm may have already removed this init.
                        found = subprocess.run(["docker", "inspect", owned], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
                        if found.returncode == 0:
                            run(["docker", "rm", owned])
            found = subprocess.run(["docker", "network", "inspect", network], capture_output=True, timeout=10)
            if found.returncode == 0:
                metadata = json.loads(found.stdout)[0]
                assert metadata["Labels"].get("connect.fixture") == "gate6-https"
                report["network_created"] = True
        except Exception as error:
            cleanup_errors.append(type(error).__name__ + ": recovering uncertain owned fixture creation")
        if report["container_created"]:
            try:
                run(["docker", "stop", "--time", "15", name], timeout=25)
                metadata = json.loads(subprocess.check_output(["docker", "inspect", name], timeout=10))[0]
                report["shutdown"] = {k: metadata["State"][k] for k in ("Running", "ExitCode", "OOMKilled", "Error")}
                private_logs = subprocess.check_output(["docker", "logs", name], stderr=subprocess.STDOUT, timeout=10)
                (output / "server.log").write_bytes(private_logs)
                report["joined_stop_observed"] = b"stopped" in private_logs.splitlines()
                report["server_log_sha256"] = digest(output / "server.log")
                report["panic_observed"] = b"panicked at" in private_logs
                report["credential_bytes_in_server_log"] = any(c["credential"].encode() in private_logs for c in (operator, client))
            except Exception as error:
                cleanup_errors.append(type(error).__name__ + ": inspecting/stopping owned container")
            try:
                run(["docker", "rm", name])
                absent = subprocess.run(["docker", "inspect", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
                report["container_removed"] = absent.returncode != 0
            except Exception as error:
                cleanup_errors.append(type(error).__name__ + ": removing owned container")
        if report["network_created"]:
            try:
                run(["docker", "network", "rm", network])
                absent = subprocess.run(["docker", "network", "inspect", network], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
                report["network_removed"] = absent.returncode != 0
            except Exception as error:
                cleanup_errors.append(type(error).__name__ + ": removing owned network")
        log.close()
        report["cleanup_errors"] = cleanup_errors
        report["seconds"] = time.monotonic() - start
        report["binary_unchanged"] = digest(binary) == args.binary_sha256
        report["runner_unchanged"] = digest(__file__) == report["runner_sha256"]
        report["passed"] = (report.get("behavior_passed", False) and report.get("joined_stop_observed", False)
                            and report.get("shutdown", {}).get("ExitCode") == 0
                            and report.get("shutdown", {}).get("Running") is False
                            and report.get("shutdown", {}).get("OOMKilled") is False
                            and report.get("shutdown", {}).get("Error") == ""
                            and not report.get("panic_observed", True) and not report.get("credential_bytes_in_server_log", True)
                            and report.get("container_removed", False) and report.get("network_removed", False) and not cleanup_errors
                            and report["binary_unchanged"] and report["runner_unchanged"])
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
