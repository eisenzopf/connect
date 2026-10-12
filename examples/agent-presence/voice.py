#!/usr/bin/env python3
"""A persistent, caller-owned voice request over the public native QUIC SDK."""
import argparse
import json
import os
from pathlib import Path
import tomllib
import uuid
from connect_sdk import NativeClient

p = argparse.ArgumentParser()
p.add_argument('action', choices=['start', 'status', 'end'])
p.add_argument('--config', required=True)
p.add_argument('--state', required=True, help='Private caller receipt file; reuse it after uncertainty')
p.add_argument('--conversation')
p.add_argument('--route')
p.add_argument('--destination')
p.add_argument('--executable', default='connect')
a = p.parse_args()
state_path = Path(a.state)
identity = tomllib.loads(Path(a.config).read_text())['identity_id']

def save(value, new=False):
    target = state_path if new else state_path.with_suffix('.pending.json')
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f:
        json.dump(value, f, indent=2)
        f.flush()
        os.fsync(f.fileno())
    if not new:
        os.replace(target, state_path)
    dfd = os.open(state_path.parent, os.O_RDONLY)
    os.fsync(dfd)
    os.close(dfd)

if state_path.exists():
    state = json.loads(state_path.read_text())
    if state['identity_id'] != identity:
        raise SystemExit('Receipt belongs to another Connect identity')
    for name in ['conversation', 'route', 'destination']:
        supplied = getattr(a, name)
        if supplied is not None and supplied != state[name]:
            raise SystemExit('Existing request inputs cannot change')
else:
    if a.action != 'start' or not all([a.conversation, a.route, a.destination]):
        raise SystemExit('First start requires conversation, route and destination')
    state = {'identity_id': identity, 'conversation': a.conversation, 'route': a.route,
             'destination': a.destination, 'start_id': 'voice-' + uuid.uuid4().hex,
             'end_id': 'end-' + uuid.uuid4().hex, 'call_id': None}
    save(state, new=True)  # persist before any network mutation

with NativeClient(a.config, a.executable) as client:
    if a.action == 'start' or state['call_id'] is None:
        result = client.call_start(state['start_id'], state['conversation'],
                                   state['route'], state['destination'])
        state['call_id'] = result['id']
        save(state)
    if a.action == 'status':
        result = client.call_status(state['conversation'], state['call_id'])
    elif a.action == 'end':
        result = client.call_end(state['end_id'], state['conversation'], state['call_id'])
    print(json.dumps({'call_id': result['id'], 'state': result['state'],
                      'provider_id': result['provider_id'], 'ended_reason': result['ended_reason']}))
