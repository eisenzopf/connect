"""Connect clients. Caller-owned mutation IDs are preserved across uncertainty."""
import json
import subprocess
import select
import threading
import uuid
import urllib.request
import urllib.error
from urllib.parse import urlsplit


class ConnectError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def _outcome(response):
    if response.get("profile") != "connect-control/1":
        raise ConnectError("protocol", "Unexpected Connect profile")
    result = response["result"]
    if "Err" in result:
        raise ConnectError(result["Err"]["code"], result["Err"]["message"])
    return result["Ok"]


class Operations:
    def resolve(self, address):
        return self.request({"op": "resolve", "address": address})["data"]

    def create(self, request_id, participants):
        return self.request({"op": "create", "request_id": request_id,
                             "participants": participants})["data"]

    def send(self, request_id, conversation_id, body):
        return self.request({"op": "send", "request_id": request_id,
                             "conversation_id": conversation_id, "body": body})["data"]

    def history(self, conversation_id, after=None, limit=100):
        return self.request({"op": "history", "conversation_id": conversation_id,
                             "after": after, "limit": limit})["data"]

    def inbox(self, cursor=None, limit=100, wait_ms=0):
        return self.request({"op": "inbox", "cursor": cursor,
                             "limit": limit, "wait_ms": wait_ms})["data"]

    def request_status(self, request_id):
        return self.request({"op": "request_status", "request_id": request_id})["data"]

    def provider_status(self, conversation_id):
        return self.request({"op": "provider_status", "conversation_id": conversation_id})["data"]

    def close_conversation(self, request_id, conversation_id):
        return self.request({"op": "close", "request_id": request_id,
                             "conversation_id": conversation_id})["data"]

    def call_start(self, request_id, conversation_id, route_id, destination):
        return self.request({"op": "call_start", "request_id": request_id,
                             "conversation_id": conversation_id, "route_id": route_id,
                             "destination": destination})["data"]

    def call_status(self, conversation_id, call_id):
        return self.request({"op": "call_status", "conversation_id": conversation_id,
                             "call_id": call_id})["data"]

    def call_end(self, request_id, conversation_id, call_id):
        return self.request({"op": "call_end", "request_id": request_id,
                             "conversation_id": conversation_id, "call_id": call_id})["data"]

    def health(self):
        return self.request({"op": "health"})["data"]


class NativeClient(Operations):
    """Persistent JSON-lines subprocess; Connect's Rust SDK owns verified QUIC.

    One operation at a time. Server/client config bounds request time. stderr
    goes to the caller's stderr without copying application bodies to logs.
    """
    def __init__(self, config, executable="connect", timeout=15):
        self._timeout = timeout
        self._lock = threading.Lock()
        self._process = subprocess.Popen([executable, "stdio", "--config", str(config)],
                                         stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def request(self, operation):
        with self._lock:
            request_id = str(uuid.uuid4())  # exchange ID, never a mutation ID
            line = json.dumps({"id": request_id, "operation": operation}).encode() + b"\n"
            if len(line) > 65537:
                raise ConnectError("invalid-request", "Operation exceeds 64 KiB")
            try:
                self._process.stdin.write(line)
                self._process.stdin.flush()
                ready, _, _ = select.select([self._process.stdout], [], [], self._timeout)
                if not ready:
                    self._process.kill()
                    self._process.wait()
                    raise ConnectError("transport-uncertain", "Helper deadline expired; preserve logical request ID")
                raw = self._process.stdout.readline(600000)
                if not raw.endswith(b"\n"):
                    raise ConnectError("transport-uncertain", "Helper closed; preserve logical request ID")
                value = json.loads(raw)
                if value["id"] != request_id:
                    raise ConnectError("protocol", "Exchange ID mismatch")
                return _outcome(value["response"])
            except (BrokenPipeError, OSError) as error:
                raise ConnectError("transport-uncertain", "Helper failed; preserve logical request ID") from error

    def close(self):
        if self._process.stdin:
            self._process.stdin.close()
        try:
            self._process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self._process.kill()
            self._process.wait()
        if self._process.stdout:
            self._process.stdout.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


class HttpClient(Operations):
    """Local HTTP/JSON facade; this transport is HTTP, not native QUIC."""
    def __init__(self, url, credential, timeout=12):
        parsed = urlsplit(url)
        if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "::1", "localhost") or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError("Use a loopback HTTP URL")
        self._url = url.rstrip("/") + "/v1/operations"
        self._credential = credential
        self._timeout = timeout
        self._opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())

    def request(self, operation):
        request = urllib.request.Request(self._url, data=json.dumps(operation).encode(),
                                        headers={"Content-Type": "application/json",
                                                 "Authorization": "Bearer " + self._credential})
        try:
            with self._opener.open(request, timeout=self._timeout) as response:
                return _outcome(json.load(response))
        except urllib.error.HTTPError as error:
            try:
                return _outcome(json.load(error))
            except (ValueError, KeyError) as parse_error:
                raise ConnectError("http-error", "HTTP request rejected") from parse_error
        except (OSError, TimeoutError) as error:
            raise ConnectError("transport-uncertain", "HTTP failed; preserve logical request ID") from error
