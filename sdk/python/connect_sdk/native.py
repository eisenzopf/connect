"""Native QUIC through a persistent Rust helper; one bounded exchange at a time."""
import json
import os
import select
import subprocess
import threading
import time
import uuid
from .errors import ConnectError
from .operations import Operations
from .protocol import MAX_REQUEST, MAX_RESPONSE, decode_response, expected_kind, timeout_value

class NativeClient(Operations):
    def __init__(self, config, executable="connect", timeout=15):
        self._timeout = timeout_value(timeout)
        self._lock = threading.Lock()
        self._closed = False
        try:
            self._process = subprocess.Popen(
                [executable, "stdio", "--config", str(config)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0,
            )
        except OSError as error:
            raise ConnectError("transport-uncertain", "Could not start Connect helper") from error
        os.set_blocking(self._process.stdin.fileno(), False)
        os.set_blocking(self._process.stdout.fileno(), False)

    def _wait(self, stream, writing, deadline):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError()
        readers, writers, _ = select.select([] if writing else [stream], [stream] if writing else [], [], remaining)
        if not (writers if writing else readers):
            raise TimeoutError()

    def request(self, operation):
        expected = expected_kind(operation)
        exchange_id = str(uuid.uuid4())
        line = json.dumps({"id": exchange_id, "operation": operation}).encode() + b"\n"
        if len(line) > MAX_REQUEST:
            raise ConnectError("invalid-request", "Operation exceeds 64 KiB")
        with self._lock:
            if self._closed:
                raise ConnectError("transport-uncertain", "Client closed; preserve logical request ID")
            deadline = time.monotonic() + self._timeout
            try:
                remaining = memoryview(line)
                while remaining:
                    self._wait(self._process.stdin, True, deadline)
                    try:
                        count = os.write(self._process.stdin.fileno(), remaining)
                        remaining = remaining[count:]
                    except BlockingIOError:
                        continue
                response = bytearray()
                while b"\n" not in response:
                    self._wait(self._process.stdout, False, deadline)
                    try:
                        chunk = os.read(self._process.stdout.fileno(), min(65536, MAX_RESPONSE + 1 - len(response)))
                    except BlockingIOError:
                        continue
                    if not chunk:
                        raise OSError("Helper closed")
                    response.extend(chunk)
                    if len(response) > MAX_RESPONSE:
                        raise ConnectError("protocol", "Response exceeds bounds")
                raw, extra = response.split(b"\n", 1)
                if extra:
                    raise ConnectError("protocol", "Unexpected extra helper response")
                try:
                    value = json.loads(raw)
                    if not isinstance(value, dict) or value.get("id") != exchange_id or "response" not in value:
                        raise ValueError()
                except (ValueError, TypeError, UnicodeError, RecursionError) as error:
                    raise ConnectError("protocol", "Malformed or mismatched helper response") from error
                return decode_response(json.dumps(value["response"]), expected)
            except ConnectError as error:
                if error.code == "protocol":
                    self._stop()
                raise
            except (OSError, TimeoutError, ValueError) as error:
                self._stop()
                raise ConnectError("transport-uncertain", "Helper failed or deadline expired; preserve logical request ID") from error

    def _stop(self):
        self._closed = True
        if self._process.poll() is None:
            self._process.kill()
        self._process.wait()
        self._process.stdin.close()
        self._process.stdout.close()

    def close(self):
        with self._lock:
            if self._closed:
                return
            self._closed = True
            self._process.stdin.close()
            try:
                self._process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait()
            self._process.stdout.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
