#!/usr/bin/env python3
"""Bounded real TCP peer. Protected credentials arrive only on stdin."""
import base64
import json
import socket
import ssl
import sys

config = json.loads(sys.stdin.buffer.readline())
peer = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
peer.settimeout(10)
peer.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
no_options = sys.platform == 'darwin'
if no_options:
    # Public Darwin TCP_NOOPT (0x8), verified against the local SDK tcp.h and
    # apple-oss-distributions/xnu bsd/netinet/tcp_usrreq.c. Disable scaling so
    # macOS loopback buffer resizing cannot absorb this entire record fixture.
    peer.setsockopt(socket.IPPROTO_TCP, 8, 1)
peer.connect(('127.0.0.1', config['port']))
peer.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
if 'trust_der' in config:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.load_verify_locations(cadata=ssl.DER_cert_to_PEM_cert(base64.b64decode(config['trust_der'])))
    context.set_alpn_protocols(['http/1.1'])
    peer = context.wrap_socket(peer, server_hostname=config['server_name'])
request = config['request'].encode('utf-8')
assert len(request) <= 65536
peer.sendall(request)
headers = bytearray()
while not headers.endswith(b'\r\n\r\n'):
    assert len(headers) < 8192
    chunk = peer.recv(1)
    assert chunk
    headers.extend(chunk)
# Loopback MSS setup can resize the buffer on the first received packet.
# Reset after headers, before opening the fixture control channel.
peer.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
print(json.dumps({'headers': headers.decode('ascii'), 'no_options': no_options}), flush=True)
# The real socket stays unread until the Rust test has queued its actual
# authority writers or driven expiry/shutdown. No network simulator is used.
command = json.loads(sys.stdin.buffer.readline())
assert command == 'read'
peer.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 256 * 1024)
body = bytearray()
error = None
try:
    while True:
        chunk = peer.recv(65536)
        if not chunk:
            break
        body.extend(chunk)
        assert len(body) <= 2 * 1024 * 1024
except OSError as exc:
    error = exc.errno
peer.close()
print(json.dumps({'body': base64.b64encode(body).decode('ascii'), 'error': error}), flush=True)
