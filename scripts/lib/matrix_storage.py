"""Harness storage admission and bounded durable metadata, never media verdicts."""
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile
import time
import uuid

# At most32 main+32 archive+16 borrowed admissions, each with61 reads,
# plus bounded effects/fallbacks; round the finite envelope above5032.
MAX_STORAGE_REQUESTS = 8192
CONTROL_PARENT = Path('/workspace/.local')
MIB = 1024**2
GIB = 1024**3
# One bounded cleanup message (16KiB), two atomic copies, filesystem rounding.
GUARD_RESERVE_BYTES = 64 * 1024
# Full 32-cell metadata plus one atomic replacement and bounded failure receipts.
RUN_METADATA_LIMIT = 2 * MIB
RUN_RESERVE_BYTES = 2 * RUN_METADATA_LIMIT + GUARD_RESERVE_BYTES
RAW_SOURCE_BYTES = 6 * 62 * 30 * 640 * 360 * 3 // 2
HOST_RESERVE_BYTES = 8 * GIB
SPOOL_RESERVE_BYTES = 8 * GIB


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def sync_ancestors(path, stop):
    path, stop = Path(path), Path(stop)
    if not path.is_relative_to(stop):
        raise ValueError('Task-owned directory synchronization boundary required')
    while True:
        sync_directory(path)
        if path == stop:
            break
        path = path.parent


def sync_file(descriptor):
    os.fsync(descriptor)
    if hasattr(fcntl, 'F_FULLFSYNC'):
        fcntl.fcntl(descriptor, fcntl.F_FULLFSYNC)


class MetadataReserve:
    """Only this synthetic inode may fund a failed bounded metadata write."""
    def __init__(self, path, size=RUN_RESERVE_BYTES):
        self.path = Path(path)
        descriptor = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            remaining = size
            block = bytes(min(size, MIB))
            while remaining:
                written = os.write(descriptor, block[:min(remaining, len(block))])
                remaining -= written
            sync_file(descriptor)
            observed = os.fstat(descriptor)
            self.identity = (observed.st_dev, observed.st_ino, observed.st_size)
        finally:
            os.close(descriptor)
        sync_directory(self.path.parent)
        self.released = False

    @classmethod
    def attach(cls, path, identity):
        owner = cls.__new__(cls)
        owner.path, owner.identity = Path(path), tuple(identity)
        owner.released = not owner.path.exists() and not owner.path.is_symlink()
        if not owner.released:
            observed = owner.path.lstat()
            if (not stat.S_ISREG(observed.st_mode) or observed.st_nlink != 1
                    or stat.S_IMODE(observed.st_mode) != 0o600 or observed.st_uid != os.getuid()
                    or (observed.st_dev,observed.st_ino,observed.st_size) != owner.identity):
                raise ValueError('Original synthetic reserve identity differs')
        return owner

    def release(self):
        if self.released:
            return
        observed = self.path.lstat()
        if (not stat.S_ISREG(observed.st_mode) or observed.st_nlink != 1
                or (observed.st_dev, observed.st_ino, observed.st_size) != self.identity):
            raise ValueError('Original synthetic metadata reserve inode differs')
        self.path.unlink()
        sync_directory(self.path.parent)
        self.released = True


def atomic_bytes(path, body, *, reserve=None, limit=RUN_METADATA_LIMIT):
    """Replacement never truncates the last valid record, even on ENOSPC."""
    path = Path(path)
    if path.is_symlink():
        raise ValueError('Original metadata path cannot be a symlink')
    if len(body) > limit:
        raise ValueError('Bounded harness metadata required')
    def attempt():
        descriptor, name = tempfile.mkstemp(prefix='.' + path.name + '.', dir=path.parent)
        try:
            with os.fdopen(descriptor, 'wb') as stream:
                stream.write(body)
                stream.flush()
                sync_file(stream.fileno())
            os.replace(name, path)
            sync_directory(path.parent)
        finally:
            try:
                os.unlink(name)
            except FileNotFoundError:
                pass
    try:
        attempt()
    except OSError as error:
        if error.errno != errno.ENOSPC or reserve is None or reserve.released:
            raise
        reserve.release()
        attempt()


def atomic_json(path, value, *, reserve=None, limit=RUN_METADATA_LIMIT):
    atomic_bytes(path, (json.dumps(value, indent=2) + '\n').encode(), reserve=reserve, limit=limit)


def request_digest(request):
    return hashlib.sha256(json.dumps(request,sort_keys=True,separators=(',',':')).encode()).hexdigest()


def write_response(path,result,*,reserve):
    path=Path(path)
    if path.exists():
        if json.loads(path.read_text())!=result:
            raise ValueError('Original storage answer already bound; never overwrite it')
        return
    atomic_json(path,result,reserve=reserve)


def source_budget():
    # Operational headroom, not a claimed hard cap on encoder output or logs.
    # Existing source decoders independently cap each Opus/IVF container at32MiB.
    return dict(raw_programmes=RAW_SOURCE_BYTES,
                encoded_containers=6 * 2 * 32 * MIB,
                source_json=6 * (2 * 32 * MIB * 4 // 3 + MIB),
                live_encoder_build=256 * MIB, metadata=RUN_RESERVE_BYTES)


def cell_budget():
    values = source_budget()
    # Three real cycles, existing512MiB/cycle guest transfer bound, plus decoded
    # PCM/reference working space. These are disclosed operational reserves;
    # archive admission below uses actual logical member sizes instead.
    values.update(received_three_cycles=3 * 512 * MIB,
                  decoded_and_reference=512 * MIB,
                  sealed_executables=512 * MIB, build_growth=2 * GIB,
                  capture_and_logs=512 * MIB)
    values['archive_overlap'] = sum(v for k, v in values.items() if k != 'build_growth')
    return values


def guest_budget(*, borrowed_lease=False):
    if not borrowed_lease:
        return cell_budget()
    # Existing remote authority child stages at most512MiB and collects at
    # most512MiB. It borrows its parent's already-prepared native executable;
    # it does not generate another six-source/three-cycle media programme.
    return dict(private_source_transfer=512*MIB,private_output_transfer=512*MIB,
                metadata=RUN_RESERVE_BYTES)


def volume(path):
    path = Path(path).resolve(strict=True)
    observed = path.stat()
    usage = shutil.disk_usage(path)
    filesystem = os.statvfs(path)
    return dict(path=str(path), device=observed.st_dev, total_bytes=usage.total,
                free_bytes=usage.free, available_inodes=filesystem.f_favail)


def admit(observed, required_bytes, *, required_by_role=None):
    """All three observations are mandatory; logical guest capacity is no proxy."""
    reasons = []
    for role, reserve in [('guest', 0), ('backing', HOST_RESERVE_BYTES), ('spool', SPOOL_RESERVE_BYTES)]:
        value = observed.get(role)
        if (not isinstance(value, dict) or type(value.get('free_bytes')) is not int
                or type(value.get('available_inodes')) is not int):
            reasons.append(role + '-observation-unavailable')
        elif value['free_bytes'] < (required_by_role or {}).get(role, required_bytes) + reserve or value['available_inodes'] < 4096:
            reasons.append(role + '-storage-headroom-unavailable')
    return dict(admitted=not reasons, reasons=reasons, required_bytes=required_bytes,
                required_by_role=required_by_role or {role:required_bytes for role in ('guest','backing','spool')},
                operational_reserves=dict(backing=HOST_RESERVE_BYTES, spool=SPOOL_RESERVE_BYTES),
                observations=observed)


def archive_budget(entries):
    # Tar writes each hardlinked name independently. Allow PAX headers per member,
    # tar alignment/end records, and gzip's worst-case deflate expansion.
    tar_bytes = 10240
    for name, row in entries.items():
        tar_bytes += 4096 + ((len(name.encode()) + 511) // 512) * 512
        if row['kind'] == 'file':
            tar_bytes += ((row['bytes'] + 511) // 512) * 512
    return tar_bytes + ((tar_bytes + 16382) // 16383) * 5 + 64 + RUN_RESERVE_BYTES


def request_host(kind, payload):
    """Fresh original-coordinator response through private run-bound metadata."""
    directory = Path(os.environ['CONNECT_MATRIX_STORAGE_CONTROL'])
    config = json.loads((directory / 'config.json').read_text())
    if config['token'] != os.environ['CONNECT_MATRIX_STORAGE_TOKEN']:
        raise ValueError('Original storage coordinator identity differs')
    request_id = uuid.uuid4().hex
    request = dict(token=config['token'], id=request_id, kind=kind, payload=payload,
                   source_revision=config['source_revision'],run_directory=config['run_directory'])
    # Own an immutable JSON copy of this original request while awaiting its answer.
    request=json.loads(json.dumps(request))
    expected_request_sha=request_digest(request)
    identity = config.get('reserve_identity')
    reserve = MetadataReserve.attach(directory/config['reserve_name'], identity) if identity else None
    original_request=directory/(request_id+'.request.json')
    if original_request.exists():
        if request_digest(json.loads(original_request.read_text()))!=expected_request_sha:
            raise ValueError('Original storage request ID cannot change intent')
    else:
        atomic_json(original_request,request,reserve=reserve)
    cutoff = time.monotonic() + 600
    response = directory / (request_id + '.response.json')
    while time.monotonic() < cutoff:
        if response.exists():
            result = json.loads(response.read_text())
            if (result.get('id') != request_id or result.get('token') != config['token']
                    or result.get('request_sha256')!=expected_request_sha or result.get('kind')!=kind
                    or result.get('source_revision')!=config['source_revision']
                    or result.get('run_directory')!=config['run_directory']):
                raise ValueError('Original storage response identity differs')
            if not result.get('ok'):
                raise RuntimeError('Original storage coordinator rejected request')
            return result['value']
        time.sleep(.1)
    raise TimeoutError('Original storage coordinator response unavailable')


def await_admission(read_guest, required, *, required_by_role=None):
    # Online discard and a concurrently closed peer's durable custody can
    # settle after its lease release. Wait only before entering media, keeping
    # the same floor and taking a fresh observation on every read-only retry.
    cutoff=time.monotonic()+60
    initial_reasons=None;observations=0
    while True:
        host=request_host('observe',dict(required_bytes=required))
        result=admit(dict(guest=read_guest(),backing=host['backing'],spool=host['spool']),
                     required,required_by_role=required_by_role)
        observations+=1
        if initial_reasons is None:initial_reasons=result['reasons']
        result.update(storage_observation_count=observations,initial_block_reasons=initial_reasons,
                      bounded_storage_settle_seconds=60)
        if result['admitted']:
            return result
        remaining=cutoff-time.monotonic()
        if remaining<=0:
            raise StorageAdmissionError(result)
        time.sleep(min(1,remaining))


def preflight(directory, *, required=None):
    directory=Path(directory)
    required=sum(cell_budget().values()) if required is None else required
    return await_admission(lambda:volume(directory),required)


class StorageAdmissionError(RuntimeError):
    def __init__(self, observation):
        super().__init__('Original storage admission unavailable before component entry')
        self.observation = observation


def metadata_failure(value):
    """HOST durability is explicit; it does not imply a VM receipt was saved."""
    return request_host('metadata-fallback', dict(format='connect/storage-metadata-fallback/1',
        vm_receipt_saved=False, original=value))


class CoordinatorStorage:
    """One original coordinator services fresh admission and a serial drain."""
    def __init__(self, root, output, run_directory, revision):
        import subprocess
        self.root, self.output = Path(root), Path(output)
        self.source_revision,self.run_directory=revision,str(run_directory)
        sync_directory(self.output.parent)
        base = Path.home() / '.codex/tmp'
        base.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.directory = Path(tempfile.mkdtemp(prefix='connect-matrix-storage-', dir=base))
        self.token = uuid.uuid4().hex
        self.backing = Path.home() / '.colima/_lima/colima-connect-gate6/disk'
        if not self.backing.is_file() or self.backing.is_symlink():
            raise ValueError('Exact owned VM backing disk unavailable')
        self.backing_identity = (self.backing.stat().st_dev, self.backing.stat().st_ino)
        self.reserve = MetadataReserve(self.directory / 'metadata.reserve')
        self.handled, self.failures = [], []
        atomic_json(self.directory / 'config.json', dict(token=self.token, source_revision=revision,
                    run_directory=str(run_directory),reserve_identity=self.reserve.identity,reserve_name='metadata.reserve'))
        # Control metadata uses the existing original SSH transport. The owned
        # VM intentionally has no HOST filesystem mount; never guess one.
        self.remote_directory = Path(str(run_directory) + '-storage-control')
        self.remote_base = dict(directory=str(self.remote_directory), token=self.token,
                                run_directory=str(run_directory), source_revision=revision)
        self.remote_command('bootstrap', self.remote_base)
        from .matrix_archive_drain import ArchiveDrain, GuestDrain
        self.drain = ArchiveDrain(output / 'retained-arm-archives', run_directory, revision)
        self.guest_drain = GuestDrain(output, run_directory, revision)

    @property
    def environment(self):
        return dict(CONNECT_MATRIX_STORAGE_CONTROL=str(self.directory), CONNECT_MATRIX_STORAGE_TOKEN=self.token,
                    CONNECT_MATRIX_STORAGE_REMOTE_CONTROL=str(self.remote_directory))

    @property
    def remote_environment(self):
        return dict(CONNECT_MATRIX_STORAGE_CONTROL=str(self.remote_directory),
                    CONNECT_MATRIX_STORAGE_TOKEN=self.token)

    def remote_command(self, action, payload):
        import subprocess
        result = subprocess.run(['colima','--profile','connect-gate6','ssh','--','python3','-B',
            '/workspace/scripts/lib/matrix_storage.py','--control',action],
            input=json.dumps(payload).encode(),capture_output=True,timeout=30,check=True)
        return json.loads(result.stdout)

    def remote_snapshot(self):
        value = self.remote_command('snapshot', self.remote_base)
        for request in value['requests']:
            request_id = request.get('id')
            # Snapshot includes only missing/mismatched replies. Even a handled
            # effect must recover its original answer when that reply was lost.
            result = self.answer(request_id, request)
            self.remote_command('respond', dict(self.remote_base,response=result))
            if request_id not in self.handled:self.handled.append(request_id)
            atomic_json(self.output / 'storage-coordinator.json', self.receipt(), reserve=self.reserve)
        if value['matrix'] is None:
            raise ValueError('Original VM matrix not yet available')
        return value['matrix']

    def observe(self):
        if (self.backing.stat().st_dev, self.backing.stat().st_ino) != self.backing_identity:
            raise ValueError('Actual original VM backing inode changed')
        return dict(backing=volume(self.backing), spool=volume(self.output))

    def answer(self, request_id, request):
        phase = request.get('kind') if isinstance(request,dict) else None
        valid = (isinstance(request,dict) and isinstance(request_id,str) and len(request_id)==32
                 and all(c in '0123456789abcdef' for c in request_id)
                 and request.get('id')==request_id and request.get('token')==self.token
                 and request.get('source_revision')==self.source_revision and request.get('run_directory')==self.run_directory)
        request_sha = request_digest(request)
        response_binding=dict(request_sha256=request_sha,kind=phase,source_revision=self.source_revision,run_directory=self.run_directory)
        ledger = self.output / ('storage-answer-' + (request_id if valid else 'invalid-'+request_sha) + '.json')
        if valid and phase!='observe' and ledger.exists():
            original=json.loads(ledger.read_text())
            if original['request_sha256']!=request_sha:
                failure=dict(request_id=request_id,failure='ChangedOriginalStorageRequest',phase=phase)
                if failure not in self.failures:self.failures.append(failure)
                return dict(id=request_id,token=self.token,ok=False,failure='ChangedOriginalStorageRequest',**response_binding)
            # A response transport can fail after durable copy/unlink. Recover
            # that original receipt; never dispatch the storage effect twice.
            return original['response']
        try:
            if not valid:
                raise ValueError('Original storage request identity differs')
            if request['kind'] == 'observe':
                value = self.observe()
            elif request['kind'] == 'metadata-fallback':
                value = dict(host_durable=True, vm_receipt_saved=False,
                             receipt_path=str(self.output / ('storage-fallback-' + request_id + '.json')))
                atomic_json(Path(value['receipt_path']), request['payload'], reserve=self.reserve)
            elif request['kind'] == 'drain-guest':
                value = self.guest_drain.drain(request['payload'])
            elif request['kind'] == 'drain-archive':
                required = request['payload']['archive_signature']['bytes']
                if volume(self.output)['free_bytes'] < required + SPOOL_RESERVE_BYTES:
                    raise ValueError('Durable artifact spool headroom unavailable')
                value = self.drain.drain(request['payload'])
            else:
                raise ValueError('Unknown bounded storage request')
            result = dict(id=request_id, token=self.token, ok=True, value=value,**response_binding)
        except Exception as error:
            import traceback
            failure = dict(request_id=request_id, failure=type(error).__name__,
                           phase=phase, private_traceback=traceback.format_exc()[-65536:])
            atomic_json(self.output / ('storage-failure-' + str(request_id) + '.json'), failure, reserve=self.reserve)
            self.failures.append(dict(request_id=request_id, failure=type(error).__name__, phase=phase))
            result = dict(id=request_id, token=self.token, ok=False, failure=type(error).__name__,**response_binding)
        if phase!='observe':
            atomic_json(ledger,dict(request_sha256=request_sha,response=result),reserve=self.reserve)
        return result

    def service(self):
        requests = sorted(self.directory.glob('*.request.json'))
        if len(requests) > MAX_STORAGE_REQUESTS:
            raise ValueError('Bounded original coordinator request queue exceeded')
        for path in requests:
            request_id = path.name.removesuffix('.request.json')
            if path.stat().st_size > RUN_METADATA_LIMIT or path.is_symlink():
                raise ValueError('Bounded ordinary storage request required')
            request=json.loads(path.read_text())
            response=self.directory/(request_id+'.response.json')
            if (request_id in self.handled and response.exists()
                    and json.loads(response.read_text()).get('request_sha256')==request_digest(request)):
                continue
            result = self.answer(request_id,request)
            write_response(self.directory / (request_id + '.response.json'), result, reserve=self.reserve)
            if request_id not in self.handled:self.handled.append(request_id)
            atomic_json(self.output / 'storage-coordinator.json', self.receipt(), reserve=self.reserve)

    def receipt(self):
        return dict(format='connect/original-coordinator-storage/1',
                    bounded_pending_archives=1, bounded_pending_guest_payload_sets=1,
                    maximum_pending_bytes=self.drain.maximum_pending_bytes+self.guest_drain.maximum_pending_bytes, serial_supervisor_owned=True,
                    completed_archive_custodies=self.drain.completed,
                    completed_guest_payload_custodies=self.guest_drain.completed,
                    handled_requests=len(self.handled), failures=self.failures,
                    original_copy_transports=self.drain.transports + self.guest_drain.transports,
                    all_original_copy_transports_joined=all(t['original_joined'] for t in
                        self.drain.transports + self.guest_drain.transports))


def initialize_guest_reserve(root):
    root = Path(root)
    manifest = root / '.metadata-reserve.json'
    owner = MetadataReserve(root / '.metadata.reserve')
    atomic_json(manifest, dict(identity=owner.identity))
    return owner


def guest_reserve(root):
    root = Path(root)
    manifest = json.loads((root / '.metadata-reserve.json').read_text())
    return MetadataReserve.attach(root / '.metadata.reserve', manifest['identity'])


def control_action(action, payload):
    directory = Path(payload['directory'])
    run = Path(payload['run_directory'])
    if (run.parent != CONTROL_PARENT or not run.name.startswith('g6-058-')
            or directory != Path(str(run) + '-storage-control') or directory.resolve() != directory):
        raise ValueError('Original VM coordinator control directory required')
    if action == 'bootstrap':
        directory.mkdir(mode=0o700, exist_ok=False)
        reserve = initialize_guest_reserve(directory)
        atomic_json(directory/'config.json', dict(payload,reserve_identity=reserve.identity,reserve_name='.metadata.reserve'), reserve=reserve)
        return dict(created=True)
    config = json.loads((directory/'config.json').read_text())
    if any(config[key] != payload[key] for key in ('directory','token','run_directory','source_revision')):
        raise ValueError('Original coordinator control identity differs')
    if action == 'snapshot':
        requests = []
        paths = sorted(directory.glob('*.request.json'))
        if len(paths)>MAX_STORAGE_REQUESTS:
            raise ValueError('Bounded original VM request queue exceeded')
        for path in paths:
            if path.is_symlink() or path.stat().st_size>RUN_METADATA_LIMIT:
                raise ValueError('Bounded ordinary VM storage request required')
            request = json.loads(path.read_text())
            if path.name != request['id'] + '.request.json':
                raise ValueError('Original VM storage request filename differs')
            response=directory/(request['id']+'.response.json')
            if not response.exists() or json.loads(response.read_text()).get('request_sha256')!=request_digest(request):
                requests.append(request)
        matrix = run/'matrix.json'
        return dict(matrix=json.loads(matrix.read_text()) if matrix.exists() else None, requests=requests)
    if action == 'respond':
        result = payload['response'];identity=result['id']
        if len(identity)!=32 or any(c not in '0123456789abcdef' for c in identity) or result['token']!=config['token']:
            raise ValueError('Original VM response identity differs')
        request=json.loads((directory/(identity+'.request.json')).read_text())
        if (result.get('request_sha256')!=request_digest(request) or result.get('kind')!=request['kind']
                or result.get('source_revision')!=config['source_revision']
                or result.get('run_directory')!=config['run_directory']):
            raise ValueError('Original storage response request/source binding differs')
        write_response(directory/(identity+'.response.json'),result,reserve=guest_reserve(directory))
        return dict(response_saved=True)
    raise ValueError('Unknown original coordinator control action')


if __name__ == '__main__':
    import sys
    if len(sys.argv)!=3 or sys.argv[1]!='--control':
        raise SystemExit('Original storage control action required')
    print(json.dumps(control_action(sys.argv[2],json.load(sys.stdin))))


class ComponentStorageFailure(RuntimeError):
    def __init__(self, fallback):
        super().__init__('Original component verification unavailable after report storage failure')
        self.fallback = fallback


def write_component(path, report, *, reserve):
    """Preserve observed custody, never reconstruct a missing full report."""
    path = Path(path)
    body = (json.dumps(report, indent=2) + '\n').encode()
    try:
        atomic_bytes(path, body, reserve=reserve, limit=len(body))
        return
    except OSError as error:
        from .vm_workload_lease import cleanup_evidence
        cell_id = report.get('cell_id')
        if cell_id is None:
            cell_id = '/'.join(['G6-058',report.get('host',{}).get('platform','unknown'),
                'local' if report.get('network_profile')=='local' else 'remote-profile-fixture',
                'native-quic',report.get('signing_algorithm','unknown')])
        failure = report.get('failure') or {}
        source_fields = ('source_inputs_unchanged','sdk_dist_inputs_unchanged','executable_unchanged','executables_unchanged')
        fallback = dict(format='connect/original-component-storage-failure/1',
            source_revision=report.get('source_revision'),cell_id=cell_id,producer_pid=os.getpid(),
            original_report_path=str(path),full_report_saved=False,
            original_full_report_sha256=hashlib.sha256(body).hexdigest(),
            full_media_verification_available=False,cell_qualified=False,task_complete=False,
            original_passed=report.get('passed'),
            known_failure={k:str(failure[k])[:256] for k in ('phase','name','code') if k in failure},
            original_source_flags={k:report[k] for k in source_fields if k in report},
            original_server_identities=[{k:v[k] for k in ('name','original_pid','original_start_ticks','original_exit_code',
                'original_joined','original_process_group_joined') if k in v} for v in report.get('servers',[])[:4]],
            cleanup_evidence=cleanup_evidence(report),storage_error=type(error).__name__,storage_errno=error.errno,
            vm_fallback_saved=False,host_fallback_saved=False)
        fallback_path = path.with_name(path.name + '.storage-failure.json')
        try:
            # This small record alone is funded by the original synthetic inode.
            saved = dict(fallback,vm_fallback_saved=True)
            atomic_json(fallback_path,saved,reserve=reserve,limit=32768)
            fallback = saved
        except Exception as local_error:
            fallback['fallback_storage_error'] = type(local_error).__name__
            try:
                acknowledgement = metadata_failure(fallback)
                fallback['host_fallback_saved'] = acknowledgement.get('host_durable') is True
            except Exception as host_error:
                fallback['host_fallback_error'] = type(host_error).__name__
        raise ComponentStorageFailure(fallback) from error
