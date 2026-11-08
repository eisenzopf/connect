"""Original coordinator owns lossless, acknowledged closed ARM archive custody."""
import hashlib
import json
import os
import signal
from pathlib import Path
import stat
import subprocess
import sys
import tarfile

try:
    from .matrix_storage import atomic_json, request_host, sync_directory, sync_file, sync_ancestors
    from .vm_workload_lease import cleanup_evidence
except ImportError:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from lib.matrix_storage import atomic_json, request_host, sync_directory, sync_file, sync_ancestors
    from lib.vm_workload_lease import cleanup_evidence

RUN_PARENT = Path('/workspace/.local')
MAX_PENDING_BYTES = 12 * 1024**3  # One original archive, no unbounded copy queue.


def original_owners_joined(report):
    if cleanup_evidence(report)['confirmed'] is not True:
        return False
    if not report.get('servers'):
        return False
    if report.get('format') != 'connect/browser-matrix-component/1':
        if report.get('conversation_control', {}).get('all_observed_helpers_joined') is not True:
            return False
    if report.get('browser_forced_cleanup', False) is not False:
        return False
    if 'client_network_removed' in report and report['client_network_removed'] is not True:
        return False
    audio = report.get('owned_audio', {})
    if 'original_process_joined' in audio and audio['original_process_joined'] is not True:
        return False
    records = [report.get('native_process_profile', {}), report.get('failure_cleanup', {}),
               report.get('packet_capture', {}), audio, *report.get('servers', [])]
    records.extend(value.get('process_owner', {}) for name,value in report.get('current_authority', {}).items()
                   if name in {'expiry','revocation'} and isinstance(value,dict))
    return all(value.get('forced', False) is False and value.get('forced_cleanup', False) is False
               for value in records)


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def path_absent(path):
    try:
        Path(path).lstat()
    except FileNotFoundError:
        return True
    return False


def signature(path):
    value = path.lstat()
    if not stat.S_ISREG(value.st_mode) or value.st_nlink != 1 or path.resolve() != path:
        raise ValueError('Original ordinary archive inode required')
    return dict(device=value.st_dev, inode=value.st_ino, bytes=value.st_size,
                mode=stat.S_IMODE(value.st_mode), mtime_ns=value.st_mtime_ns,
                ctime_ns=value.st_ctime_ns)


def validate_remote(payload):
    root = Path(payload['run_directory'])
    if (root.parent != RUN_PARENT or len(root.name) != 39 or not root.name.startswith('g6-058-')
            or any(c not in '0123456789abcdef' for c in root.name[7:])
            or root.resolve() != root):
        raise ValueError('Original ARM matrix directory required')
    matrix = json.loads((root / 'matrix.json').read_text())
    cell = payload['cell']
    if (matrix['source_revision'] != payload['source_revision'] or cell['server'] != 'ubuntu24-arm64'
            or cell['status'] not in {'passed', 'failed', 'blocked'}
            or cell.get('verifier',{}).get('integrity') is not True
            or cell['verifier'].get('report_sha256') != cell['execution']['component']['report_sha256']
            or next(v for v in matrix['cells'] if v['id'] == cell['id']) != cell):
        raise ValueError('Exact original terminal cell required')
    if (cell['network'] not in {'local', 'remote-profile-fixture'}
            or cell['transport'] not in {'native-quic','chrome-webtransport','chrome-wss-webrtc','firefox-wss-webrtc'}
            or cell['signing'] not in {'EdDSA','ES256'}
            or cell['id'] != '/'.join(['G6-058',cell['server'],cell['network'],cell['transport'],cell['signing']])):
        raise ValueError('Original selected ARM cell identity required')
    expected = cell['execution']['evidence_archive']
    path = root / 'cells' / cell['server'] / cell['network'] / cell['transport'] / (cell['signing'] + '.tar.gz')
    if expected['path'] != str(path) or expected['verified_before_removal'] is not True:
        raise ValueError('Original archive binding required')
    sidecar = path.with_name(cell['signing'] + '.archive.json')
    if sidecar.resolve() != sidecar or sidecar.is_symlink():
        raise ValueError('Original archive receipt required')
    receipt = json.loads(sidecar.read_text())
    member = 'component.json' if cell['transport'] != 'native-quic' else 'evidence/media/rehearsal.json'
    if (receipt['archive_sha256'] != expected['sha256'] or receipt['archive_bytes'] != expected['bytes']
            or receipt['verified_before_removal'] is not True
            or receipt['entries'][member]['sha256'] != cell['execution']['component']['report_sha256']
            or not path_absent(path.parent / cell['signing'])):
        raise ValueError('Original verified closed archive receipt differs')
    observed = signature(path)
    if observed['bytes'] != expected['bytes'] or observed['bytes'] > MAX_PENDING_BYTES:
        raise ValueError('Bounded original closed archive required')
    return path, sidecar, receipt, member, observed


def drain_original(root, revision, cell, report):
    if not original_owners_joined(report):
        raise ValueError('All original physical owner joins required for archive drain')
    payload = dict(run_directory=str(root), source_revision=revision, cell=cell)
    path, sidecar, receipt, member, observed = validate_remote(payload)
    payload.update(archive_signature=observed, sidecar_sha256=digest(sidecar))
    # The worker stays inside its existing lease until the original coordinator
    # has durably copied, independently verified, and acknowledged custody.
    return request_host('drain-archive', payload)


def remote_action(action, payload):
    path, sidecar, receipt, member, observed = validate_remote(payload)
    if observed != payload['archive_signature'] or digest(sidecar) != payload['sidecar_sha256']:
        raise ValueError('Original archive identity changed')
    if action == 'inspect':
        return dict(signature=observed, receipt=receipt, member=member)
    if action == 'stream':
        with path.open('rb') as stream:
            while block := stream.read(1024 * 1024):
                sys.stdout.buffer.write(block)
        sys.stdout.buffer.flush()
        if signature(path) != observed:
            raise ValueError('Original archive changed during transfer')
        return None
    if action != 'unlink':
        raise ValueError('Unknown original archive custody action')
    ack = payload['durable_ack']
    if (ack['sha256'] != receipt['archive_sha256'] or ack['bytes'] != observed['bytes']
            or ack['mode'] != observed['mode'] or ack['mtime_ns'] != observed['mtime_ns']
            or ack.get('host_file_synchronized') is not True or ack.get('host_directory_synchronized') is not True
            or ack.get('original_stream_joined') is not True or ack.get('all_original_owners_joined') is not True
            or ack.get('host_report_sha256') != payload['cell']['execution']['component']['report_sha256']):
        raise ValueError('Exact durable HOST custody acknowledgement required')
    if digest(path) != receipt['archive_sha256'] or signature(path) != observed:
        raise ValueError('Original archive content changed before retirement')
    proof = sidecar.with_name(sidecar.stem + '.custody-pre.json')
    atomic_json(proof, dict(original=payload, vm_original_unlinked=False))
    if signature(path) != observed:
        raise ValueError('Original archive changed before unlink')
    path.unlink()
    sync_directory(path.parent)
    absent=path_absent(path)
    result = dict(original_archive_absent=absent, original_replacement_preserved=not absent,original_sidecar_preserved=True,
                  durable_ack=ack, pre_unlink_proof_sha256=digest(proof))
    atomic_json(sidecar.with_name(sidecar.stem + '.custody-post.json'), result)
    if not absent:
        raise ValueError('Original archive path was replaced; preserve the replacement inode')
    return result


class ArchiveDrain:
    """Serial original-coordinator calls; every transport is naturally joined."""
    maximum_pending_bytes = MAX_PENDING_BYTES
    def __init__(self, destination, run_directory, revision, *, run=subprocess.run, popen=subprocess.Popen):
        self.destination = Path(destination)
        self.run_directory, self.revision = str(run_directory), revision
        self.run, self.popen = run, popen
        self.completed, self.transports = [], []

    def command(self, action):
        return ['colima', '--profile', 'connect-gate6', 'ssh', '--', 'python3', '-B',
                '/workspace/scripts/lib/matrix_archive_drain.py', '--remote', action]

    def json_command(self, action, payload):
        child = self.popen(self.command(action), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, start_new_session=True)
        transport = dict(action=action, original_pid=child.pid, original_joined=False, forced=False)
        self.transports.append(transport)
        try:
            stdout, stderr = child.communicate(json.dumps(payload).encode(), timeout=120)
        except BaseException:
            transport['forced'] = True
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
            try:
                child.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.communicate(timeout=5)
            raise
        finally:
            transport['exit_code'] = child.returncode
            try:
                os.killpg(child.pid, 0)
            except ProcessLookupError:
                transport['original_joined'] = child.returncode is not None
        if child.returncode != 0 or not transport['original_joined'] or transport['forced']:
            raise RuntimeError('Original closed-archive custody transport failed')
        if len(stdout) > 32 * 1024**2:
            raise ValueError('Bounded original archive receipt required')
        return json.loads(stdout)

    def drain(self, payload):
        if payload['run_directory'] != self.run_directory or payload['source_revision'] != self.revision:
            raise ValueError('Original coordinator run/source differs')
        observed = self.json_command('inspect', payload)
        original = observed['signature']
        if original != payload['archive_signature'] or original['bytes'] > MAX_PENDING_BYTES:
            raise ValueError('Original bounded queue entry differs')
        relative = Path(payload['cell']['id'].removeprefix('G6-058/'))
        destination = self.destination / relative.parent / (relative.name + '.tar.gz')
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        with destination.open('xb') as output:
            child = self.popen(self.command('stream'), stdin=subprocess.PIPE, stdout=output,
                               stderr=subprocess.PIPE, start_new_session=True)
            transport = dict(action='stream', original_pid=child.pid, original_joined=False, forced=False)
            self.transports.append(transport)
            try:
                _, stderr = child.communicate(json.dumps(payload).encode(), timeout=120)
            except BaseException:
                transport['forced'] = True
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.communicate(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.communicate(timeout=5)
                raise
            finally:
                transport['exit_code'] = child.returncode
                try:
                    os.killpg(child.pid, 0)
                except ProcessLookupError:
                    transport['original_joined'] = child.returncode is not None
                atomic_json(destination.with_suffix('.transport.json'), transport)
            if child.returncode != 0 or not transport['original_joined'] or transport['forced']:
                raise RuntimeError('Original archive stream failed; VM original preserved')
            output.flush()
            sync_file(output.fileno())
        os.chmod(destination, original['mode'])
        os.utime(destination, ns=(original['mtime_ns'], original['mtime_ns']))
        with destination.open('rb') as stream:
            sync_file(stream.fileno())
        sync_ancestors(destination.parent,self.destination.parent)
        receipt = observed['receipt']
        if (digest(destination) != receipt['archive_sha256'] or destination.stat().st_size != original['bytes']
                or stat.S_IMODE(destination.stat().st_mode) != original['mode']
                or destination.stat().st_mtime_ns != original['mtime_ns']):
            raise ValueError('Durable HOST archive identity differs; VM original preserved')
        with tarfile.open(destination, 'r:gz') as archive:
            report_bytes = archive.extractfile(observed['member']).read()
        report_sha = hashlib.sha256(report_bytes).hexdigest()
        if (report_sha != payload['cell']['execution']['component']['report_sha256']
                or not original_owners_joined(json.loads(report_bytes))):
            raise ValueError('Original report/physical owner custody unconfirmed; VM original preserved')
        ack = dict(host_path=str(destination), sha256=receipt['archive_sha256'], bytes=original['bytes'],
            mode=original['mode'], mtime_ns=original['mtime_ns'], host_report_sha256=report_sha,
            host_file_synchronized=True, host_directory_synchronized=True,
            original_stream_joined=True, original_stream_exit_code=child.returncode,
            all_original_owners_joined=True)
        atomic_json(destination.with_suffix('.durable.json'), ack)
        result = self.json_command('unlink', dict(payload, durable_ack=ack))
        if result.get('original_archive_absent') is not True:
            raise ValueError('Original archive retirement did not observe absence')
        result.update(host_archive=str(destination), host_ack=ack)
        atomic_json(destination.with_suffix('.custody.json'), result)
        self.completed.append(result)
        return result




MAX_GUEST_BYTES = 4 * 1024**3
MAX_GUEST_FILES = 256
GUEST_PARENT = Path('/workspace/.local')


def guest_candidate(relative, inputs):
    """Retire generated payload copies only; source/config/report metadata stays."""
    import re
    relative = Path(relative)
    name = relative.as_posix()
    if name in inputs or relative.is_absolute() or '..' in relative.parts:
        return False
    if name in {'.local/bin/connect','.local/bin/connect-external-rust-journey'}:
        return True
    if any(part in {'scripts','sdk','contracts','.git','trusted-profile','untrusted-profile'} for part in relative.parts):
        return False
    if re.fullmatch(r'received-[0-5]\.json',relative.name):
        return True
    if 'av-sources' in relative.parts and re.fullmatch(r'source-[0-5]\.(json|ivf|ogg)',relative.name):
        return True
    return relative.suffix in {'.pcap','.f32','.rgb','.ivf','.ogg','.wav','.webm','.yuv'}


def guest_validate(payload):
    root = Path(payload['guest_root'])
    if (root.parent != GUEST_PARENT or len(root.name)!=43 or not root.name.startswith('mac-remote-')
            or any(c not in '0123456789abcdef' for c in root.name[11:]) or root.resolve()!=root):
        raise ValueError('Original closed guest root required')
    from lib.mac_linux_guest import lease_owners_joined
    if not lease_owners_joined(root):
        raise ValueError('Actual original guest process/container custody remains unconfirmed')
    lease = json.loads((root/'lease-state.json').read_text())
    if lease.get('state')!='released' or lease.get('original_owners_joined') is not True:
        raise ValueError('Original guest workload lease not naturally released')
    receipt = payload['guest_receipt']
    if (receipt['run_id']!=root.name or receipt.get('original_joined') is not True
            or receipt.get('all_guest_owners_joined') is not True or receipt.get('original_vm_lease_joined') is not True
            or receipt.get('forced') is not False or receipt.get('cleanup_failures')!=[]
            or receipt.get('client_inputs_unchanged') is not True):
        raise ValueError('Original HOST guest custody receipt differs')
    if not (root/'job-intent.json').is_file():
        raise ValueError('Original guest job intent required')
    job_bytes = (root/'job.json').read_bytes();job=json.loads(job_bytes)
    if job['run_id']!=root.name or job['source_revision']!=payload['source_revision']:
        raise ValueError('Original guest job source/run differs')
    output = root/job['output']
    if not output.resolve().is_relative_to(root) or output.is_symlink():
        raise ValueError('Original guest output escaped owner root')
    actual = json.loads((output/'guest-owner.json').read_text())
    if (actual!=receipt['guest'] or actual.get('forced') is not False or actual.get('original_joined') is not True
            or actual.get('format')!='connect/mac-linux-original-job/1'
            or type(actual.get('original_pid')) is not int or actual['original_pid']<=1
            or type(actual.get('original_start_ticks')) is not int or actual['original_start_ticks']<=0
            or ('capture_original_joined' in actual and actual['capture_original_joined'] is not True)
            or json.loads((root/'job-state.json').read_text())!=actual):
        raise ValueError('Original guest job/owner/capture identity differs')
    for path in (root/'.owners').glob('*-receipt.json'):
        owner=json.loads(path.read_text())
        if owner.get('forced') is not False or owner.get('original_joined') is not True:
            raise ValueError('Original guest helper was not naturally joined')
    inputs=payload['client_inputs']
    if not isinstance(inputs,dict) or len(inputs)>10000 or inputs.get('job.json')!=hashlib.sha256(job_bytes).hexdigest():
        raise ValueError('Original staged guest input manifest differs')
    for name,sha in inputs.items():
        relative=Path(name);path=root/relative
        if relative.is_absolute() or '..' in relative.parts or path.resolve()!=path or digest(path)!=sha:
            raise ValueError('Original guest input changed')
    return root, inputs


def guest_absence(root,files):
    present=[]
    for name in files:
        try:
            (root/name).lstat()
        except FileNotFoundError:
            continue
        present.append(name)
    return dict(all_selected_absent=not present,present_paths=present)


def guest_action(action,payload):
    root,inputs=guest_validate(payload)
    if action=='inspect':
        files={}
        for path in sorted(root.rglob('*')):
            relative=path.relative_to(root)
            if path.is_file() and guest_candidate(relative,inputs):
                files[relative.as_posix()]=dict(signature=signature(path),sha256=digest(path))
                expected=payload.get('original_binary_hashes',{}).get(relative.as_posix())
                if relative.suffix=='.pcap' and payload.get('original_capture_sha256') is not None:
                    expected=payload['original_capture_sha256']
                if expected is not None and files[relative.as_posix()]['sha256']!=expected:
                    raise ValueError('Original prepared guest binary hash differs')
        if set(payload.get('original_binary_hashes',{}))-set(files):
            raise ValueError('Original guest binary copy is missing')
        if len(files)>MAX_GUEST_FILES or sum(v['signature']['bytes'] for v in files.values())>MAX_GUEST_BYTES:
            raise ValueError('Bounded closed guest payload queue exceeded')
        return dict(files=files,all_original_owners_joined=True,original_lease_released=True)
    files=payload['files']
    if len(files)>MAX_GUEST_FILES or sum(v['signature']['bytes'] for v in files.values())>MAX_GUEST_BYTES:
        raise ValueError('Bounded original guest file list required')
    for name,row in files.items():
        path=root/name
        if not guest_candidate(name,inputs) or signature(path)!=row['signature']:
            raise ValueError('Original selected guest payload identity differs')
    if action=='stream':
        with tarfile.open(fileobj=sys.stdout.buffer,mode='w|') as archive:
            for name,row in files.items():
                path=root/name;item=tarfile.TarInfo(name)
                item.mode,item.size=row['signature']['mode'],row['signature']['bytes']
                with path.open('rb') as source:archive.addfile(item,source)
                if signature(path)!=row['signature']:
                    raise ValueError('Original guest payload changed during transfer')
        return None
    if action!='unlink':
        raise ValueError('Unknown original guest custody action')
    acknowledgement=payload['durable_ack']
    if (acknowledgement.get('all_original_files_verified') is not True
            or acknowledgement.get('original_stream_joined') is not True
            or acknowledgement.get('host_file_synchronized') is not True
            or acknowledgement.get('host_directory_synchronized') is not True
            or acknowledgement.get('files')!=files):
        raise ValueError('Exact durable closed guest payload acknowledgement required')
    for name,row in files.items():
        if digest(root/name)!=row['sha256'] or signature(root/name)!=row['signature']:
            raise ValueError('Original guest payload changed before retirement')
    from lib.matrix_storage import guest_reserve
    reserve=guest_reserve(root)
    atomic_json(root/'closed-payload-custody-pre.json',dict(files=files,durable_ack=acknowledgement),reserve=reserve)
    removed=[];synchronized=[]
    try:
        for name,row in files.items():
            if signature(root/name)!=row['signature']:
                raise ValueError('Original guest payload changed at retirement')
            (root/name).unlink();removed.append(name)
            sync_directory((root/name).parent);synchronized.append(name)
    finally:
        observed=guest_absence(root,files)
        atomic_json(root/'closed-payload-custody-post.json',dict(removed=removed,**observed,
            unlink_directories_synchronized=len(synchronized)==len(files)),reserve=reserve)
    guest_validate(payload)
    observed=guest_absence(root,files)
    if not observed['all_selected_absent']:
        atomic_json(root/'closed-payload-custody-recheck.json',observed,reserve=reserve)
        raise ValueError('A selected guest path was replaced; preserve the new inode')
    return dict(removed=removed,**observed,unlink_directories_synchronized=True,
                original_metadata_and_inputs_preserved=True)


class GuestDrain(ArchiveDrain):
    maximum_pending_bytes = MAX_GUEST_BYTES
    def __init__(self,matrix_output,run_directory,revision,**kwargs):
        super().__init__(Path(matrix_output)/'retained-mac-guest-payloads',run_directory,revision,**kwargs)
        self.matrix_output=Path(matrix_output)

    def command(self,action):
        command=super().command(action);command[-2]='--guest';return command

    def bind_host(self,payload):
        cell=payload['cell']
        if (payload['source_revision']!=self.revision or cell['server']!='macos-arm64'
                or cell.get('verifier',{}).get('integrity') is not True
                or cell['network']!='remote-profile-fixture' or cell['status'] not in {'passed','failed','blocked'}
                or cell['transport'] not in {'native-quic','chrome-webtransport','chrome-wss-webrtc','firefox-wss-webrtc'}
                or cell['signing'] not in {'EdDSA','ES256'}
                or cell['id']!='/'.join(['G6-058',cell['server'],cell['network'],cell['transport'],cell['signing']])):
            raise ValueError('Original terminal Mac remote cell required')
        directory=self.matrix_output/'macos-arm64/cells'/cell['server']/cell['network']/cell['transport']/cell['signing']
        if str(directory)!=payload['component_directory'] or directory.resolve()!=directory:
            raise ValueError('Original HOST component directory differs')
        matrix=json.loads((self.matrix_output/'macos-arm64/matrix.json').read_text())
        if matrix['source_revision']!=self.revision or next(c for c in matrix['cells'] if c['id']==cell['id'])!=cell:
            raise ValueError('Original terminal HOST cell changed')
        path=directory/('evidence/media/rehearsal.json' if cell['transport']=='native-quic' else 'component.json')
        if digest(path)!=cell['execution']['component']['report_sha256']:
            raise ValueError('Original HOST report hash differs')
        report=json.loads(path.read_text())
        if report.get('source_revision')!=self.revision or not original_owners_joined(report):
            raise ValueError('All original component owners must join before guest payload retirement')
        receipt=report['remote_client_owner']
        from .mac_linux_fixture import remote_browser_owner_joined
        if not remote_browser_owner_joined(receipt):
            raise ValueError('Original remote guest custody is incomplete')
        owner=directory/('private/media/remote-owner' if cell['transport']=='native-quic' else 'private/remote-owner')
        inputs=json.loads((owner/'client-inputs.json').read_text())
        binaries={}
        if cell['transport']=='native-quic':
            binaries={'.local/bin/connect':report['client_binary_sha256'],
                      '.local/bin/connect-external-rust-journey':report['peer_binary_sha256']}
        return dict(guest_root=str(GUEST_PARENT/receipt['run_id']),source_revision=self.revision,
                    guest_receipt=receipt,client_inputs=inputs,original_binary_hashes=binaries,
                    original_capture_sha256=report.get('packet_capture',{}).get('pcap_sha256'))

    def drain(self,payload):
        remote=self.bind_host(payload);observed=self.json_command('inspect',remote);files=observed['files']
        total=sum(v['signature']['bytes'] for v in files.values())
        for name,expected_sha in remote.get('original_binary_hashes',{}).items():
            if name not in files or files[name]['sha256']!=expected_sha:
                raise ValueError('Original guest binary copy is missing or changed')
        for name in files:
            relative=Path(name)
            if relative.suffix=='.pcap' and remote.get('original_capture_sha256') is not None and files[name]['sha256']!=remote['original_capture_sha256']:
                raise ValueError('Original capture payload hash differs')
            if relative.is_absolute() or '..' in relative.parts or '\\' in name or not guest_candidate(name,remote['client_inputs']):
                raise ValueError('Ordinary original guest payload name required')
        from .matrix_storage import volume,SPOOL_RESERVE_BYTES
        if len(files)>MAX_GUEST_FILES or total>MAX_GUEST_BYTES or volume(self.matrix_output)['free_bytes']<2*total+2*1024**2+SPOOL_RESERVE_BYTES:
            raise ValueError('Closed guest payload spool headroom unavailable')
        destination=self.destination/Path(remote['guest_root']).name
        destination.mkdir(mode=0o700,parents=True,exist_ok=False)
        transfer=destination/'original-stream.tar'
        with (destination/'original-stream.stderr').open('xb') as stderr,transfer.open('xb') as output:
            child=self.popen(self.command('stream'),stdin=subprocess.PIPE,stdout=output,stderr=stderr,start_new_session=True)
            transport=dict(action='guest-stream',original_pid=child.pid,original_joined=False,forced=False)
            self.transports.append(transport)
            try:
                child.communicate(json.dumps(dict(remote,files=files)).encode(),timeout=120)
            finally:
                if child.poll() is None:
                    transport['forced']=True;os.killpg(child.pid,signal.SIGTERM)
                    try:child.wait(timeout=5)
                    except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=5)
                transport['exit_code']=child.returncode
                try:os.killpg(child.pid,0)
                except ProcessLookupError:transport['original_joined']=child.returncode is not None
                atomic_json(destination/'original-stream.json',transport)
            output.flush();sync_file(output.fileno())
        sync_directory(destination)
        if child.returncode!=0 or transport['forced'] or not transport['original_joined']:
            raise ValueError('Original guest payload stream did not naturally join')
        seen=set()
        with tarfile.open(transfer,mode='r:') as archive:
            for member in archive:
                if member.name not in files or member.name in seen or not member.isfile():
                    raise ValueError('Original guest payload tar member differs')
                row=files[member.name];expected=row['signature']
                if member.size!=expected['bytes'] or member.mode!=expected['mode']:
                    raise ValueError('Original guest payload size/mode differs')
                target=destination/member.name;target.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
                with archive.extractfile(member) as source,target.open('xb') as output:
                    while block:=source.read(1024*1024):output.write(block)
                    output.flush();sync_file(output.fileno())
                os.chmod(target,expected['mode']);os.utime(target,ns=(expected['mtime_ns'],expected['mtime_ns']))
                with target.open('rb') as output:sync_file(output.fileno())
                sync_directory(target.parent)
                if (digest(target)!=row['sha256'] or target.stat().st_size!=expected['bytes']
                        or stat.S_IMODE(target.stat().st_mode)!=expected['mode'] or target.stat().st_mtime_ns!=expected['mtime_ns']):
                    raise ValueError('Durable HOST guest payload differs')
                seen.add(member.name)
        if seen!=set(files):
            raise ValueError('Original guest payload transfer incomplete')
        for folder in sorted((p for p in destination.rglob('*') if p.is_dir()),key=lambda p:len(p.parts),reverse=True):
            sync_directory(folder)
        sync_ancestors(destination,self.destination.parent)
        ack=dict(files=files,host_destination=str(destination),all_original_files_verified=True,
            original_stream_joined=True,host_file_synchronized=True,host_directory_synchronized=True,original_transfer_sha256=digest(transfer))
        atomic_json(destination/'durable-ack.json',ack)
        result=self.json_command('unlink',dict(remote,files=files,durable_ack=ack))
        if result.get('all_selected_absent') is not True:
            raise ValueError('Original guest retirement did not observe absence')
        result.update(host_destination=str(destination),files=files,bytes=total)
        atomic_json(destination/'custody-before-spool-retirement.json',result)
        transfer.unlink();sync_directory(destination)
        result['redundant_transfer_spool_retired_after_durable_files']=True
        atomic_json(destination/'custody.json',result);self.completed.append(result)
        return result


if __name__ == '__main__':
    if len(sys.argv)!=3 or sys.argv[1] not in {'--remote','--guest'}:
        raise SystemExit('Original remote custody action required')
    result=(remote_action if sys.argv[1]=='--remote' else guest_action)(sys.argv[2],json.load(sys.stdin))
    if result is not None:print(json.dumps(result))
