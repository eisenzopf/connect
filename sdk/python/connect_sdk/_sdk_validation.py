"""Bounded synchronous local Rust validation; no config, credentials or network."""
from __future__ import annotations

import os
import selectors
import subprocess
import time

from .sdk_types import SdkError


def _validate(executable: str, command: str, value: bytes) -> bytes:
    """Own the original one-shot child, including bounded pipes and forced join."""
    child = None
    expires = time.monotonic() + 2
    try:
        child = subprocess.Popen([executable, command], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        assert child.stdin is not None and child.stdout is not None
        output = bytearray()
        sent = 0
        with selectors.DefaultSelector() as selector:
            for stream, event in ((child.stdin, selectors.EVENT_WRITE), (child.stdout, selectors.EVENT_READ)):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, event)
            while selector.get_map():
                remaining = expires - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError()
                for key, _ in selector.select(remaining):
                    if key.fileobj is child.stdin:
                        try:
                            sent += os.write(child.stdin.fileno(), value[sent:sent + 65536])
                        except BlockingIOError:
                            continue
                        if sent == len(value):
                            selector.unregister(child.stdin)
                            child.stdin.close()
                    else:
                        try:
                            chunk = os.read(child.stdout.fileno(), 16385 - len(output))
                        except BlockingIOError:
                            continue
                        if not chunk:
                            selector.unregister(child.stdout)
                        output.extend(chunk)
                        if len(output) > 16384:
                            raise ValueError()
        remaining = expires - time.monotonic()
        if remaining <= 0 or child.wait(timeout=remaining) != 0:
            raise ValueError()
        return bytes(output)
    except (OSError, ValueError, TimeoutError, subprocess.TimeoutExpired):
        raise SdkError("configuration", "not-submitted") from None
    finally:
        if child is not None:
            if child.poll() is None:
                try:
                    child.kill()
                except ProcessLookupError:
                    pass
            try:
                child.wait(timeout=1)
            except subprocess.TimeoutExpired:
                raise SdkError("cleanup-incomplete", "not-submitted") from None
            finally:
                for pipe in (child.stdin, child.stdout):
                    if pipe is not None:
                        pipe.close()
