"""Loopback HTTP/JSON with bounded bodies, no proxies, redirects or mutation retries."""
import http.client
import json
import socket
import threading
import time
from urllib.parse import quote, urlencode, urlsplit
from .errors import ConnectError
from .operations import Operations
from .task_operations import exchange_timeout
from .protocol import MAX_REQUEST, MAX_RESPONSE, decode_response, expected_kind, timeout_value


class HttpClient(Operations):
    def __init__(self, url, credential, timeout=12):
        parsed = urlsplit(url)
        if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "::1", "localhost") or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError("Use a loopback HTTP URL")
        self._host = parsed.hostname
        self._port = parsed.port or 80
        self._base_path = parsed.path.rstrip("/")
        self._path = parsed.path.rstrip("/") + "/v1/operations"
        self._credential = credential
        self._timeout = timeout_value(timeout)

    def request(self, operation, *, task_binding=None):
        self._require_task_profile(operation, task_binding)
        expected = expected_kind(operation)
        body = json.dumps(operation).encode()
        if len(body) > MAX_REQUEST - 1:
            raise ConnectError("invalid-request", "Operation exceeds 64 KiB")
        timeout = exchange_timeout(self._timeout, operation)
        connection = http.client.HTTPConnection(self._host, self._port, timeout=timeout)
        timer = None
        response = None
        deadline = time.monotonic() + timeout
        expired = threading.Event()
        try:
            connection.connect()
            sock = connection.sock
            def expire():
                expired.set()
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            timer = threading.Timer(max(0, deadline - time.monotonic()), expire)
            timer.daemon = True
            timer.start()
            connection.request("POST", self._path, body, {"Content-Type": "application/json", "Authorization": "Bearer " + self._credential})
            response = connection.getresponse()
            if 300 <= response.status < 400:
                raise ConnectError("protocol", "HTTP redirect refused")
            raw = response.read(MAX_RESPONSE + 1)
            if expired.is_set():
                raise TimeoutError()
            if len(raw) > MAX_RESPONSE:
                raise ConnectError("protocol", "Response exceeds bounds")
            return self._observe_outcome(decode_response(raw, expected))
        except (OSError, TimeoutError, http.client.HTTPException) as error:
            raise ConnectError("transport-uncertain", "HTTP failed; preserve logical request ID") from error
        finally:
            if timer is not None:
                timer.cancel()
            if response is not None:
                response.close()
            connection.close()

    def upload_artifact_chunk(self, conversation_id, artifact_id, request_id, offset, data):
        """Upload one raw binary chunk and return its validated artifact record."""
        if not isinstance(data, (bytes, bytearray, memoryview)) or not 0 < len(data) <= 1024 * 1024:
            raise ConnectError("invalid-request", "Artifact chunk is outside SDK bounds")
        query = urlencode({"request_id": request_id, "offset": offset})
        path = self._artifact_path(conversation_id, artifact_id) + "?" + query
        raw, _ = self._raw_request("PUT", path, bytes(data), MAX_RESPONSE)
        return decode_response(raw, "artifact")

    def download_artifact_chunk(self, conversation_id, artifact_id, offset, limit):
        """Download one raw binary chunk with its verified offset and EOF marker."""
        if isinstance(limit, bool) or not isinstance(limit, int) or not 0 < limit <= 1024 * 1024:
            raise ConnectError("invalid-request", "Artifact chunk limit is outside SDK bounds")
        query = urlencode({"offset": offset, "limit": limit})
        path = self._artifact_path(conversation_id, artifact_id) + "?" + query
        raw, headers = self._raw_request("GET", path, None, limit)
        try:
            response_offset = int(headers["x-connect-offset"])
            eof = {"true": True, "false": False}[headers["x-connect-eof"]]
            if response_offset != offset or headers.get_content_type() != "application/octet-stream":
                raise ValueError()
        except (KeyError, TypeError, ValueError) as error:
            raise ConnectError("protocol", "Invalid artifact byte response") from error
        return {"data": raw, "offset": response_offset, "eof": eof}

    def _artifact_path(self, conversation_id, artifact_id):
        return self._base_path.rstrip("/") + "/v1/conversations/{}/artifacts/{}".format(
            quote(conversation_id, safe=""), quote(artifact_id, safe="")
        )

    def _raw_request(self, method, path, body, maximum):
        connection = http.client.HTTPConnection(self._host, self._port, timeout=self._timeout)
        timer = None
        response = None
        deadline = time.monotonic() + self._timeout
        expired = threading.Event()
        try:
            connection.connect()
            sock = connection.sock
            def expire():
                expired.set()
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            timer = threading.Timer(max(0, deadline - time.monotonic()), expire)
            timer.daemon = True
            timer.start()
            connection.request(method, path, body, {
                "Content-Type": "application/octet-stream",
                "Authorization": "Bearer " + self._credential,
            })
            response = connection.getresponse()
            if 300 <= response.status < 400:
                raise ConnectError("protocol", "HTTP redirect refused")
            raw = response.read(max(maximum, MAX_RESPONSE if response.status != 200 else 0) + 1)
            if expired.is_set():
                raise TimeoutError()
            bound = maximum if response.status == 200 else MAX_RESPONSE
            if len(raw) > bound:
                raise ConnectError("protocol", "Response exceeds bounds")
            if response.status != 200:
                decode_response(raw, "artifact_chunk")
                raise ConnectError("protocol", "Unexpected HTTP artifact response")
            return raw, response.headers
        except ConnectError:
            raise
        except (OSError, TimeoutError, http.client.HTTPException) as error:
            raise ConnectError("transport-uncertain", "HTTP failed; preserve logical request ID") from error
        finally:
            if timer is not None:
                timer.cancel()
            if response is not None:
                response.close()
            connection.close()
