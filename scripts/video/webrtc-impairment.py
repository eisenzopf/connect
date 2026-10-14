"""Owned loopback WebSocket/ICE relay for actual STUN/DTLS/SRTP packet faults.

No SDP, ICE secrets or credentials are exported. Candidate ports must belong to
the selected disposable server PID; targets always use loopback.
"""
import asyncio
import base64
import hashlib
import json
import os
import re
import struct
import subprocess
import time
from urllib.parse import urlparse


async def frame(reader):
    header = await reader.readexactly(2)
    if header[0] & 0x70 or not header[0] & 0x80: raise ValueError('Unsupported fragmented/compressed qualification WebSocket frame')
    size = header[1] & 127
    if size == 126: size = struct.unpack('!H', await reader.readexactly(2))[0]
    if size == 127: size = struct.unpack('!Q', await reader.readexactly(8))[0]
    if size > 65536: raise ValueError('Qualification signaling frame exceeds bound')
    mask = await reader.readexactly(4) if header[1] & 128 else None
    data = await reader.readexactly(size)
    if mask: data = bytes(value ^ mask[index % 4] for index, value in enumerate(data))
    return header[0] & 15, data


def encode(opcode, data, masked):
    if len(data) > 65536: raise ValueError('Qualification signaling frame exceeds bound')
    flag = 128 if masked else 0
    if len(data) < 126: header = bytes([128 | opcode, flag | len(data)])
    elif len(data) <= 65535: header = bytes([128 | opcode, flag | 126]) + struct.pack('!H', len(data))
    else: header = bytes([128 | opcode, flag | 127]) + struct.pack('!Q', len(data))
    if not masked: return header + data
    mask = os.urandom(4)
    return header + mask + bytes(value ^ mask[index % 4] for index, value in enumerate(data))


def owned_udp_ports(pid):
    result = subprocess.run(['lsof', '-nP', '-a', '-p', str(pid), '-iUDP', '-Fn'],
        capture_output=True, text=True, timeout=3)
    if result.returncode: raise RuntimeError('Cannot verify selected server UDP ownership')
    return {int(match.group(1)) for line in result.stdout.splitlines()
        if line.startswith('n') for match in [re.search(r':(\d+)(?:->|$)', line)] if match}


class WebRtcImpairment:
    def __init__(self, signaling, server_pid, datagram_class, seconds):
        selected = urlparse(signaling)
        if selected.scheme != 'ws' or selected.hostname != '127.0.0.1' or selected.query or selected.fragment:
            raise ValueError('Select an isolated loopback signaling server')
        self.selected = selected; self.server_pid = server_pid; self.datagram_class = datagram_class
        self.seconds = seconds; self.relays = {}; self.connections = set(); self.errors = []; self.tasks = set()
        self.relay_peers = {}; self.peer_count = 0

    async def __aenter__(self):
        self.start_unix = time.time()
        self.server = await asyncio.start_server(self.accept, '127.0.0.1', 0, limit=65536)
        self.url = 'ws://127.0.0.1:' + str(self.server.sockets[0].getsockname()[1])
        return self

    def bursts(self):
        return [{'start_unix_seconds': self.start_unix + self.seconds * at,
            'end_unix_seconds': self.start_unix + self.seconds * at + .25, 'duration_seconds': .25}
            for at in [.25, .6]]

    async def rewrite(self, data, peer=0):
        message = json.loads(data)
        if not isinstance(message, dict) or not isinstance(message.get('sdp'), str): return data
        lines = message['sdp'].splitlines(); ports = owned_udp_ports(self.server_pid); rewritten = []
        for line in lines:
            if line.startswith('a=candidate:'):
                parts = line.split()
                if len(parts) < 8 or parts[2].lower() != 'udp' or parts[6:8] != ['typ', 'host']:
                    raise ValueError('Qualification requires selected server host UDP ICE candidates')
                port = int(parts[5])
                if port not in ports: raise ValueError('ICE candidate port does not belong to selected server')
                if port not in self.relays:
                    if len(self.relays) >= 8: raise ValueError('Qualification candidate relay count exceeds bound')
                    impaired = peer == 0
                    relay = self.datagram_class(('127.0.0.1', port), seed=515 + len(self.relays),
                        drop_probability=.03 if impaired else 0, maximum_delay=.1 if impaired else 0)
                    await relay.__aenter__()
                    if impaired: relay.bursts = [(b['start_unix_seconds'] - relay.start_unix, b['duration_seconds']) for b in self.bursts()]
                    self.relays[port] = relay
                    self.relay_peers[port] = peer
                elif self.relay_peers[port] != peer:
                    raise ValueError('Shared server ICE socket cannot establish the selected single-hop impairment profile')
                parts[4] = '127.0.0.1'; parts[5] = str(self.relays[port].address[1]); line = ' '.join(parts)
            rewritten.append(line)
        message['sdp'] = '\r\n'.join(rewritten) + '\r\n'
        return json.dumps(message).encode()

    async def accept(self, reader, writer):
        task = asyncio.current_task(); self.tasks.add(task); upstream_writer = None
        self.connections.add(writer)
        peer = self.peer_count; self.peer_count += 1
        try:
            if len(self.connections) > 2 or peer >= 2: raise ValueError('Qualification signaling peer count exceeds two')
            request = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 5)
            if len(request) > 8192: raise ValueError('Qualification HTTP handshake exceeds bound')
            rows = request.decode('ascii').split('\r\n')
            headers = {key.lower(): value.strip() for line in rows[1:] if ':' in line for key, value in [line.split(':', 1)]}
            key = headers['sec-websocket-key']
            upstream_reader, upstream_writer = await asyncio.open_connection('127.0.0.1', self.selected.port)
            upstream_key = base64.b64encode(os.urandom(16)).decode()
            upstream_writer.write((f'GET {self.selected.path or "/"} HTTP/1.1\r\nHost: 127.0.0.1:{self.selected.port}\r\n'
                'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n'
                f'Sec-WebSocket-Key: {upstream_key}\r\n\r\n').encode()); await upstream_writer.drain()
            response = await asyncio.wait_for(upstream_reader.readuntil(b'\r\n\r\n'), 5)
            if not response.startswith(b'HTTP/1.1 101 '): raise ValueError('Selected signaling upgrade rejected')
            accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            writer.write(('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n'
                f'Sec-WebSocket-Accept: {accept}\r\n\r\n').encode()); await writer.drain()
            async def pump(source, destination, masked, rewrite):
                while True:
                    opcode, payload = await frame(source)
                    if rewrite and opcode == 1: payload = await self.rewrite(payload, peer)
                    destination.write(encode(opcode, payload, masked)); await destination.drain()
                    if opcode == 8: return
            pumps = [asyncio.create_task(pump(reader, upstream_writer, True, False)),
                asyncio.create_task(pump(upstream_reader, writer, False, True))]
            try:
                done, pending = await asyncio.wait(pumps, return_when=asyncio.FIRST_COMPLETED)
                for result in done: await result
            finally:
                for result in pumps: result.cancel()
                await asyncio.gather(*pumps, return_exceptions=True)
        except (asyncio.IncompleteReadError, ConnectionError): pass
        except Exception as error: self.errors.append(type(error).__name__)
        finally:
            if upstream_writer: upstream_writer.close()
            writer.close(); self.connections.discard(writer); self.tasks.discard(task)

    async def __aexit__(self, *_):
        self.server.close(); await self.server.wait_closed()
        for connection in self.connections: connection.close()
        for task in list(self.tasks): task.cancel()
        if self.tasks: await asyncio.gather(*self.tasks, return_exceptions=True)
        for relay in self.relays.values(): await relay.__aexit__(None, None, None)

    def report(self):
        return {'scope': 'actual loopback WebRTC STUN/DTLS/SRTP packets through owned SDP/ICE relays',
            'candidate_relays': len(self.relays), 'candidate_relays_bound': 8,
            'errors': self.errors, 'relays': [dict(relay.report(), owned_frontend_port=relay.address[1],
                signaling_peer=self.relay_peers[port]) for port, relay in self.relays.items()]}

    def verify_selected_paths(self, observations):
        """Require the actual selected media route throughout the measured window."""
        owned = {relay.address[1]: relay for relay in self.relays.values()}
        paths = []
        for observation in observations:
            samples = observation.get('selected_ice_samples', [])
            seconds = observation['elapsed_seconds']
            valid = (not observation.get('selected_ice_observation_errors', 0)
                and len(samples) >= seconds * 1.5
                and samples[-1]['unix_seconds'] - samples[0]['unix_seconds'] >= seconds - .8
                and all(sample.get('remote_port') in owned and sample.get('transport') == 'udp'
                    and sample.get('candidate_type') == 'host' for sample in samples))
            probabilities = {owned[s['remote_port']].drop_probability for s in samples
                if s.get('remote_port') in owned}
            impaired = valid and probabilities == {.03}
            clean = valid and probabilities == {0}
            paths.append({'status': 'passed' if impaired or clean else 'failed',
                'samples': len(samples), 'selected_remote_ports': sorted({s['remote_port'] for s in samples if s.get('remote_port')}),
                'impaired_hop': impaired, 'clean_hop': clean})
        passed = bool(paths) and all(path['status'] == 'passed' for path in paths) and sum(path['impaired_hop'] for path in paths) == 1
        return {'status': 'passed' if passed else 'failed', 'interval_seconds': .5,
            'method': 'public browser diagnostics selected candidate-pair remote port must remain an owned host UDP relay throughout the measured window; exactly one selected browser hop is impaired',
            'paths': paths}
