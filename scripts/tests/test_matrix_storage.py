"""Fault injection uses only small temporary files; no VM/media/workload."""
import errno
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib import matrix_storage as storage
from lib.vm_workload_lease import GuardedVmWorkloadLease


class StorageAdmission(unittest.TestCase):
    def observation(self, free=100 * storage.GIB):
        return {role:dict(free_bytes=free, available_inodes=100000) for role in ('guest','backing','spool')}

    def test_large_guest_does_not_hide_full_physical_backing(self):
        observed = self.observation();observed['backing']['free_bytes'] = storage.GIB
        result = storage.admit(observed, 2 * storage.GIB)
        self.assertFalse(result['admitted']);self.assertIn('backing-storage-headroom-unavailable', result['reasons'])
        # Space can settle after a concurrent original closed-guest drain. The
        # same floor is rechecked before starting any component; no warmup.
        hosts=[dict(backing=observed['backing'],spool=observed['spool']),
               dict(backing=self.observation()['backing'],spool=self.observation()['spool'])]
        with patch.object(storage,'request_host',side_effect=hosts),patch.object(storage.time,'sleep') as pause, \
                patch.object(storage.time,'monotonic',return_value=0):
            current=storage.await_admission(lambda:observed['guest'],2*storage.GIB)
        self.assertTrue(current['admitted']);self.assertEqual(current['storage_observation_count'],2)
        self.assertIn('backing-storage-headroom-unavailable',current['initial_block_reasons']);pause.assert_called_once()


    def test_spool_missing_or_exhausted_inodes_never_admits(self):
        for change in ('missing','inodes'):
            observed = self.observation()
            if change == 'missing':del observed['spool']
            else:observed['spool']['available_inodes'] = 0
            self.assertFalse(storage.admit(observed, 1)['admitted'])

    def test_budget_discloses_sources_three_cycles_build_and_archive_overlap(self):
        budget = storage.cell_budget()
        self.assertEqual(budget['raw_programmes'], 3856896000)
        self.assertEqual(budget['received_three_cycles'], 3 * 512 * storage.MIB)
        self.assertGreater(budget['build_growth'], 0);self.assertGreater(budget['archive_overlap'], budget['raw_programmes'])
        self.assertEqual(storage.GUARD_RESERVE_BYTES, 4 * 16384)
        child=storage.guest_budget(borrowed_lease=True)
        self.assertNotIn('raw_programmes',child)
        self.assertEqual(sum(child.values()),1024*storage.MIB+storage.RUN_RESERVE_BYTES)


    def test_archive_budget_counts_independently_emitted_hardlink_names(self):
        row = dict(kind='file', bytes=1024**2, mode=0o600)
        one = storage.archive_budget({'a':row})
        both = storage.archive_budget({'a':row, 'b':row})
        self.assertGreaterEqual(both - one, row['bytes'])

    def test_unavailable_coordinator_does_not_become_a_zero_volume(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(storage, 'request_host', side_effect=KeyError('missing')):
            with self.assertRaises(KeyError):storage.preflight(Path(temp))


class DurableMetadata(unittest.TestCase):
    def test_enospc_keeps_last_valid_original_without_owned_reserve(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'state.json';path.write_text('{"original":true}')
            with patch.object(storage.os, 'replace', side_effect=OSError(errno.ENOSPC, 'full')):
                with self.assertRaises(OSError):storage.atomic_json(path, dict(replacement=True))
            self.assertEqual(json.loads(path.read_text()), dict(original=True))
            self.assertEqual(list(Path(temp).iterdir()), [path])

    def test_enospc_releases_only_owned_synthetic_inode_and_retries_atomically(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);path=root/'state.json';path.write_text('{"old":true}')
            reserve=storage.MetadataReserve(root/'reserve', 4096);replace=storage.os.replace;calls=[]
            def full_once(source, destination):
                calls.append(1)
                if len(calls)==1:
                    self.assertEqual(json.loads(path.read_text()), dict(old=True));raise OSError(errno.ENOSPC,'full')
                replace(source,destination)
            with patch.object(storage.os,'replace',side_effect=full_once):storage.atomic_json(path,dict(actual_joined=True),reserve=reserve)
            self.assertTrue(reserve.released);self.assertFalse(reserve.path.exists())
            self.assertEqual(json.loads(path.read_text()),dict(actual_joined=True));self.assertEqual(len(calls),2)

    def test_reused_reserve_path_or_symlink_never_deletes_unrelated_bytes(self):
        for kind in ('regular','symlink'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temp:
                root=Path(temp);owner=storage.MetadataReserve(root/'reserve',4096);owner.path.unlink()
                evidence=root/'evidence';evidence.write_bytes(b'original-evidence')
                if kind=='regular':owner.path.write_bytes(b'unrelated')
                else:owner.path.symlink_to(evidence)
                with self.assertRaises(ValueError):owner.release()
                self.assertEqual(evidence.read_bytes(),b'original-evidence');self.assertTrue(owner.path.exists())

    def test_non_enospc_or_oversized_metadata_does_not_consume_reserve(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);owner=storage.MetadataReserve(root/'reserve',4096)
            with patch.object(storage.os,'replace',side_effect=OSError(errno.EIO,'io')):
                with self.assertRaises(OSError):storage.atomic_json(root/'state',{},reserve=owner)
            with self.assertRaises(ValueError):storage.atomic_bytes(root/'state',b'12345',limit=4,reserve=owner)
            self.assertFalse(owner.released);self.assertTrue(owner.path.exists())

    def test_symlink_metadata_target_is_not_replaced(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);original=root/'evidence';original.write_text('original');link=root/'state';link.symlink_to(original)
            with self.assertRaises(ValueError):storage.atomic_json(link,{})
            self.assertTrue(link.is_symlink());self.assertEqual(original.read_text(),'original')

    def test_guardian_unentered_custody_is_explicit_and_unknown_remains_blocked(self):
        guard=GuardedVmWorkloadLease();self.assertFalse(guard.evidence['confirmed'])
        guard.complete_unentered();self.assertTrue(guard.evidence['confirmed'])
        self.assertIs(guard.evidence['component_owner_started'],False)
        guard.complete({});self.assertFalse(guard.evidence['confirmed'])


class OriginalLeaseOrdering(unittest.TestCase):
    def test_storage_refusal_after_lease_never_starts_component_and_releases_unentered(self):
        loader=importlib.util.spec_from_file_location('storage_matrix_driver',ROOT/'scripts/run-gate6-transport-matrix.py')
        driver=importlib.util.module_from_spec(loader);loader.loader.exec_module(driver)
        events=[]
        class Lease:
            blocked=False
            def __enter__(self):events.append('lease');return self
            def complete_unentered(self):events.append('unentered')
            def __exit__(self,*_):events.append('release')
        cell=dict(id='original',server='ubuntu24-arm64',status='not-run',required_checks=['media'],
                  execution=dict(runnable=True,attempted=False),transport='native-quic',signing='EdDSA',network='local')
        later=dict(cell,id='later',execution=dict(runnable=True,attempted=False))
        matrix=dict(requested_stage='full',execution_environment=dict(server='ubuntu24-arm64'),cells=[cell,later])
        def reject(_):events.append('admission');raise storage.StorageAdmissionError(dict(admitted=False,reasons=['backing-full']))
        with tempfile.TemporaryDirectory() as temp, patch.object(driver.platform,'system',return_value='Linux'), \
                patch.object(driver,'GuardedVmWorkloadLease',return_value=Lease()),patch.object(driver,'summarize'):
            driver.execute_selected(matrix,['original','later'],Path(temp),lambda _:self.fail('component started'),storage_admit=reject)
        self.assertEqual(events,['lease','admission','unentered','release'])
        self.assertEqual(later['status'],'blocked');self.assertFalse(later['execution']['attempted'])
        self.assertEqual(cell['status'],'blocked');self.assertFalse(cell['execution']['attempted'])
        self.assertEqual(cell['check_results']['media']['status'],'blocked')

    def test_missing_observer_is_also_proven_unentered_not_attempted_media(self):
        loader=importlib.util.spec_from_file_location('storage_matrix_driver_missing',ROOT/'scripts/run-gate6-transport-matrix.py')
        driver=importlib.util.module_from_spec(loader);loader.loader.exec_module(driver)
        cell=dict(id='original',server='macos-arm64',status='not-run',required_checks=['media'],execution=dict(runnable=True,attempted=False))
        matrix=dict(requested_stage='full',execution_environment=dict(server='macos-arm64'),cells=[cell])
        with tempfile.TemporaryDirectory() as temp,patch.object(driver.platform,'system',return_value='Darwin'),patch.object(driver,'summarize'):
            driver.execute_selected(matrix,['original'],Path(temp),lambda _:self.fail('component started'),
                                    storage_admit=lambda _:(_ for _ in ()).throw(KeyError('missing mount')))
        self.assertFalse(cell['execution']['attempted']);self.assertEqual(cell['status'],'blocked')


    def test_worker_main_calls_storage_preflight_after_wiring_preflight(self):
        loader = importlib.util.spec_from_file_location(
            'storage_worker_main', ROOT / 'scripts/run-gate6-transport-matrix.py')
        driver = importlib.util.module_from_spec(loader)
        loader.loader.exec_module(driver)
        revision = 'a' * 40
        cells = [dict(id='cell-' + str(index), status='passed',
                      server='macos-arm64' if index < 16 else 'ubuntu24-arm64',
                      execution=dict(runnable=False)) for index in range(32)]
        matrix = dict(cells=cells, source_revision=revision, matrix_qualified=False,
                      execution_environment=dict(server='ubuntu24-arm64'))
        events = []
        wiring = dict(required_cells=32, tests_executed=False)
        admitted = dict(admitted=True, source='original-storage-preflight')

        def git(command, **_):
            if command == ['git', 'rev-parse', 'HEAD']:
                return revision + '\n'
            if command[:3] == ['git', 'status', '--porcelain']:
                return ''
            self.fail('Unexpected external command: ' + repr(command))

        def wiring_preflight(value):
            self.assertIs(value, matrix)
            events.append('wiring')
            return wiring

        def storage_preflight(directory):
            self.assertEqual(directory, output.resolve())
            events.append('storage')
            return admitted

        def selected(value, ids, directory, execute, *, storage_admit, **_):
            self.assertIs(value, matrix)
            self.assertEqual(ids, [cell['id'] for cell in cells])
            self.assertEqual(directory, output.resolve())
            self.assertEqual(storage_admit(cells[16]), admitted)

        with tempfile.TemporaryDirectory(dir=ROOT / '.local') as temp:
            output = Path(temp) / 'worker'
            argv = ['matrix', '--phase', 'worker', '--stage', 'full',
                    '--expected-revision', revision, '--output', str(output)]
            with patch.object(driver.sys, 'argv', argv), \
                    patch.dict(driver.os.environ, {'CI': '', 'GITHUB_ACTIONS': ''}), \
                    patch.object(driver.os, 'chdir'), patch.object(driver, 'save'), \
                    patch.object(driver, 'MetadataReserve'), \
                    patch.object(driver.subprocess, 'check_output', side_effect=git), \
                    patch.object(driver.subprocess, 'run') as external_build, \
                    patch.object(driver, 'inventory', return_value=matrix), \
                    patch.object(driver, 'probe', return_value={}), \
                    patch.object(driver, 'configure_execution'), \
                    patch.object(driver, 'full_run_preflight', side_effect=wiring_preflight), \
                    patch.object(driver, 'preflight', side_effect=storage_preflight) as actual_storage, \
                    patch.object(driver, 'execute_selected', side_effect=selected), \
                    patch.object(driver, 'select_cells', return_value=cells), \
                    patch.object(driver, 'summarize', return_value={}), patch('builtins.print'):
                self.assertEqual(driver.main(), 0)
            actual_storage.assert_called_once_with(output.resolve())
            external_build.assert_not_called()
        self.assertEqual(events, ['wiring', 'storage'])


class OriginalCoordinator(unittest.TestCase):
    def fake(self,root):
        from types import SimpleNamespace
        owner=storage.CoordinatorStorage.__new__(storage.CoordinatorStorage)
        owner.directory=root/'control';owner.directory.mkdir()
        owner.output=root/'output';owner.output.mkdir()
        owner.reserve=storage.MetadataReserve(owner.directory/'reserve',4096)
        owner.token='a'*32;owner.handled=[];owner.failures=[]
        owner.source_revision='a'*40;owner.run_directory='/workspace/.local/g6-058-'+'c'*32
        owner.drain=SimpleNamespace(completed=[],transports=[],maximum_pending_bytes=12*storage.GIB,drain=lambda _:self.fail('unexpected archive copy'))
        owner.guest_drain=SimpleNamespace(completed=[],transports=[],maximum_pending_bytes=4*storage.GIB,drain=lambda _:self.fail('unexpected guest copy'))
        return owner

    def request(self,owner,kind,payload,token=None):
        identity='b'*32
        storage.atomic_json(owner.directory/(identity+'.request.json'),
            dict(id=identity,token=owner.token if token is None else token,kind=kind,payload=payload,
                 source_revision=owner.source_revision,run_directory=owner.run_directory))
        owner.service()
        return json.loads((owner.directory/(identity+'.response.json')).read_text())

    def test_wrong_coordinator_token_never_starts_transfer(self):
        with tempfile.TemporaryDirectory() as temp:
            owner=self.fake(Path(temp));result=self.request(owner,'drain-archive',{},token='wrong')
            self.assertFalse(result['ok']);self.assertEqual(len(owner.failures),1)

    def test_low_spool_backpressure_never_copies_or_unlinks_closed_archive(self):
        with tempfile.TemporaryDirectory() as temp:
            owner=self.fake(Path(temp))
            with patch.object(storage,'volume',return_value=dict(free_bytes=1)):
                result=self.request(owner,'drain-archive',dict(archive_signature=dict(bytes=200)))
            self.assertFalse(result['ok']);self.assertEqual(owner.drain.completed,[])

    def test_host_fallback_preserves_unknown_actual_join_and_missing_vm_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            owner=self.fake(Path(temp));actual=dict(vm_receipt_saved=False,original=dict(confirmed=False))
            result=self.request(owner,'metadata-fallback',actual)
            self.assertTrue(result['ok']);self.assertFalse(result['value']['vm_receipt_saved'])
            self.assertEqual(json.loads(Path(result['value']['receipt_path']).read_text()),actual)
            self.assertFalse(json.loads(Path(result['value']['receipt_path']).read_text())['original']['confirmed'])
            identity='b'*32
            replay=dict(id=identity,token=owner.token,kind='metadata-fallback',payload=actual,
                        source_revision=owner.source_revision,run_directory=owner.run_directory)
            receipt_path=Path(result['value']['receipt_path']);before=receipt_path.stat().st_mtime_ns
            self.assertEqual(owner.answer(identity,replay),result)
            self.assertEqual(receipt_path.stat().st_mtime_ns,before)
            response_path=owner.directory/(identity+'.response.json');response_path.unlink()
            owner.service()
            self.assertEqual(json.loads(response_path.read_text()),result)
            self.assertEqual(receipt_path.stat().st_mtime_ns,before);self.assertEqual(owner.handled,[identity])
            commands=[];owner.remote_base={}
            def original_remote_command(action,payload):
                commands.append(action)
                return dict(requests=[replay],matrix={}) if action=='snapshot' else dict(response_saved=True)
            owner.remote_command=original_remote_command
            self.assertEqual(owner.remote_snapshot(),{})
            self.assertEqual(commands,['snapshot','respond'])
            self.assertEqual(receipt_path.stat().st_mtime_ns,before);self.assertEqual(owner.handled,[identity])
            altered=dict(replay,payload=dict(actual,original=dict(confirmed=True)))
            self.assertFalse(owner.answer(identity,altered)['ok'])
            self.assertEqual(json.loads(receipt_path.read_text()),actual)


    def test_backing_disk_inode_change_is_not_a_new_valid_observation(self):
        with tempfile.TemporaryDirectory() as temp:
            owner=self.fake(Path(temp));owner.backing=Path(temp)/'disk';owner.backing.write_bytes(b'disk')
            owner.backing_identity=(owner.backing.stat().st_dev,owner.backing.stat().st_ino+1)
            with self.assertRaises(ValueError):owner.observe()


class ExistingSshControl(unittest.TestCase):
    def scope(self,temp):
        parent=Path(temp).resolve();run=parent/('g6-058-'+'c'*32)
        return parent,run,dict(directory=str(run)+'-storage-control',run_directory=str(run),token='a'*32,source_revision='b'*40)

    def test_bootstrap_and_respond_preserve_original_request_and_bound_response(self):
        with tempfile.TemporaryDirectory() as temp:
            parent,run,payload=self.scope(temp)
            with patch.object(storage,'CONTROL_PARENT',parent):
                storage.control_action('bootstrap',payload);directory=Path(payload['directory'])
                identity='d'*32;request=dict(id=identity,token=payload['token'],kind='observe',payload={},
                    source_revision=payload['source_revision'],run_directory=payload['run_directory'])
                source=directory/(identity+'.request.json');storage.atomic_json(source,request);original=source.read_bytes()
                before=storage.control_action('snapshot',payload)
                self.assertIsNone(before['matrix']);self.assertEqual(before['requests'],[request])
                response=dict(id=identity,token=payload['token'],ok=True,value=dict(actual=True),
                    request_sha256=storage.request_digest(request),kind='observe',
                    source_revision=payload['source_revision'],run_directory=payload['run_directory'])
                storage.control_action('respond',dict(payload,response=response))
                self.assertEqual(storage.control_action('snapshot',payload)['requests'],[])
                self.assertEqual(source.read_bytes(),original)
                self.assertEqual(json.loads((directory/(identity+'.response.json')).read_text()),response)
                from types import SimpleNamespace
                with patch.dict(storage.os.environ,dict(CONNECT_MATRIX_STORAGE_CONTROL=str(directory),CONNECT_MATRIX_STORAGE_TOKEN=payload['token'])), \
                        patch.object(storage.uuid,'uuid4',return_value=SimpleNamespace(hex=identity)):
                    self.assertEqual(storage.request_host('observe',{}),dict(actual=True))
                    with self.assertRaises(ValueError):storage.request_host('observe',dict(changed=True))
                    self.assertEqual(source.read_bytes(),original)
                    source.unlink()  # Inject a stale reply whose request was lost.
                    with self.assertRaises(ValueError):storage.request_host('observe',dict(changed=True))
                    self.assertEqual(json.loads((directory/(identity+'.response.json')).read_text()),response)


    def test_wrong_run_directory_or_original_token_cannot_write_response(self):
        with tempfile.TemporaryDirectory() as temp:
            parent,run,payload=self.scope(temp)
            with patch.object(storage,'CONTROL_PARENT',parent):
                storage.control_action('bootstrap',payload)
                with self.assertRaises(ValueError):storage.control_action('snapshot',dict(payload,token='wrong'))
                with self.assertRaises(ValueError):storage.control_action('bootstrap',dict(payload,directory=str(parent/'unrelated')))
                self.assertFalse((parent/'unrelated').exists())

    def test_oversized_or_symlink_request_never_enters_original_copy_queue(self):
        with tempfile.TemporaryDirectory() as temp:
            parent,run,payload=self.scope(temp)
            with patch.object(storage,'CONTROL_PARENT',parent):
                storage.control_action('bootstrap',payload);directory=Path(payload['directory'])
                evidence=parent/'evidence';evidence.write_text('{}');request=directory/('d'*32+'.request.json');request.symlink_to(evidence)
                with self.assertRaises(ValueError):storage.control_action('snapshot',payload)
                self.assertEqual(evidence.read_text(),'{}')


class MacUnenteredCustody(unittest.TestCase):
    def test_queued_guest_storage_refusal_joins_only_real_lease_not_nonexistent_job(self):
        from types import SimpleNamespace
        from lib.mac_linux_fixture import MacLinuxFixture
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);fixture=MacLinuxFixture(root,root/'owner',{})
            fixture.storage_refused_before_entry=True
            fixture.call=lambda *args,**kwargs:b'true'
            fixture.lease=SimpleNamespace(stdin=SimpleNamespace(close=lambda:None),
                stdout=SimpleNamespace(close=lambda:None),stderr=SimpleNamespace(close=lambda:None),wait=lambda timeout:0)
            receipt=fixture.close()
            self.assertTrue(receipt['preflight_only']);self.assertFalse(receipt['guest_job_started'])
            self.assertFalse(receipt['original_joined']);self.assertTrue(receipt['unentered_custody_confirmed'])
            self.assertTrue(receipt['original_vm_lease_joined']);self.assertFalse(receipt['passed'])


class GuardianMetadataFailure(unittest.TestCase):
    def test_unknown_owners_keep_original_flock_when_metadata_or_reserve_cleanup_fails(self):
        import io
        from types import SimpleNamespace
        from lib import vm_workload_lease as leases
        class StopObservation(BaseException):pass
        for kind in ('fallback-unavailable','reserve-release-error'):
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as temp:
                state=dict(held=False,observed=False)
                class Lease:
                    directory=Path(temp)
                    def __enter__(self):state['held']=True;return self
                    def __exit__(self,*_):state['held']=False
                output=io.StringIO()
                def blocked(_):
                    self.assertTrue(state['held']);self.assertIn('blocked',output.getvalue());state['observed']=True
                    raise StopObservation()  # Ends this test observation only.
                reserve=SimpleNamespace(release=lambda:None)
                if kind=='reserve-release-error':reserve.release=lambda:(_ for _ in ()).throw(OSError('reserve mismatch'))
                with patch.object(leases,'VmWorkloadLease',return_value=Lease()), \
                        patch.object(leases,'MetadataReserve',return_value=reserve), \
                        patch.object(leases,'atomic_json',side_effect=OSError(errno.ENOSPC,'full') if kind=='fallback-unavailable' else None), \
                        patch.object(leases,'metadata_failure',side_effect=TimeoutError('coordinator unavailable')), \
                        patch.object(leases.sys,'stdin',SimpleNamespace(buffer=io.BytesIO(b'{"confirmed":false}'))), \
                        patch.object(leases.sys,'stdout',output),patch.object(leases.time,'sleep',side_effect=blocked):
                    with self.assertRaises(StopObservation):leases.guard()
                self.assertTrue(state['observed'])


class OriginalComponentFallback(unittest.TestCase):
    def report(self,joined=True):
        return dict(format='connect/transport-fixture-rehearsal/1',source_revision='a'*40,
            host=dict(platform='ubuntu24-arm64'),network_profile='local',signing_algorithm='EdDSA',
            passed=False,source_inputs_unchanged=True,executables_unchanged=True,sdk_dist_inputs_unchanged=False,
            failure=dict(phase='actual-original-phase',name='OriginalFailure'),servers=[dict(name='actual',original_joined=True)],
            native_programme_started=True,native_process_profile=dict(original_supervisor_joined=joined,
                original_workers_joined=joined),packet_capture=dict(owner_created=False))

    def test_failed_full_report_keeps_actual_join_source_and_failure_facts_only(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);reserve=storage.MetadataReserve(root/'reserve',65536);report=self.report()
            with patch.object(storage,'atomic_bytes',side_effect=OSError(errno.ENOSPC,'full')):
                # Patch the full-report operation only; the bounded atomic_json
                # fallback uses the original small-write implementation below.
                def saved(path,value,**kwargs):
                    Path(path).write_text(json.dumps(value))
                with patch.object(storage,'atomic_json',side_effect=saved):
                    with self.assertRaises(storage.ComponentStorageFailure) as caught:
                        storage.write_component(root/'rehearsal.json',report,reserve=reserve)
            value=caught.exception.fallback
            self.assertTrue(value['cleanup_evidence']['confirmed']);self.assertTrue(value['vm_fallback_saved'])
            self.assertFalse(value['full_report_saved']);self.assertFalse(value['full_media_verification_available'])
            self.assertFalse(value['original_passed']);self.assertFalse(value['original_source_flags']['sdk_dist_inputs_unchanged'])
            self.assertEqual(value['known_failure']['name'],'OriginalFailure')
            self.assertNotIn('decoded_media',value);self.assertFalse((root/'rehearsal.json').exists())

    def test_unknown_cleanup_stays_unknown_even_with_durable_host_fallback(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);reserve=storage.MetadataReserve(root/'reserve',65536)
            with patch.object(storage,'atomic_bytes',side_effect=OSError(errno.ENOSPC,'full')), \
                    patch.object(storage,'atomic_json',side_effect=OSError(errno.ENOSPC,'still full')), \
                    patch.object(storage,'metadata_failure',return_value=dict(host_durable=True)):
                with self.assertRaises(storage.ComponentStorageFailure) as caught:
                    storage.write_component(root/'rehearsal.json',self.report(False),reserve=reserve)
            value=caught.exception.fallback
            self.assertFalse(value['cleanup_evidence']['confirmed']);self.assertFalse(value['vm_fallback_saved'])
            self.assertTrue(value['host_fallback_saved']);self.assertFalse(value['cell_qualified'])
            lease=GuardedVmWorkloadLease();lease.complete_storage_failure(value)
            self.assertFalse(lease.evidence['confirmed'])

    def test_runner_preserves_joined_failure_without_fabricated_full_verification(self):
        loader=importlib.util.spec_from_file_location('storage_component_driver',ROOT/'scripts/run-gate6-transport-matrix.py')
        driver=importlib.util.module_from_spec(loader);loader.loader.exec_module(driver)
        cell_id='G6-058/ubuntu24-arm64/local/native-quic/EdDSA';events=[]
        fallback=dict(format='connect/original-component-storage-failure/1',source_revision='a'*40,cell_id=cell_id,
            full_report_saved=False,full_media_verification_available=False,cleanup_evidence=dict(confirmed=True),
            original_full_report_sha256='b'*64,known_failure=dict(name='OriginalFailure'))
        class Lease:
            blocked=False
            def __enter__(self):return self
            def complete_storage_failure(self,value):events.append(value)
            def __exit__(self,*_):events.append('released')
        cell=dict(id=cell_id,server='ubuntu24-arm64',status='not-run',required_checks=['media','authority'],
                  execution=dict(runnable=True,attempted=False))
        matrix=dict(source_revision='a'*40,requested_stage='full',execution_environment=dict(server='ubuntu24-arm64'),cells=[cell])
        def failed(_):raise storage.ComponentStorageFailure(fallback)
        with tempfile.TemporaryDirectory() as temp,patch.object(driver.platform,'system',return_value='Linux'), \
                patch.object(driver,'GuardedVmWorkloadLease',return_value=Lease()),patch.object(driver,'summarize'):
            driver.execute_selected(matrix,[cell_id],Path(temp),failed)
        self.assertEqual(events,[fallback,'released']);self.assertEqual(cell['status'],'failed')
        self.assertTrue(cell['execution']['attempted']);self.assertNotIn('component',cell['execution'])
        self.assertTrue(all(value['status']=='blocked' for value in cell['check_results'].values()))
        self.assertEqual(cell['execution']['storage_failure']['known_failure']['name'],'OriginalFailure')


class OriginalWorkerLoss(unittest.TestCase):
    def test_eof_and_broken_ack_pipe_keep_actual_isolated_guardian_flock(self):
        import fcntl
        import selectors
        import subprocess
        import time
        with tempfile.TemporaryDirectory() as temp:
            directory=Path(temp).resolve()
            code="""import sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from lib import vm_workload_lease as m
original=m.VmWorkloadLease
m.VmWorkloadLease=lambda:original(directory=Path(sys.argv[2]))
m.guard()
"""
            child=subprocess.Popen([sys.executable,'-B','-c',code,str(ROOT/'scripts'),str(directory)],
                stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,start_new_session=True)
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(child.stdout,selectors.EVENT_READ)
                    self.assertTrue(selector.select(timeout=5))
                    self.assertEqual(child.stdout.readline(),b'acquired\n')
                child.stdout.close();child.stdin.close()
                receipt=directory/('guardian-'+str(child.pid)+'.json');cutoff=time.monotonic()+5
                while not receipt.exists() and child.poll() is None and time.monotonic()<cutoff:time.sleep(.01)
                self.assertTrue(receipt.exists());self.assertFalse(json.loads(receipt.read_text())['cleanup']['confirmed'])
                # Give the failed acknowledgement its turn; EOF never grants a
                # successful workload receipt or releases the original flock.
                time.sleep(.1);self.assertIsNone(child.poll())
                with (directory/'lock').open('r+') as lock:
                    with self.assertRaises(BlockingIOError):fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            finally:
                if child.poll() is None:child.terminate()
                child.wait(timeout=5)
                if child.stdout and not child.stdout.closed:child.stdout.close()
                if child.stdin and not child.stdin.closed:child.stdin.close()


class BoundedArchiveBackpressure(unittest.TestCase):
    def test_failed_closed_archive_stops_next_expansion_without_rewriting_behavior(self):
        loader=importlib.util.spec_from_file_location('storage_queue_driver',ROOT/'scripts/run-gate6-transport-matrix.py')
        driver=importlib.util.module_from_spec(loader);loader.loader.exec_module(driver)
        cells=[dict(id='original-'+signing,server='macos-arm64',network='local',transport='native-quic',signing=signing,
                    status='not-run',required_checks=['media'],execution=dict(runnable=True,attempted=False))
               for signing in ('EdDSA','ES256')]
        matrix=dict(source_revision='a'*40,requested_stage='full',execution_environment=dict(server='macos-arm64'),cells=cells)
        visited=[]
        def execute(cell):visited.append(cell['id']);return {},'b'*64,0
        def evaluated(cell,*args,**kwargs):cell.update(status='passed',check_results=dict(media=dict(status='passed')))
        with tempfile.TemporaryDirectory() as temp:
            output=Path(temp);(output/'cells/macos-arm64/local/native-quic/EdDSA').mkdir(parents=True)
            with patch.object(driver.platform,'system',return_value='Darwin'),patch.object(driver,'summarize'), \
                    patch.object(driver,'evaluate_cell',side_effect=evaluated), \
                    patch('lib.matrix_archive.archive_joined',side_effect=OSError(errno.ENOSPC,'full')):
                driver.execute_selected(matrix,[c['id'] for c in cells],output,execute)
        self.assertEqual(visited,['original-EdDSA']);self.assertEqual(cells[0]['status'],'passed')
        self.assertEqual(cells[0]['check_results']['media']['status'],'passed')
        self.assertEqual(cells[0]['execution']['archive_failure'],'OSError')
        self.assertEqual(cells[1]['status'],'blocked');self.assertFalse(cells[1]['execution']['attempted'])
