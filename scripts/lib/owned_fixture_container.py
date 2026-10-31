"""Persist exact Docker custody before create; recover uncertain replies by ownership."""
import json
import os
from pathlib import Path
import re
import subprocess
import uuid


def command(args, timeout=15):
    return subprocess.check_output(args, text=True, stderr=subprocess.PIPE, timeout=timeout).strip()


class OwnedContainer:
    def __init__(self, directory, image, network, invoke=command):
        assert re.fullmatch(r'sha256:[0-9a-f]{64}', image)
        assert network == 'none' or re.fullmatch(r'container:[0-9a-f]{64}', network)
        self.directory, self.image, self.network, self.invoke = Path(directory), image, network, invoke
        self.name, self.label = 'connect-g6-expiry-' + uuid.uuid4().hex[:12], uuid.uuid4().hex
        self.cid = None
        self.requested = False
        self.report = dict(original_container_joined=False, owned_container_removed=False, forced_cleanup=False)
        self.save()

    def save(self):
        path = self.directory / 'container-custody.json'
        with open(path, 'w', opener=lambda name, flags: os.open(name, flags, 0o600)) as stream:
            json.dump(dict(name=self.name, label=self.label, image=self.image, network=self.network,
                cid=self.cid, requested=self.requested, cleanup=self.report), stream)

    def inspect(self):
        value = json.loads(self.invoke(['docker', 'inspect', self.cid or self.name]))[0]
        assert value['Name'] == '/' + self.name and value['Image'] == self.image
        assert value['Config']['Labels'].get('connect.gate6.expiry') == self.label
        assert value['HostConfig']['NetworkMode'] == self.network
        assert re.fullmatch(r'[0-9a-f]{64}', value['Id'])
        assert self.cid is None or value['Id'] == self.cid
        self.cid = value['Id']
        self.save()
        return value

    def create(self, arguments, entrypoint, tail):
        self.requested = True
        self.save()
        identity = self.invoke(['docker', 'create', '--name', self.name, '--label',
            'connect.gate6.expiry=' + self.label, '--network', self.network, '--read-only',
            '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
            '--user', f'{os.getuid()}:{os.getgid()}', *arguments,
            '--entrypoint', entrypoint, self.image, *tail])
        assert re.fullmatch(r'[0-9a-f]{64}', identity)
        self.cid = identity
        self.inspect()
        assert self.invoke(['docker', 'start', self.cid]) == self.cid
        return self

    def remove(self, original_exit=False):
        if not self.requested:
            return self.report
        try:
            info = self.inspect()
            if info['State']['Running']:
                # A caller must establish its normal exit/drain before removal.
                self.report['forced_cleanup'] = not original_exit
                self.invoke(['docker', 'kill', '--signal', 'TERM', self.cid])
                code = int(self.invoke(['docker', 'wait', self.cid], timeout=10))
            else:
                code = info['State']['ExitCode']
            self.report['original_exit_code'] = code
            self.report['original_container_joined'] = not self.inspect()['State']['Running']
            assert self.report['original_container_joined']
            assert self.invoke(['docker', 'rm', self.cid]) == self.cid
            self.report['owned_container_removed'] = True
        except Exception as error:
            # Never delete a container whose exact image/network/label no longer
            # matches this retained custody record.
            self.report['cleanup_error'] = type(error).__name__ if isinstance(error,
                (AssertionError, subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError)) else 'other'
        self.save()
        return self.report
