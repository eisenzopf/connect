"""Loopback HTTP/JSON with bounded bodies, no proxies, redirects or mutation retries."""
import http.client
import json
import socket
import threading
import time
from urllib.parse import urlsplit
from .errors import ConnectError
from .operations import Operations
from .protocol import MAX_REQUEST, MAX_RESPONSE, decode_response, expected_kind, timeout_value


class HttpClient(Operations):
    def __init__(self, url, credential, timeout=12):
        parsed = urlsplit(url)
        if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "::1", "localhost") or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError("Use a loopback HTTP URL")
        self._host = parsed.hostname
        self._port = parsed.port or 80
        self._path = parsed.path.rstrip("/") + "/v1/operations"
        self._credential = credential
        self._timeout = timeout_value(timeout)

    def request(self, operation):
        expected = expected_kind(operation)
        body = json.dumps(operation).encode()
        if len(body) > MAX_REQUEST - 1:
            raise ConnectError("invalid-request", "Operation exceeds 64 KiB")
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
            connection.request("POST", self._path, body, {"Content-Type": "application/json", "Authorization": "Bearer " + self._credential})
            response = connection.getresponse()
            if 300 <= response.status < 400:
                raise ConnectError("protocol", "HTTP redirect refused")
            raw = response.read(MAX_RESPONSE + 1)
            if expired.is_set():
                raise TimeoutError()
            if len(raw) > MAX_RESPONSE:
                raise ConnectError("protocol", "Response exceeds bounds")
            return decode_response(raw, expected)
        except (OSError, TimeoutError, http.client.HTTPException) as error:
            raise ConnectError("transport-uncertain", "HTTP failed; preserve logical request ID") from error
        finally:
            if timer is not None:
                timer.cancel()
            if response is not None:
                response.close()
            connection.close()
