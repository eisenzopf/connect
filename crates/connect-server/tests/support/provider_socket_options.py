#!/usr/bin/env python3
"""Constrain one inherited, bound loopback listener before it can accept IO."""
import json
import socket
import sys

assert sys.platform == "darwin"
with socket.socket(fileno=0) as listener:
    assert listener.family == socket.AF_INET
    assert listener.type == socket.SOCK_STREAM
    assert listener.getsockname()[0] == "127.0.0.1"
    # Darwin TCP_NOOPT disables receive-window scaling on accepted peers.
    # Same explicit fixture option as http_record_peer.py; no production
    # connection or descriptor is inherited by this short-lived helper.
    listener.setsockopt(socket.IPPROTO_TCP, 8, 1)
    # Darwin returns the enabled flag bit (8), rather than normalized True.
    assert listener.getsockopt(socket.IPPROTO_TCP, 8) != 0
print(json.dumps({"no_options": True}), flush=True)
