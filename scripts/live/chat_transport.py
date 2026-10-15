"""One bounded provider request in a disposable process; no retries or redirects."""
import http.client
import json
import os
from pathlib import Path
import re
import ssl
import subprocess
import sys
import time
from urllib.parse import urlsplit

from voice_runtime import save

LIMIT = 1024 * 1024


def origin(value):
    """Accept a plain HTTPS origin or an explicit loopback simulator origin."""
    url = urlsplit(value)
    if (url.username or url.password or url.query or url.fragment or url.path not in ("", "/")
            or not url.hostname or (url.scheme != "https" and not (
                url.scheme == "http" and url.hostname in ("127.0.0.1", "localhost", "::1")))):
        raise ValueError("Expected HTTPS or explicit loopback HTTP API origin")
    _ = url.port
    return url


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", value):
        raise ValueError("Invalid provider resource identifier")
    return value


def request(base, method, route, payload, directory, name):
    """Keep the key out of argv/files; a killed POST remains uncertain."""
    origin(base)
    key = os.environ.get("VAPI_PRIVATE_KEY", "").strip()
    if not key:
        raise ValueError("VAPI_PRIVATE_KEY is required")
    path = directory / (name + "-request.json")
    result_path = directory / (name + "-response.json")
    if path.exists() or result_path.exists():
        raise ValueError("Retain existing request evidence; do not dispatch again")
    save(path, {"base": base, "method": method, "route": route, "payload": payload})
    environment = {k: os.environ[k] for k in ("PATH", "HOME", "TMPDIR", "LANG") if k in os.environ}
    environment.update(VAPI_PRIVATE_KEY=key, PYTHONDONTWRITEBYTECODE="1")
    # The worker only performs HTTP and writes its result; it spawns no children.
    completed = subprocess.run([sys.executable, str(Path(__file__).resolve()), str(path)],
                               env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, timeout=40, check=False)
    if completed.returncode != 0 or not result_path.exists():
        raise RuntimeError("Provider request incomplete; retain original intent")
    result = json.loads(result_path.read_text())
    if result.get("status") != "received":
        raise RuntimeError("Provider request incomplete; retain original intent")
    return result["body"]


def worker(path):
    """HTTP time/size bounds plus the parent's absolute process deadline."""
    os.umask(0o077)
    result = {"status": "failed"}
    connection = None
    http_status = None
    try:
        with path.open("rb") as stream:
            raw = stream.read(LIMIT + 1)
        if len(raw) > LIMIT:
            raise ValueError("Oversized request")
        value = json.loads(raw)
        url = origin(value["base"])
        method, route = value["method"], value["route"]
        if not ((method == "POST" and route == "/chat") or (method == "GET" and re.fullmatch(
                r"/(chat|session)/[A-Za-z0-9_-]{1,128}", route))):
            raise ValueError("Unsupported rehearsal request")
        body = json.dumps(value["payload"]).encode() if value["payload"] is not None else None
        deadline = time.monotonic() + 30
        connection = (http.client.HTTPSConnection(url.hostname, url.port, timeout=30,
                      context=ssl.create_default_context()) if url.scheme == "https" else
                      http.client.HTTPConnection(url.hostname, url.port, timeout=30))
        connection.request(method, route, body=body, headers={
            "Authorization": "Bearer " + os.environ["VAPI_PRIVATE_KEY"],
            "User-Agent": "Connect/0.1.0",
            "Content-Type": "application/json", "Accept": "application/json"})
        response = connection.getresponse()
        http_status = response.status
        if not 200 <= response.status < 300:
            raise ValueError("Provider returned a non-success status")
        data = bytearray()
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Provider response deadline exceeded")
            if connection.sock is not None:
                connection.sock.settimeout(remaining)
            part = response.read1(min(16384, LIMIT + 1 - len(data)))
            if not part:
                break
            data.extend(part)
            if len(data) > LIMIT:
                raise ValueError("Oversized provider response")
        decoded = json.loads(data)
        if not isinstance(decoded, dict):
            raise ValueError("Expected provider object")
        result = {"status": "received", "body": decoded}
    except Exception as error:
        result = {"status": "failed", "error_class": type(error).__name__, "http_status": http_status}
    finally:
        if connection is not None:
            connection.close()
    save(path.with_name(path.name.replace("-request.json", "-response.json")), result)
    return 0 if result["status"] == "received" else 1


if __name__ == "__main__":
    raise SystemExit(worker(Path(sys.argv[1])))
