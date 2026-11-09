"""Closed guest retention uses tiny files and fake transport; no VM/media."""
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from lib import matrix_archive_drain as drain
from lib import matrix_storage as storage
from lib import mac_linux_guest as guest


class GuestSelection(unittest.TestCase):
    def test_staged_inputs_source_and_report_metadata_remain_at_original_paths(self):
        inputs={'private/audio/owned.wav':'original','scripts/input.py':'original'}
        for name in ['private/audio/owned.wav','scripts/input.py','sdk/library.ogg','job.json','output/guest-owner.json','output/browser-observation.json']:
            self.assertFalse(drain.guest_candidate(name,inputs),name)
        for name in ['.local/bin/connect','.local/bin/connect-external-rust-journey','private/media.pcap',
                     'private/av-sources/source-0.json','output/received-0.json','output/received-0.f32']:
            self.assertTrue(drain.guest_candidate(name,inputs),name)

    def test_traversal_and_nonpayload_json_never_enter_retirement_list(self):
        for name in ['../../file.pcap','/absolute.pcap','output/plan.json','output/source-9.json','output/operation.json']:
            self.assertFalse(drain.guest_candidate(name,{}),name)


class OriginalGuestRetirement(unittest.TestCase):
    def fixture(self,temp):
        parent=Path(temp).resolve();root=parent/('mac-remote-'+'c'*32);root.mkdir()
        output=root/'output';output.mkdir();(root/'.owners').mkdir()
        job=dict(run_id=root.name,source_revision='a'*40,output='output',container=None)
        (root/'job.json').write_text(json.dumps(job));(root/'job-intent.json').write_text(json.dumps(dict(run_id=root.name)))
        actual=dict(format='connect/mac-linux-original-job/1',run_id=root.name,original_joined=True,
                    original_pid=2147483646,original_start_ticks=123,exit_code=1,forced=False,capture_original_joined=True)
        for path in [root/'job-state.json',output/'guest-owner.json']:path.write_text(json.dumps(actual))
        (root/'lease-state.json').write_text(json.dumps(dict(state='released',original_owners_joined=True)))
        (root/'scripts').mkdir();(root/'scripts/input.py').write_text('original')
        receipt=dict(run_id=root.name,original_joined=True,all_guest_owners_joined=True,original_vm_lease_joined=True,
                     forced=False,cleanup_failures=[],client_inputs_unchanged=True,guest=actual)
        inputs={name:drain.digest(root/name) for name in ['job.json','job-intent.json','scripts/input.py']}
        for name in ['.local/bin/connect','output/media.pcap','output/received-0.json','private/av-sources/source-0.json']:
            path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(('original '+name).encode())
        storage.initialize_guest_reserve(root)
        payload=dict(guest_root=str(root),source_revision='a'*40,guest_receipt=receipt,client_inputs=inputs,
                     original_binary_hashes={'.local/bin/connect':drain.digest(root/'.local/bin/connect')})
        return parent,root,payload

    def test_exact_durable_payload_retirement_preserves_all_original_reports_and_inputs(self):
        with tempfile.TemporaryDirectory() as temp:
            parent,root,payload=self.fixture(temp);report=(root/'output/guest-owner.json').read_bytes()
            with patch.object(drain,'GUEST_PARENT',parent),patch.object(guest,'group_alive',return_value=False):
                observed=drain.guest_action('inspect',payload);self.assertEqual(len(observed['files']),4)
                ack=dict(files=observed['files'],all_original_files_verified=True,original_stream_joined=True,
                         host_file_synchronized=True,host_directory_synchronized=True)
                result=drain.guest_action('unlink',dict(payload,files=observed['files'],durable_ack=ack))
            self.assertTrue(result['all_selected_absent']);self.assertEqual((root/'output/guest-owner.json').read_bytes(),report)
            for name,sha in payload['client_inputs'].items():self.assertEqual(drain.digest(root/name),sha)
            self.assertTrue((root/'closed-payload-custody-pre.json').exists());self.assertTrue((root/'closed-payload-custody-post.json').exists())

    def test_unknown_lease_forced_job_or_helper_never_selects_original_payloads(self):
        for failure in ['lease','job','helper']:
            with self.subTest(failure=failure),tempfile.TemporaryDirectory() as temp:
                parent,root,payload=self.fixture(temp)
                if failure=='lease':(root/'lease-state.json').write_text('{"state":"blocked"}')
                elif failure=='job':payload['guest_receipt']['forced']=True
                else:
                    (root/'.owners/a-intent.json').write_text('{}')
                    (root/'.owners/a-receipt.json').write_text(json.dumps(dict(original_joined=True,original_pid=42,forced=True)))
                with patch.object(drain,'GUEST_PARENT',parent),patch.object(guest,'group_alive',return_value=False),self.assertRaises(ValueError):
                    drain.guest_action('inspect',payload)
                self.assertTrue((root/'.local/bin/connect').exists())

    def test_changed_payload_aborts_entire_retirement_before_first_unlink(self):
        with tempfile.TemporaryDirectory() as temp:
            parent,root,payload=self.fixture(temp)
            with patch.object(drain,'GUEST_PARENT',parent),patch.object(guest,'group_alive',return_value=False):
                observed=drain.guest_action('inspect',payload);(root/'output/media.pcap').write_bytes(b'changed')
                ack=dict(files=observed['files'],all_original_files_verified=True,original_stream_joined=True,
                         host_file_synchronized=True,host_directory_synchronized=True)
                with self.assertRaises(ValueError):drain.guest_action('unlink',dict(payload,files=observed['files'],durable_ack=ack))
            self.assertTrue((root/'.local/bin/connect').exists());self.assertTrue((root/'output/media.pcap').exists())

        # A replacement created after the original unlink is observed and kept;
        # successful unlink calls alone cannot prove final path absence.
        with tempfile.TemporaryDirectory() as temp:
            parent,root,payload=self.fixture(temp);target=root/'output/media.pcap'
            with patch.object(drain,'GUEST_PARENT',parent),patch.object(guest,'group_alive',return_value=False):
                observed=drain.guest_action('inspect',payload)
                ack=dict(files=observed['files'],all_original_files_verified=True,original_stream_joined=True,
                         host_file_synchronized=True,host_directory_synchronized=True)
                original_unlink=Path.unlink
                def replace_after_unlink(path,*args,**kwargs):
                    original_unlink(path,*args,**kwargs)
                    if path==target:path.write_bytes(b'replacement-inode-must-survive')
                with patch.object(Path,'unlink',new=replace_after_unlink),self.assertRaises(ValueError):
                    drain.guest_action('unlink',dict(payload,files=observed['files'],durable_ack=ack))
            self.assertEqual(target.read_bytes(),b'replacement-inode-must-survive')
            self.assertFalse(json.loads((root/'closed-payload-custody-post.json').read_text())['all_selected_absent'])

    def test_wrong_input_or_original_binary_hash_never_admits_retirement(self):
        for kind in ['input','binary']:
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as temp:
                parent,root,payload=self.fixture(temp)
                if kind=='input':payload['client_inputs']['scripts/input.py']='0'*64
                else:payload['original_binary_hashes']['.local/bin/connect']='0'*64
                with patch.object(drain,'GUEST_PARENT',parent),patch.object(guest,'group_alive',return_value=False),self.assertRaises(ValueError):
                    drain.guest_action('inspect',payload)
                self.assertTrue((root/'.local/bin/connect').exists())


class DurableHostGuestCopy(unittest.TestCase):
    def fixture(self,temp,corrupt=False,*,nested=False):
        root=Path(temp).resolve();matrix_root=root/'matrix';matrix_root.mkdir()
        owner=drain.GuestDrain(matrix_root,'/workspace/.local/g6-058-'+'a'*32,'a'*40)
        data=b'original closed media payload';row=dict(sha256=hashlib.sha256(data).hexdigest(),
            signature=dict(device=1,inode=2,bytes=len(data),mode=0o600,mtime_ns=1234567890,ctime_ns=100))
        files={'output/media.pcap':row,'.local/bin/connect':dict(row,signature=dict(row['signature'],mode=0o700)),
               '.local/bin/connect-external-rust-journey':dict(row,signature=dict(row['signature'],mode=0o700))};guest_id='mac-remote-'+'c'*32
        report=dict(format='connect/transport-fixture-rehearsal/1',source_revision='a'*40,passed=False,
            servers=[dict(original_joined=True)],native_programme_started=True,
            native_process_profile=dict(original_supervisor_joined=True,original_workers_joined=True),
            conversation_control=dict(all_observed_helpers_joined=True),packet_capture=dict(owner_created=False),
            client_binary_sha256=row['sha256'],peer_binary_sha256=row['sha256'],
            remote_client_owner=dict(format='connect/mac-linux-owner/1',run_id=guest_id,original_joined=True,
                all_guest_owners_joined=True,original_vm_lease_joined=True,forced=False,cleanup_failures=[],client_inputs_unchanged=True,
                guest=dict(format='connect/mac-linux-original-job/1',run_id=guest_id,original_pid=42,original_start_ticks=100,
                           exit_code=1,forced=False,original_joined=True)))
        if nested:
            report['remote_client_owner']['guest'].update(source_revision='a'*40,
                capture_owner_created=True,capture_original_joined=True)
            report['native_process_profile']=dict(format='connect/mac-linux-native-process-resources/1',
                collection_complete=True,original_owner=report['remote_client_owner'])
            report['packet_capture']=dict(owner_created=True,original_joined=True,pcap_sha256=row['sha256'])
        directory=matrix_root/'macos-arm64/cells/macos-arm64/remote-profile-fixture/native-quic/EdDSA'
        (directory/'evidence/media').mkdir(parents=True);path=directory/'evidence/media/rehearsal.json';path.write_text(json.dumps(report))
        private=directory/'private/media/remote-owner';private.mkdir(parents=True);(private/'client-inputs.json').write_text('{}')
        cell=dict(id='G6-058/macos-arm64/remote-profile-fixture/native-quic/EdDSA',server='macos-arm64',
            network='remote-profile-fixture',transport='native-quic',signing='EdDSA',status='failed',
            verifier=dict(integrity=True),execution=dict(component=dict(report_sha256=drain.digest(path))))
        (matrix_root/'macos-arm64/matrix.json').write_text(json.dumps(dict(source_revision='a'*40,cells=[cell])))
        raw=io.BytesIO()
        with tarfile.open(fileobj=raw,mode='w:') as archive:
            for name,item in files.items():
                member=tarfile.TarInfo(name);member.size=len(data);member.mode=item['signature']['mode']
                archive.addfile(member,io.BytesIO(b'x'*len(data) if corrupt else data))
        calls=[]
        class Process:
            pid=2147483647;returncode=0
            def __init__(self,*args,**kwargs):kwargs['stdout'].write(raw.getvalue())
            def communicate(self,data,timeout):return None,b''
            def poll(self):return self.returncode
            def wait(self,timeout):return self.returncode
        owner.popen=Process
        def command(action,payload):
            calls.append(action)
            if action=='inspect':return dict(files=files)
            self.assertEqual(action,'unlink');self.assertTrue((owner.destination/guest_id/'durable-ack.json').exists())
            self.assertEqual(payload['durable_ack']['files'],files)
            return dict(all_selected_absent=True,removed=list(files))
        owner.json_command=command
        return owner,dict(source_revision='a'*40,cell=cell,component_directory=str(directory)),calls,row

    def test_original_failed_guest_payload_has_durable_exact_copy_before_retirement(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,row=self.fixture(temp)
            with patch.object(storage,'volume',return_value=dict(free_bytes=100*storage.GIB)):
                result=owner.drain(payload)
            self.assertEqual(calls,['inspect','unlink']);self.assertTrue(result['all_selected_absent'])
            path=Path(result['host_destination'])/'output/media.pcap'
            self.assertEqual(drain.digest(path),row['sha256']);self.assertEqual(path.stat().st_mtime_ns,row['signature']['mtime_ns'])
            self.assertEqual(len(owner.completed),1);self.assertTrue(owner.transports[0]['original_joined'])

    def test_nested_remote_native_custody_reaches_normal_drain_and_preserves_failed_report(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,row=self.fixture(temp,nested=True)
            report_path=Path(payload['component_directory'])/'evidence/media/rehearsal.json'
            original=report_path.read_bytes()
            with patch.object(storage,'volume',return_value=dict(free_bytes=100*storage.GIB)):
                result=owner.drain(payload)
            self.assertEqual(calls,['inspect','unlink'])
            self.assertTrue(result['all_selected_absent'])
            self.assertEqual(report_path.read_bytes(),original)
            self.assertEqual(payload['cell']['status'],'failed')
            report=json.loads(original)
            self.assertFalse(report['passed'])
            self.assertEqual(report['remote_client_owner']['guest']['exit_code'],1)
            self.assertNotIn('original_workers_joined',report['native_process_profile'])

    def test_nested_remote_native_unknown_or_forced_owner_never_enters_guest_transport(self):
        for defect in ('forced','missing','foreign-source','wrong-format'):
            with self.subTest(defect=defect),tempfile.TemporaryDirectory() as temp:
                owner,payload,calls,row=self.fixture(temp,nested=True)
                report_path=Path(payload['component_directory'])/'evidence/media/rehearsal.json'
                report=json.loads(report_path.read_text())
                profile=report['native_process_profile']
                if defect=='forced':
                    profile['original_owner']['guest']['forced']=True
                elif defect=='missing':
                    del profile['original_owner']['original_vm_lease_joined']
                elif defect=='foreign-source':
                    profile['original_owner']['guest']['source_revision']='b'*40
                else:
                    profile['format']='connect/unknown-profile/1'
                    profile.update(original_workers_joined=True,original_supervisor_joined=True)
                report_path.write_text(json.dumps(report))
                payload['cell']['execution']['component']['report_sha256']=drain.digest(report_path)
                matrix_path=owner.matrix_output/'macos-arm64/matrix.json'
                matrix_path.write_text(json.dumps(dict(source_revision='a'*40,cells=[payload['cell']])))
                original=report_path.read_bytes()
                with self.assertRaisesRegex(ValueError,'All original component owners must join'):
                    owner.drain(payload)
                self.assertEqual(calls,[])
                self.assertEqual(owner.completed,[])
                self.assertEqual(report_path.read_bytes(),original)
                self.assertEqual(payload['cell']['status'],'failed')

    def test_bad_payload_or_spool_pressure_keeps_original_without_acknowledgement(self):
        for failure in ['hash','spool']:
            with self.subTest(failure=failure),tempfile.TemporaryDirectory() as temp:
                owner,payload,calls,row=self.fixture(temp,corrupt=failure=='hash')
                with patch.object(storage,'volume',return_value=dict(free_bytes=100*storage.GIB if failure=='hash' else 0)),self.assertRaises(ValueError):
                    owner.drain(payload)
                self.assertEqual(calls,['inspect']);self.assertEqual(owner.completed,[])

    def test_wrong_destination_mtime_keeps_original_without_acknowledgement(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,row=self.fixture(temp)
            with patch.object(storage,'volume',return_value=dict(free_bytes=100*storage.GIB)),patch.object(drain.os,'utime'),self.assertRaises(ValueError):
                owner.drain(payload)
            self.assertEqual(calls,['inspect'])

    def test_changed_original_component_hash_never_enters_guest_transport(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,row=self.fixture(temp);payload['cell']['execution']['component']['report_sha256']='0'*64
            with self.assertRaises(ValueError):owner.drain(payload)
            self.assertEqual(calls,[])
