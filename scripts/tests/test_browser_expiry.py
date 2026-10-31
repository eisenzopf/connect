"""Expired credential evidence and uncertain Docker custody boundaries."""
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
OWNER = runpy.run_path(str(ROOT / 'scripts/lib/owned_fixture_container.py'))['OwnedContainer']
EXPIRY = runpy.run_path(str(ROOT / 'scripts/check-gate6-browser-expiry.py'))


class ExpiryBoundaries(unittest.TestCase):
    def owner(self, directory):
        owner = OWNER(directory, 'sha256:' + 'b' * 64, 'none')
        info = dict(Id='a' * 64, Name='/' + owner.name, Image=owner.image,
            Config=dict(Labels={'connect.gate6.expiry': owner.label}),
            HostConfig=dict(NetworkMode='none'), State=dict(Running=False, ExitCode=0))
        return owner, info

    def test_uncertain_create_reply_recovers_only_exact_retained_ownership(self):
        with tempfile.TemporaryDirectory() as directory:
            owner, info = self.owner(directory)
            calls = []
            def invoke(args, **kwargs):
                calls.append(args)
                if args[1] == 'create':
                    saved = json.loads(Path(directory, 'container-custody.json').read_text())
                    self.assertTrue(saved['requested'])
                    self.assertEqual(saved['label'], owner.label)
                    raise subprocess.TimeoutExpired(args[:2], 15)
                if args[1] == 'inspect': return json.dumps([info])
                self.assertEqual(args, ['docker', 'rm', info['Id']])
                return info['Id']
            owner.invoke = invoke
            with self.assertRaises(subprocess.TimeoutExpired): owner.create([], 'python3', [])
            self.assertIsNone(owner.cid)
            report = owner.remove()
            self.assertTrue(report['owned_container_removed'])
            self.assertTrue(report['original_container_joined'])
            self.assertEqual(calls[1], ['docker', 'inspect', owner.name])

    def test_foreign_image_network_label_or_identity_cannot_be_removed(self):
        for field in ('Image', 'NetworkMode', 'label', 'Name', 'Id'):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                owner, info = self.owner(directory)
                owner.requested = True
                owner.cid = info['Id']
                if field == 'NetworkMode': info['HostConfig'][field] = 'host'
                elif field == 'label': info['Config']['Labels']['connect.gate6.expiry'] = 'foreign'
                else: info[field] = 'foreign'
                calls = []
                def invoke(args, **kwargs):
                    calls.append(args)
                    self.assertEqual(args[1], 'inspect')
                    return json.dumps([info])
                owner.invoke = invoke
                report = owner.remove()
                self.assertFalse(report['owned_container_removed'])
                self.assertEqual(report['cleanup_error'], 'AssertionError')
                self.assertEqual(len(calls), 1)

    def test_running_child_without_confirmed_original_join_marks_forced_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            owner, info = self.owner(directory)
            owner.requested = True
            owner.cid = info['Id']
            info['State']['Running'] = True
            def invoke(args, **kwargs):
                if args[1] == 'inspect': return json.dumps([info])
                if args[1] == 'kill':
                    self.assertEqual(args[2:4], ['--signal', 'TERM'])
                    info['State']['Running'] = False
                    return owner.cid
                if args[1] == 'wait': return '0'
                self.assertEqual(args[1], 'rm')
                return owner.cid
            owner.invoke = invoke
            report = owner.remove()
            self.assertTrue(report['owned_container_removed'])
            self.assertTrue(report['forced_cleanup'])

    def evidence(self, directory):
        path = Path(directory)
        (path / 'browser').mkdir()
        (path / 'audio').mkdir()
        results = [dict(participant=i, error_code='unsupported-media-profile', seconds=5.1,
            limit_seconds=8, logical_offers=0, submission_attempted=False, owned_peer_closed=True,
            owned_audio_tracks_ended=True, remote_description_set=False, private='private-secret') for i in range(2)]
        browser = dict(status='passed', reason='expired', original_contexts_closed=2, results=results)
        oracle = dict(passed=True, logical_operations=0, physical_allocations=0)
        pulse = dict(passed=True, original_process_joined=True, exit_code=0, binary_sha256='a' * 64,
            version='pulseaudio 16.1', private='private-secret')
        (path / 'browser/browser-result.jsonl').write_text(json.dumps({'expired_relay': browser}))
        (path / 'browser/expired-relay-oracle.json').write_text(json.dumps(oracle))
        (path / 'audio/summary.json').write_text(json.dumps(pulse))
        return browser, oracle, pulse

    def test_real_expiry_oracle_requires_auth_refusal_zero_allocations_and_original_joins(self):
        log = 'incoming packet message processed, error 401: Unauthorized\ncredentials of user <private-secret> have expired\n'
        for defect in ('none', 'no-auth-refusal', 'allocation', 'logical-offer', 'capture', 'audio-kill'):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as directory:
                browser, oracle, pulse = self.evidence(directory)
                current = log
                if defect == 'no-auth-refusal': current = ''
                if defect == 'allocation': current += 'ALLOCATE processed, success\n'
                if defect == 'logical-offer': oracle['logical_operations'] = 1
                if defect == 'capture': browser['results'][0]['owned_audio_tracks_ended'] = False
                if defect == 'audio-kill': pulse['forced_kill'] = True
                path = Path(directory)
                (path / 'browser/browser-result.jsonl').write_text(json.dumps({'expired_relay': browser}))
                (path / 'browser/expired-relay-oracle.json').write_text(json.dumps(oracle))
                (path / 'audio/summary.json').write_text(json.dumps(pulse))
                if defect == 'none':
                    value = EXPIRY['expiry_facts'](path, current, 'private-secret')
                    self.assertNotIn('private-secret', json.dumps(value))
                    self.assertEqual(value['expired_credential_refusals'], 1)
                else:
                    with self.assertRaises(AssertionError): EXPIRY['expiry_facts'](path, current, 'private-secret')


if __name__ == '__main__':
    unittest.main()
