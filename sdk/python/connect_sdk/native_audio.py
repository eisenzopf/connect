"""Bounded binary helper for native UCTP/QUIC PCMU audio."""
import os
import select
import subprocess
import threading
import time
from .errors import ConnectError
from .protocol import timeout_value

_MEDIA_SUBSCRIBE = 0x01
_MEDIA_AUDIO = 0x02
_MEDIA_CLOSE = 0x03
_MEDIA_READY = 0x81
_MEDIA_RECEIVED = 0x82
_MEDIA_ENDED = 0x83
_MEDIA_ERROR = 0xFF
_MAX_MEDIA_FRAME = 4096


class NativeAudioClient:
    """Raw PCMU audio over a separate bounded binary helper channel.

    Helper diagnostics inherit stderr by default. Supply a writable file via
    ``stderr`` to capture them without creating an unread internal pipe.
    """

    def __init__(self, config, conversation_id, session_id, remote_identity_id,
                 executable="connect", timeout=15, stderr=None):
        self._timeout = timeout_value(timeout)
        self._read_lock = threading.Lock()
        self._write_lock = threading.Lock()
        self._closed = False
        self._ended = False
        try:
            self._process = subprocess.Popen([
                executable, "media-stdio", "--config", str(config),
                "--conversation-id", conversation_id,
                "--session-id", session_id,
                "--remote-identity-id", remote_identity_id,
            ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, bufsize=0)
        except OSError as error:
            raise ConnectError("transport-uncertain", "Could not start Connect media helper") from error
        os.set_blocking(self._process.stdin.fileno(), False)
        os.set_blocking(self._process.stdout.fileno(), False)
        kind, payload = self._read_frame(self._timeout)
        if kind != _MEDIA_READY:
            self._stop()
            raise ConnectError("protocol", "Media helper did not become ready")
        try:
            self.publisher_stream_id = payload.decode("utf-8")
        except UnicodeError as error:
            self._stop()
            raise ConnectError("protocol", "Invalid media stream identifier") from error

    def _exact(self, count, deadline):
        value = bytearray()
        while len(value) < count:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError()
            ready, _, _ = select.select([self._process.stdout], [], [], remaining)
            if not ready:
                raise TimeoutError()
            chunk = os.read(self._process.stdout.fileno(), count - len(value))
            if not chunk:
                raise OSError("Media helper closed")
            value.extend(chunk)
        return bytes(value)

    def _read_frame(self, timeout):
        with self._read_lock:
            if self._closed:
                raise ConnectError("media-ended" if self._ended else "transport-uncertain", "Media helper is closed")
            deadline = time.monotonic() + timeout
            try:
                header = self._exact(5, deadline)
                length = int.from_bytes(header[1:], "big")
                if length > _MAX_MEDIA_FRAME:
                    raise ConnectError("protocol", "Media helper frame exceeds bounds")
                payload = self._exact(length, deadline)
                if header[0] == _MEDIA_ENDED and not payload:
                    self._ended = True
                    raise ConnectError("media-ended", "The remote media leg ended")
                if header[0] == _MEDIA_ERROR:
                    raise ConnectError("protocol", payload.decode("utf-8", "replace"))
                return header[0], payload
            except ConnectError:
                self._stop()
                raise
            except (OSError, TimeoutError) as error:
                self._stop()
                raise ConnectError("transport-uncertain", "Media helper failed or timed out") from error

    def _write_frame(self, kind, payload=b""):
        if self._closed:
            raise ConnectError("media-ended" if self._ended else "transport-uncertain", "Media helper is closed")
        if len(payload) > _MAX_MEDIA_FRAME:
            raise ConnectError("invalid-request", "Media frame exceeds bounds")
        frame = bytes([kind]) + len(payload).to_bytes(4, "big") + payload
        with self._write_lock:
            deadline = time.monotonic() + self._timeout
            remaining = memoryview(frame)
            try:
                while remaining:
                    wait = deadline - time.monotonic()
                    if wait <= 0:
                        raise TimeoutError()
                    _, ready, _ = select.select([], [self._process.stdin], [], wait)
                    if not ready:
                        raise TimeoutError()
                    count = os.write(self._process.stdin.fileno(), remaining)
                    remaining = remaining[count:]
            except (OSError, TimeoutError) as error:
                self._stop()
                raise ConnectError("transport-uncertain", "Media helper write failed") from error

    def subscribe(self, publisher_stream_id):
        payload = publisher_stream_id.encode("utf-8")
        if not payload or len(payload) > 255:
            raise ConnectError("invalid-request", "Invalid publisher stream identifier")
        self._write_frame(_MEDIA_SUBSCRIBE, payload)

    def send_pcmu(self, payload):
        payload = bytes(payload)
        if len(payload) != 160:
            raise ConnectError("invalid-request", "PCMU frames must contain 160 bytes")
        self._write_frame(_MEDIA_AUDIO, payload)

    def receive(self, timeout=None):
        kind, payload = self._read_frame(self._timeout if timeout is None else timeout_value(timeout))
        if kind != _MEDIA_RECEIVED or len(payload) != 160:
            self._stop()
            raise ConnectError("protocol", "Invalid received PCMU frame")
        return payload

    def _stop(self):
        if self._closed:
            return
        self._closed = True
        if self._process.poll() is None:
            self._process.kill()
        self._process.wait()
        self._process.stdin.close()
        self._process.stdout.close()
        if self._process.stderr is not None:
            self._process.stderr.close()

    def close(self):
        if self._closed:
            return
        try:
            self._write_frame(_MEDIA_CLOSE)
            self._process.stdin.close()
            self._process.wait(timeout=3)
            self._closed = True
            self._process.stdout.close()
            if self._process.stderr is not None:
                self._process.stderr.close()
        except (OSError, subprocess.TimeoutExpired):
            self._stop()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
