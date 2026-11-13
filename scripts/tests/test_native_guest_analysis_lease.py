"""Pure fixture-order/custody controls; no guest, browser or workload claims."""
import ast
import io
import json
from pathlib import Path
import sys
import tempfile
import traceback
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.mac_linux_fixture import MacLinuxFixture
from lib.matrix_archive_drain import closed_guest_payload_budget
from lib.native_programme_controls import (merge_native_failures, ordinary_native_check,
    run_late_native_controls, late_controls_not_run, record_native_failure,
    CONTROL_ERRORS, OriginalControlOwnerUnjoined)


class NativeGuestAnalysisLease(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def fixture(self, *, inputs=True, binaries=True, owners=True, status=0):
        fixture = MacLinuxFixture(self.root, self.root / 'owner', {})
        fixture.closed = True
        fixture.receipt = dict(passed=True, original_joined=True, forced=False, cleanup_failures=[])
        fixture.inputs = {'job.json': 'original-input-digest'}
        fixture.client_binary = '/original/client'
        fixture.client_binary_sha256 = 'original-client-digest'
        fixture.collected_directories = {'collected'}
        events = []
        class Pipe(io.BytesIO):
            def close(self):
                events.append('guardian-input-close')
                super().close()
        def wait(timeout):
            events.append('original-guardian-wait')
            self.assertIsNotNone(fixture.final_guest_work)
            self.assertEqual(timeout, 380)
            return status
        fixture.lease = SimpleNamespace(stdin=Pipe(), stdout=io.BytesIO(), stderr=io.BytesIO(), wait=wait)
        values = [fixture.inputs if inputs else {},
            {fixture.client_binary: fixture.client_binary_sha256} if binaries else {},
            dict(all_guest_owners_joined=owners, file_count=0, bytes=0)]
        def call(command, **kwargs):
            events.append(['inputs', 'binaries', 'pending-and-owners'][3-len(values)])
            return json.dumps(values.pop(0)).encode()
        fixture.call = Mock(side_effect=call)
        return fixture, events

    def test_actual_final_barrier_joins_same_guardian_then_freezes_commands_and_reuses_proof(self):
        fixture, events = self.fixture()
        proof = fixture.finish_native_guest_work([self.root / 'collected'])
        self.assertEqual(events, ['inputs', 'binaries', 'pending-and-owners',
            'guardian-input-close', 'original-guardian-wait'])
        self.assertTrue(proof['original_guardian_joined'])
        self.assertTrue(fixture.receipt['original_vm_lease_joined'])
        count = fixture.call.call_count
        self.assertTrue(fixture.verify_binaries())  # frozen local proof, not SSH
        self.assertIs(fixture.close(), fixture.receipt)
        self.assertIs(fixture.finish_native_guest_work([self.root / 'collected']), proof)
        self.assertEqual(fixture.call.call_count, count)
        with self.assertRaisesRegex(RuntimeError, 'frozen'):
            fixture.ssh(['python3', 'no-new-guest-work'])

    def test_incomplete_artifacts_integrity_or_owner_proof_cannot_release_early(self):
        for options in ({'inputs': False}, {'binaries': False}, {'owners': False}, {}):
            with self.subTest(options=options):
                directory = self.root / ('case-' + str(len(list(self.root.iterdir()))))
                directory.mkdir()
                # Use a separate root for each real constructor's exclusive owner directory.
                old = self.root
                self.root = directory
                try:
                    fixture, events = self.fixture(**options)
                finally:
                    self.root = old
                required = directory / ('missing' if not options else 'collected')
                with self.assertRaises(ValueError):
                    fixture.finish_native_guest_work([required])
                self.assertIsNone(fixture.final_guest_work)
                self.assertNotIn('guardian-input-close', events)
                self.assertNotIn('original-guardian-wait', events)
                self.assertFalse(fixture.lease.stdin.closed)
                for pipe in (fixture.lease.stdin, fixture.lease.stdout, fixture.lease.stderr):
                    pipe.close()

    def test_failed_actual_guardian_wait_never_becomes_success_or_reopens_guest(self):
        fixture, events = self.fixture(status=1)
        with self.assertRaisesRegex(RuntimeError, 'unconfirmed'):
            fixture.finish_native_guest_work([self.root / 'collected'])
        self.assertFalse(fixture.receipt['passed'])
        self.assertFalse(fixture.receipt['original_vm_lease_joined'])
        with self.assertRaises(RuntimeError):
            fixture.ssh(['no-replacement-guardian'])
        self.assertEqual(events.count('original-guardian-wait'), 1)

    def test_actual_payload_metadata_budget_retains_bytes_and_refuses_overflow_or_forced_owner(self):
        payload = self.root / 'received-0.json'
        payload.write_bytes(b'original')
        original = payload.stat()
        (self.root / '.owners').mkdir()
        owner = self.root / '.owners/command-a-receipt.json'
        owner.write_text(json.dumps(dict(original_joined=True, forced=False)))
        with patch('lib.mac_linux_guest.lease_owners_joined', return_value=True):
            value = closed_guest_payload_budget(self.root, {})
            self.assertEqual(value['bytes'], 8)
            self.assertEqual(value['file_count'], 1)
            self.assertFalse(value['pending_payload_deleted'])
            self.assertEqual(value['files']['received-0.json']['inode'], original.st_ino)
            with patch('lib.matrix_archive_drain.MAX_GUEST_BYTES', 7):
                with self.assertRaisesRegex(ValueError, 'Bounded pending'):
                    closed_guest_payload_budget(self.root, {})
            with patch('lib.matrix_archive_drain.MAX_GUEST_FILES', 0):
                with self.assertRaises(ValueError):
                    closed_guest_payload_budget(self.root, {})
            owner.write_text(json.dumps(dict(original_joined=True, forced=True)))
            with self.assertRaisesRegex(ValueError, 'Natural original'):
                closed_guest_payload_budget(self.root, {})
            owner.write_text(json.dumps(dict(original_joined=True, forced=False)))
        with patch('lib.mac_linux_guest.lease_owners_joined', return_value=False):
            self.assertFalse(closed_guest_payload_budget(self.root, {})['all_guest_owners_joined'])
        self.assertEqual(payload.read_bytes(), b'original')
        self.assertEqual(payload.stat().st_ino, original.st_ino)

    def test_deferred_late_failure_keeps_ordinary_priority_and_unknown_owner_stops_dispatch(self):
        report, deferred, calls = {}, {}, []
        def before(owner):
            calls.append(owner)
            raise ValueError('ordinary late refusal')
        late = run_late_native_controls(deferred, owners_joined=True, owners={'one', 'two'},
            expected={'owned'}, before_read=before, revoke_primary=lambda: True,
            primary_refused=lambda: True, after_read=lambda owner: [], configs_unchanged=lambda: True)
        self.assertEqual(late['status'], 'failed')
        ordinary_native_check(report, 'independent-native-opus-vp8-decode', False)
        merge_native_failures(report, deferred)
        self.assertEqual(report['failure']['phase'], 'independent-native-opus-vp8-decode')
        self.assertEqual(report['additional_failures'][1]['name'], 'ValueError')
        self.assertEqual(calls, ['one', 'two'])
        fatal = {}
        def unjoined(owner):
            raise OriginalControlOwnerUnjoined('original child has no terminal custody')
        revoke = Mock()
        with self.assertRaises(OriginalControlOwnerUnjoined):
            run_late_native_controls(fatal, owners_joined=True, owners={'one', 'two'}, expected={'owned'},
                before_read=unjoined, revoke_primary=revoke, primary_refused=lambda: True,
                after_read=lambda owner: [], configs_unchanged=lambda: True)
        revoke.assert_not_called()
        self.assertEqual(len(fatal['late_native_controls']['steps']), 1)
        self.assertEqual(fatal['late_native_controls']['steps'][0]['status'], 'owner-unconfirmed')

    def test_actual_producer_late_try_keeps_final_success_and_partial_only_on_fatal(self):
        source = Path(__file__).resolve().parents[1] / 'check-transport-fixtures.py'
        tree = ast.parse(source.read_text())
        statement = min((n for n in ast.walk(tree) if isinstance(n, ast.Try)
            and any(isinstance(v, ast.Call) and isinstance(v.func, ast.Name)
                and v.func.id == 'run_late_native_controls' for stmt in n.body for v in ast.walk(stmt))),
            key=lambda n: n.end_lineno - n.lineno)
        # Compile only the actual narrow try/except, never import/run the producer.
        code = compile(ast.Module(body=[statement], type_ignores=[]), str(source), 'exec')
        owners = ['controller-1', 'controller-2', 'participant-2', 'participant-3', 'participant-4', 'replacement-1']
        rows = [dict(id='owned', policy='persistent')]
        report, deferred, calls = {}, {}, []
        def rooms(name, config):
            calls.append(name)
            return [dict(v) for v in rows]
        env = dict(report=report, late_failures=deferred, identity_ok=True, expected_owned={'owned'},
            run_late_native_controls=run_late_native_controls, late_controls_not_run=late_controls_not_run,
            original_native_owners_joined=lambda *args: True, media_workload=True, resource_cycles=3,
            records={name: dict(config=name) for name in [*owners, 'participant-1']}, rooms=rooms, Path=Path,
            revoke_original_primary=lambda: True, original_primary_refused=lambda: True,
            files=lambda directory: {}, directory=self.root, installed={})
        exec(code, env)
        self.assertEqual(report['late_native_controls']['status'], 'passed')
        self.assertTrue(report['late_native_controls']['all_required_calls_attempted'])
        self.assertEqual(len(report['late_native_controls']['steps']), 15)
        self.assertEqual(len(calls), 12)
        self.assertEqual(deferred['late_native_controls']['status'], 'incomplete')
        # The helper's progress object differs from its final result. The actual
        # producer must never overwrite success with that incomplete progress.
        def unknown(name, config):
            raise OriginalControlOwnerUnjoined('unknown original child')
        env.update(report={}, late_failures={}, rooms=unknown)
        with self.assertRaises(OriginalControlOwnerUnjoined):
            exec(code, env)
        self.assertEqual(env['report']['late_native_controls']['status'], 'incomplete')
        self.assertEqual(env['report']['late_native_controls']['steps'][0]['status'], 'owner-unconfirmed')

    def test_actual_bad_lineage_and_barrier_failure_still_decode_without_late_dispatch(self):
        source = Path(__file__).resolve().parents[1] / 'check-transport-fixtures.py'
        tree = ast.parse(source.read_text())
        def has(node, name, *, attr=False):
            return any(isinstance(n, ast.Call) and ((isinstance(n.func, ast.Attribute) and n.func.attr == name)
                if attr else (isinstance(n.func, ast.Name) and n.func.id == name)) for n in ast.walk(node))
        lineage = min((n for n in ast.walk(tree) if isinstance(n, ast.Try) and has(n, 'owned_conversation_ids')),
            key=lambda n: n.end_lineno-n.lineno)
        late = min((n for n in ast.walk(tree) if isinstance(n, ast.Try) and has(n, 'run_late_native_controls')),
            key=lambda n: n.end_lineno-n.lineno)
        barrier = min((n for n in ast.walk(tree) if isinstance(n, ast.If) and has(n, 'finish_native_guest_work', attr=True)),
            key=lambda n: n.end_lineno-n.lineno)
        decode = min((n for n in ast.walk(tree) if isinstance(n, ast.If)
            and n.lineno > barrier.lineno and has(n, 'decode_completed_av')), key=lambda n: n.lineno)
        programme_decode = Mock(return_value={'passed': True})
        no_late = Mock(side_effect=AssertionError('No late command may dispatch'))
        report, deferred = {}, {}
        remote = SimpleNamespace(finish_native_guest_work=Mock(side_effect=ValueError('owner barrier failed')))
        env = dict(report=report, late_failures=deferred, expected_owned=None, identity_ok=True,
            CONTROL_ERRORS=CONTROL_ERRORS, record_native_failure=record_native_failure, traceback=traceback,
            cycle_observations=[{'resource_cycle_index': 1, 'plan_sha256': 'not-present'}],
            resource_cycles=3, PRIVATE=self.root, OUTPUT=self.root/'output',
            digest=Mock(side_effect=FileNotFoundError('original create plan missing')),
            owned_conversation_ids=no_late, run_late_native_controls=no_late,
            late_controls_not_run=late_controls_not_run, remote=remote, media_workload=True,
            decode_completed_av=programme_decode, media_sources={},
            native_three_cycle_quality=lambda values: {'passed': all(v['passed'] for v in values)}, decoded=None)
        code = compile(ast.Module(body=[lineage, late, barrier, decode], type_ignores=[]), str(source), 'exec')
        exec(code, env)
        no_late.assert_not_called()
        self.assertEqual(report['late_native_controls']['status'], 'not-run')
        self.assertFalse(report['native_programme_conversation_ownership']['lineage_confirmed'])
        programme_decode.assert_called_once()
        self.assertTrue(report['native_av_decoded']['passed'])
        self.assertNotIn('failure', report)
        self.assertEqual([v['phase'] for v in deferred['additional_failures']],
            ['original-native-create-lineage', 'owned-native-final-guest-work'])
        ordinary_native_check(report, 'original-ordinary-failure', False)
        merge_native_failures(report, deferred)
        self.assertEqual(report['failure']['phase'], 'original-ordinary-failure')
        self.assertEqual(len(report['additional_failures']), 3)

    def test_actual_producer_orders_late_guest_barrier_before_host_decode_and_preserves_s8(self):
        source = Path(__file__).resolve().parents[1] / 'check-transport-fixtures.py'
        tree = ast.parse(source.read_text())
        calls = [n for n in ast.walk(tree) if isinstance(n, ast.Call)]
        named = lambda name: [n for n in calls if isinstance(n.func, ast.Name) and n.func.id == name]
        attributed = lambda name: [n for n in calls if isinstance(n.func, ast.Attribute) and n.func.attr == name]
        late = named('run_late_native_controls')[0]
        barrier = attributed('finish_native_guest_work')[0]
        decodes = named('decode_completed_av')
        merged = named('merge_native_failures')[0]
        self.assertLess(late.lineno, barrier.lineno)
        self.assertTrue(all(barrier.lineno < n.lineno < merged.lineno for n in decodes))
        self.assertTrue(all(n.lineno < barrier.lineno for n in attributed('collect_directory')))
        # S8's actor-directory fix is a mandatory beforeimage, never reverted.
        loop = next(n for n in ast.walk(tree) if isinstance(n, ast.For)
            and isinstance(n.target, ast.Name) and n.target.id == 'collected_directory')
        self.assertTrue(any(isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute)
            and n.func.attr == 'collect_directory' for n in ast.walk(loop)))
        finalizer = min((n for n in ast.walk(tree) if isinstance(n, ast.Try)
            and any(isinstance(v, ast.Call) and isinstance(v.func, ast.Attribute)
                and v.func.attr == 'verify_binaries' for stmt in n.body for v in ast.walk(stmt))
            and any(isinstance(v, ast.Call) and isinstance(v.func, ast.Attribute)
                and v.func.attr == 'close' for stmt in n.finalbody for v in ast.walk(stmt))),
            key=lambda n: n.end_lineno-n.lineno)
        self.assertTrue(finalizer.finalbody)
        self.assertLess(next(n.lineno for n in ast.walk(finalizer) if isinstance(n, ast.Call)
            and isinstance(n.func, ast.Attribute) and n.func.attr == 'verify_binaries'),
            max(n.lineno for stmt in finalizer.finalbody for n in ast.walk(stmt)
                if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == 'close'))



if __name__ == '__main__':
    unittest.main()
