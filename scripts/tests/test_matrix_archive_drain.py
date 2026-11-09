"""Small local archive custody controls, with original SSH replaced by fakes."""
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from lib import matrix_archive_drain as drain


def joined_report(joined=True,forced=False):
    return dict(format='connect/transport-fixture-rehearsal/1',passed=False,
                servers=[dict(original_joined=True)],native_programme_started=True,
                native_process_profile=dict(original_supervisor_joined=joined,original_workers_joined=joined,forced=forced),
                packet_capture=dict(owner_created=False), conversation_control=dict(all_observed_helpers_joined=True))


class DurableArchiveCustody(unittest.TestCase):
    def fixture(self,root,*,owners=True,stream_status=0,corrupt=False,forced=False,report=None):
        body=json.dumps(joined_report(owners,forced) if report is None else report).encode();report_sha=hashlib.sha256(body).hexdigest()
        packed=io.BytesIO()
        with tarfile.open(fileobj=packed,mode='w:gz') as archive:
            member=tarfile.TarInfo('evidence/media/rehearsal.json');member.size=len(body);archive.addfile(member,io.BytesIO(body))
        data=packed.getvalue();sha=hashlib.sha256(data).hexdigest()
        original=dict(device=1,inode=2,bytes=len(data),mode=0o600,mtime_ns=1000000000,ctime_ns=1000000000)
        receipt=dict(archive_sha256=sha,archive_bytes=len(data))
        calls=[]
        class Process:
            pid=2147483647
            returncode=stream_status
            def __init__(self,command,**kwargs):
                kwargs['stdout'].write(data+b'bad' if corrupt else data)
            def communicate(self,raw,timeout):return None,b''
        owner=drain.ArchiveDrain(root/'spool','/workspace/.local/g6-058-original','a'*40,popen=Process)
        def command(action,payload):
            calls.append(action)
            if action=='inspect':return dict(signature=original,receipt=receipt,member='evidence/media/rehearsal.json')
            self.assertEqual(action,'unlink');self.assertTrue((root/'spool/ubuntu24-arm64/local/native-quic/EdDSA.tar.durable.json').exists())
            return dict(original_archive_absent=True)
        owner.json_command=command
        payload=dict(run_directory=owner.run_directory,source_revision=owner.revision,archive_signature=original,
                     cell=dict(id='G6-058/ubuntu24-arm64/local/native-quic/EdDSA',execution=dict(component=dict(report_sha256=report_sha))))
        return owner,payload,calls,data

    def test_failed_media_with_real_joined_owners_is_retained_before_original_unlink(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);owner,payload,calls,data=self.fixture(root)
            result=owner.drain(payload);self.assertEqual(calls,['inspect','unlink'])
            copy=Path(result['host_archive']);self.assertEqual(copy.read_bytes(),data)
            self.assertEqual(copy.stat().st_mtime_ns,1000000000);self.assertEqual(stat.S_IMODE(copy.stat().st_mode),0o600)
            self.assertTrue(result['host_ack']['all_original_owners_joined']);self.assertEqual(len(owner.completed),1)
            # The functional failure is still the original report's value.
            with tarfile.open(copy,'r:gz') as archive:self.assertFalse(json.load(archive.extractfile('evidence/media/rehearsal.json'))['passed'])

    def test_stream_failure_keeps_vm_original_and_partial_host_copy(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,_=self.fixture(Path(temp),stream_status=1)
            with self.assertRaises(RuntimeError):owner.drain(payload)
            self.assertEqual(calls,['inspect']);self.assertEqual(owner.completed,[])

    def test_wrong_archive_hash_never_acknowledges_or_unlinks(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,_=self.fixture(Path(temp),corrupt=True)
            with self.assertRaises(ValueError):owner.drain(payload)
            self.assertEqual(calls,['inspect'])

    def test_unjoined_original_owner_never_acknowledges_or_unlinks(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,_=self.fixture(Path(temp),owners=False)
            with self.assertRaises(ValueError):owner.drain(payload)
            self.assertEqual(calls,['inspect'])

    def test_forced_original_cleanup_is_not_natural_join_qualification(self):
        report=joined_report();report['servers']=[dict(original_joined=False,original_process_group_joined=True,
            original_exit_code=1,original_pid=42)]
        self.assertTrue(drain.original_owners_joined(report))
        report['servers'][0]['original_process_group_joined']=False
        self.assertFalse(drain.original_owners_joined(report))
        for field,value in [('browser_forced_cleanup',True),('client_network_removed',False),
                            ('owned_audio',dict(original_process_joined=False))]:
            report=joined_report();report[field]=value
            self.assertFalse(drain.original_owners_joined(report),field)
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,_=self.fixture(Path(temp),forced=True)
            with self.assertRaises(ValueError):owner.drain(payload)
            self.assertEqual(calls,['inspect'])

    def test_failed_nested_helper_is_archived_only_after_its_natural_original_anchor_physically_joins(self):
        report = joined_report()
        report['conversation_control']['all_observed_helpers_joined'] = False
        report['conversation_control']['capture_interpreter'] = dict(
            joined=True, forced=True, exit_code=None, signal='SIGKILL')
        report['native_process_profile'].update(
            format='connect/native-workload-process-profile/1',
            exit_code=1, failure=None, collection_complete=True)
        self.assertTrue(drain.original_owners_joined(report))
        with tempfile.TemporaryDirectory() as temp:
            owner, payload, calls, _ = self.fixture(Path(temp), report=report)
            result = owner.drain(payload)
            self.assertEqual(calls, ['inspect', 'unlink'])
            self.assertTrue(result['original_archive_absent'])
            with tarfile.open(result['host_archive'], 'r:gz') as archive:
                stored = json.load(archive.extractfile('evidence/media/rehearsal.json'))
            self.assertEqual(stored, report)
            self.assertFalse(stored['passed'])
            self.assertFalse(stored['conversation_control']['all_observed_helpers_joined'])
            self.assertTrue(stored['conversation_control']['capture_interpreter']['forced'])

    def test_failed_logical_helper_without_complete_physical_anchor_never_acknowledges(self):
        for field, value in [
                ('format', 'unknown'),
                ('original_supervisor_joined', False),
                ('original_workers_joined', False),
                ('forced', True),
                ('collection_complete', False),
                ('failure', 'unavailable'),
                ('exit_code', None),
                ('exit_code', True)]:
            with self.subTest(field=field, value=value):
                report = joined_report()
                report['conversation_control']['all_observed_helpers_joined'] = False
                report['native_process_profile'].update(
                    format='connect/native-workload-process-profile/1',
                    exit_code=1, failure=None, collection_complete=True)
                report['native_process_profile'][field] = value
                self.assertFalse(drain.original_owners_joined(report))
                with tempfile.TemporaryDirectory() as temp:
                    owner, payload, calls, _ = self.fixture(Path(temp), report=report)
                    with self.assertRaises(ValueError):
                        owner.drain(payload)
                    self.assertEqual(calls, ['inspect'])

    def test_failed_exact_destination_mtime_never_unlinks(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,_=self.fixture(Path(temp))
            with patch.object(drain.os,'utime'):
                with self.assertRaises(ValueError):owner.drain(payload)
            self.assertEqual(calls,['inspect'])

    def test_bounded_pending_queue_rejects_oversized_original(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,_=self.fixture(Path(temp));payload['archive_signature']['bytes']=drain.MAX_PENDING_BYTES+1
            with self.assertRaises(ValueError):owner.drain(payload)
            self.assertEqual(calls,['inspect'])

    def test_wrong_source_or_run_cannot_enter_drain(self):
        with tempfile.TemporaryDirectory() as temp:
            owner,payload,calls,_=self.fixture(Path(temp));payload['source_revision']='b'*40
            with self.assertRaises(ValueError):owner.drain(payload)
            self.assertEqual(calls,[])


class ExactRemoteRetirement(unittest.TestCase):
    def test_missing_durable_ack_keeps_original_even_when_report_is_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp).resolve();path=root/'EdDSA.tar.gz';path.write_bytes(b'original');sidecar=root/'EdDSA.archive.json';sidecar.write_text('{}')
            observed=drain.signature(path);receipt=dict(archive_sha256=drain.digest(path))
            payload=dict(archive_signature=observed,sidecar_sha256=drain.digest(sidecar),durable_ack={},cell=dict(execution=dict(component=dict(report_sha256='a'*64))))
            with patch.object(drain,'validate_remote',return_value=(path,sidecar,receipt,'member',observed)):
                with self.assertRaises((KeyError,ValueError)):drain.remote_action('unlink',payload)
            self.assertEqual(path.read_bytes(),b'original');self.assertTrue(sidecar.exists())

    def test_changed_original_inode_never_uses_previous_custody_ack(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp).resolve();path=root/'archive';path.write_bytes(b'original');sidecar=root/'receipt';sidecar.write_text('{}')
            observed=drain.signature(path);old=dict(observed,inode=observed['inode']+1)
            with patch.object(drain,'validate_remote',return_value=(path,sidecar,{},'member',observed)):
                with self.assertRaises(ValueError):drain.remote_action('unlink',dict(archive_signature=old,sidecar_sha256=drain.digest(sidecar)))
            self.assertTrue(path.exists())


class ActualRemoteReceiptGuards(unittest.TestCase):
    def original(self,temp):
        parent=Path(temp).resolve();root=parent/('g6-058-'+'c'*32);root.mkdir()
        directory=root/'cells/ubuntu24-arm64/local/native-quic';directory.mkdir(parents=True)
        archive=directory/'EdDSA.tar.gz';archive.write_bytes(b'small-original-archive')
        sha=drain.digest(archive);report_sha='d'*64
        cell=dict(id='G6-058/ubuntu24-arm64/local/native-quic/EdDSA',server='ubuntu24-arm64',network='local',
                  transport='native-quic',signing='EdDSA',status='failed',
                  verifier=dict(integrity=True,report_sha256=report_sha),
                  execution=dict(component=dict(report_sha256=report_sha),evidence_archive=dict(path=str(archive),
                      sha256=sha,bytes=archive.stat().st_size,verified_before_removal=True)))
        sidecar=directory/'EdDSA.archive.json';sidecar.write_text(json.dumps(dict(archive_sha256=sha,
            archive_bytes=archive.stat().st_size,verified_before_removal=True,
            entries={'evidence/media/rehearsal.json':dict(sha256=report_sha)})))
        (root/'matrix.json').write_text(json.dumps(dict(source_revision='a'*40,cells=[cell])))
        payload=dict(run_directory=str(root),source_revision='a'*40,cell=cell,
                     archive_signature=drain.signature(archive),sidecar_sha256=drain.digest(sidecar))
        ack=dict(sha256=sha,bytes=archive.stat().st_size,mode=payload['archive_signature']['mode'],
                 mtime_ns=payload['archive_signature']['mtime_ns'],host_file_synchronized=True,
                 host_directory_synchronized=True,original_stream_joined=True,all_original_owners_joined=True,
                 host_report_sha256=report_sha)
        return parent,root,archive,sidecar,payload,ack

    def test_actual_guard_preserves_sidecar_and_terminal_failed_cell_after_exact_unlink(self):
        with tempfile.TemporaryDirectory() as temp:
            parent,root,archive,sidecar,payload,ack=self.original(temp);matrix=(root/'matrix.json').read_bytes()
            with patch.object(drain,'RUN_PARENT',parent):result=drain.remote_action('unlink',dict(payload,durable_ack=ack))
            self.assertTrue(result['original_archive_absent']);self.assertFalse(archive.exists())
            self.assertTrue(sidecar.exists());self.assertEqual((root/'matrix.json').read_bytes(),matrix)
            self.assertTrue(sidecar.with_name('EdDSA.archive.custody-pre.json').exists())

        # A dangling replacement is present even though Path.exists is false.
        with tempfile.TemporaryDirectory() as temp:
            parent,root,archive,sidecar,payload,ack=self.original(temp)
            original_unlink=Path.unlink;replacement=root/'replacement-target-missing'
            def replace_after_unlink(path,*args,**kwargs):
                original_unlink(path,*args,**kwargs)
                if path==archive:path.symlink_to(replacement)
            with patch.object(drain,'RUN_PARENT',parent),patch.object(Path,'unlink',new=replace_after_unlink),self.assertRaises(ValueError):
                drain.remote_action('unlink',dict(payload,durable_ack=ack))
            self.assertTrue(archive.is_symlink());self.assertFalse(archive.exists())
            self.assertEqual(os.readlink(archive),str(replacement))
            post=json.loads(sidecar.with_name('EdDSA.archive.custody-post.json').read_text())
            self.assertFalse(post['original_archive_absent']);self.assertTrue(post['original_replacement_preserved'])

    def test_changed_terminal_dictionary_or_receipt_blocks_exact_retirement(self):
        for changed in ('cell','sidecar','owner-ack','raw-replacement'):
            with self.subTest(changed=changed),tempfile.TemporaryDirectory() as temp:
                parent,root,archive,sidecar,payload,ack=self.original(temp)
                if changed=='cell':payload['cell']['status']='passed'
                elif changed=='sidecar':sidecar.write_text('{}')
                elif changed=='owner-ack':ack['all_original_owners_joined']=False
                else:(archive.parent/'EdDSA').symlink_to(root/'missing-raw-target')
                with patch.object(drain,'RUN_PARENT',parent),self.assertRaises((ValueError,KeyError)):
                    drain.remote_action('unlink',dict(payload,durable_ack=ack))
                self.assertTrue(archive.exists())


class OriginalArchiveCreation(unittest.TestCase):
    def test_joined_nonzero_exit_and_hardlink_contents_are_durable_without_changing_failure(self):
        from lib import matrix_archive
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp).resolve();directory=root/'cell';(directory/'evidence/media').mkdir(parents=True)
            report=joined_report();report['servers']=[dict(original_joined=False,original_process_group_joined=True,
                original_exit_code=1,original_pid=42)]
            report_path=directory/'evidence/media/rehearsal.json';report_path.write_text(json.dumps(report))
            first=directory/'sealed-a';first.write_bytes(b'exact-original-binary');os.link(first,directory/'sealed-b')
            def before_archive(path,required):
                self.assertEqual(path,directory);self.assertTrue(directory.exists())
                self.assertFalse((root/'cell.tar.gz').exists());self.assertGreater(required,0)
            with patch.object(matrix_archive,'preflight',side_effect=before_archive) as admission:
                result=matrix_archive.archive_joined(directory,report)
            self.assertFalse(directory.exists());self.assertTrue(result['verified_before_removal'])
            admission.assert_called_once()
            with tarfile.open(result['path'],'r:gz') as archive:
                self.assertTrue(archive.getmember('sealed-a').isfile());self.assertTrue(archive.getmember('sealed-b').isfile())
                self.assertEqual(archive.extractfile('sealed-a').read(),archive.extractfile('sealed-b').read())
                saved=json.load(archive.extractfile('evidence/media/rehearsal.json'))
            self.assertFalse(saved['passed']);self.assertEqual(saved['servers'][0]['original_exit_code'],1)
            self.assertTrue((root/'cell.archive.json').exists())
