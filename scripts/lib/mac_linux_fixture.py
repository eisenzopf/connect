"""Mac server / existing native ARM Linux guest fixture ownership.

All transport traffic uses the actual private Mac interface. The guest receives
an exact private source/SDK bundle; only bounded fixture request/response files
cross the host bridge. Server credentials never enter process arguments.
"""
import hashlib
import io
import ipaddress
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import selectors
import signal
import subprocess
import tarfile
import time
import tomllib
import uuid

from .matrix_macos_resources import MacProcessResources
from .matrix_idle import MatrixIdle
from .matrix_storage import (atomic_json, source_budget, StorageAdmissionError,
                             guest_budget, await_admission)
from .transport_matrix_coordinator import VM_ENV, IMAGE
from .transport_fixture import toml_bytes


GUEST = "/workspace/scripts/lib/mac_linux_guest.py"
LIMIT = 512 * 1024 * 1024


def rewrite_paths(value, source, destination):
    """Rewrite bounded local artifact paths, never IDs, URLs or credential text."""
    if isinstance(value, dict):
        return {k: rewrite_paths(v, source, destination) for k, v in value.items()}
    if isinstance(value, list):
        return [rewrite_paths(v, source, destination) for v in value]
    if isinstance(value, str) and (value == str(source) or value.startswith(str(source) + "/")):
        return str(destination) + value[len(str(source)):]
    return value


def archive_files(files):
    buffer = io.BytesIO()
    total = 0
    if len(files) > 10000:
        raise ValueError("Private source file count")
    with tarfile.open(fileobj=buffer, mode="w:") as archive:
        for name, (body, executable) in sorted(files.items()):
            relative = PurePosixPath(name)
            if relative.is_absolute() or ".." in relative.parts or "\\" in name:
                raise ValueError("Private relative input required")
            total += len(body)
            if total > LIMIT:
                raise ValueError("Private source transfer bound")
            item = tarfile.TarInfo(name)
            item.mode, item.size = (0o700 if executable else 0o600), len(body)
            archive.addfile(item, io.BytesIO(body))
    return buffer.getvalue()


def unpack_verified(raw, destination, *, owned_job=None):
    destination = Path(destination)
    if destination.is_symlink():
        raise ValueError("Private result destination symlink")
    destination = destination.resolve()
    cycles = 1
    if owned_job is not None:
        if (set(owned_job) != {"format", "run_id", "output", "resource_cycles", "job_sha256"}
                or owned_job["format"] != "connect/owned-job-output-transfer/1"
                or type(owned_job["resource_cycles"]) is not int or owned_job["resource_cycles"] not in (1, 3)):
            raise ValueError("Original owned collection profile required")
        cycles = owned_job["resource_cycles"]
    if len(raw) > LIMIT * cycles:
        raise ValueError("Private result transfer bound")
    values = {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
        for item in archive.getmembers():
            name = PurePosixPath(item.name)
            if (not item.isfile() or name.is_absolute() or ".." in name.parts or "\\" in item.name
                    or item.name in values or len(values) > 10000 + (1 if owned_job is not None else 0)):
                raise ValueError("Ordinary private result required")
            values[item.name] = archive.extractfile(item).read()
    manifest = json.loads(values.pop("TRANSFER-MANIFEST.json"))
    if {name: hashlib.sha256(body).hexdigest() for name, body in values.items()} != manifest:
        raise ValueError("Guest result transfer digest mismatch")
    if owned_job is not None:
        if json.loads(values.get("TRANSFER-PROFILE.json", b"null")) != owned_job:
            raise ValueError("Original owned collection profile mismatch")
    elif "TRANSFER-PROFILE.json" in values:
        raise ValueError("Original owned collection context required")
    for name, body in values.items():
        path = Path(destination) / name
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if any(p.is_symlink() for p in (path, *path.parents)):
            raise ValueError("Private result destination symlink")
        path.write_bytes(body)
        path.chmod(0o600)
    return manifest


def programme_source_files(root):
    """Exact private source closure, including the programme's relative contract."""
    root = Path(root)
    contract = "contracts/gate6-transport-matrix.json"
    names = subprocess.check_output(["git", "ls-files", "-z", "scripts", contract], cwd=root).decode().split("\0")
    files = {}
    for name in filter(None, names):
        path = root / name
        if path.is_file() and not path.is_symlink():
            files[name] = (path.read_bytes(), bool(path.stat().st_mode & 0o100))
    # transport_movement_profile.mjs reads this file during module import. Do
    # not borrow another workspace's contract or let omission become a later
    # unidentifiable Worker startup failure. stage() seals these original bytes.
    if contract not in files:
        raise ValueError("Original native programme contract required")
    for path in (root / "sdk/typescript/dist").rglob("*"):
        if path.is_file() and not path.is_symlink():
            files[str(path.relative_to(root))] = (path.read_bytes(), False)
    files["sdk/typescript/package.json"] = ((root / "sdk/typescript/package.json").read_bytes(), False)
    return files


def remote_browser_owner_joined(receipt):
    """Project proven original custody independently of the workload verdict."""
    if not isinstance(receipt, dict) or not isinstance(receipt.get("guest"), dict):
        return False
    guest = receipt["guest"]
    return (receipt.get("format") == "connect/mac-linux-owner/1"
        and guest.get("format") == "connect/mac-linux-original-job/1"
        and isinstance(receipt.get("run_id"), str) and bool(receipt["run_id"])
        and guest.get("run_id") == receipt["run_id"]
        and receipt.get("original_joined") is True
        and guest.get("original_joined") is True
        and type(guest.get("original_pid")) is int and guest["original_pid"] > 1
        and type(guest.get("original_start_ticks")) is int and guest["original_start_ticks"] > 0
        and type(guest.get("exit_code")) is int
        and receipt.get("all_guest_owners_joined") is True
        and receipt.get("original_vm_lease_joined") is True
        and receipt.get("forced") is False and guest.get("forced") is False
        and receipt.get("cleanup_failures") == []
        and receipt.get("client_inputs_unchanged") is True)


class MacLinuxFixture:
    def __init__(self, root, directory, env, *, run_command=subprocess.run, resource_cycles=1):
        if type(resource_cycles) is not int or resource_cycles not in (1, 3):
            raise ValueError("One or three original preflight cycles required")
        self.resource_cycles = resource_cycles
        self.lease_attempt = None
        self.root, self.directory, self.env = Path(root), Path(directory), env
        self.run_command = run_command
        self.run_id = "mac-remote-" + uuid.uuid4().hex
        self.remote_root = Path("/workspace/.local") / self.run_id
        self.directory.mkdir(mode=0o700, parents=True, exist_ok=False)
        self.owner = None
        self.latest = None
        self.alignments = []
        self.inputs = {}
        self.client_binary = self.peer_binary = None
        self.server_ip = self.client_ip = None
        self.closed = False
        self.lease = None
        self.lease_failure_recorded = False
        self.lease_cleanup = None

    def ssh(self, command):
        return ["colima", "--profile", "connect-gate6", "ssh", "--", "env",
                *(name + "=" + value for name, value in VM_ENV.items()),
                *(name + "=" + value for name, value in {
                    "CONNECT_MATRIX_STORAGE_CONTROL": os.environ.get("CONNECT_MATRIX_STORAGE_REMOTE_CONTROL"),
                    "CONNECT_MATRIX_STORAGE_TOKEN": os.environ.get("CONNECT_MATRIX_STORAGE_TOKEN")}.items() if value),
                *map(str, command)]

    def call(self, command, *, data=None, timeout=30):
        result = self.run_command(self.ssh(command), input=data, capture_output=True, timeout=timeout,
                                  cwd=self.root, env=self.env)
        if result.returncode:
            (self.directory / "guest-command-failure.log").write_bytes(result.stderr[-65536:])
            raise RuntimeError("Original ARM guest command failed")
        return result.stdout

    def start(self, revision, *, image=None):
        if (platform.system(), platform.machine()) != ("Darwin", "arm64"):
            raise ValueError("Original native Mac server required")
        interface = subprocess.check_output(["/sbin/route", "-n", "get", "default"], text=True, timeout=5)
        name = re.search(r"(?m)^\s*interface:\s*(\S+)\s*$", interface).group(1)
        self.server_ip = subprocess.check_output(["/usr/sbin/ipconfig", "getifaddr", name], text=True, timeout=5).strip()
        address = ipaddress.ip_address(self.server_ip)
        if not address.is_private or address.is_loopback or address.version != 4:
            raise ValueError("Actual private Mac interface required")
        code = """import json,platform,os,subprocess;from pathlib import Path
release=platform.freedesktop_os_release()
route=json.loads(subprocess.check_output(['ip','-j','route','get',os.sys.argv[1]]))[0]
print(json.dumps(dict(system=platform.system(),architecture=platform.machine(),release=release,uid=os.getuid(),gid=os.getgid(),route=route,revision=subprocess.check_output(['git','-C','/workspace','rev-parse','HEAD'],text=True).strip(),dirty=subprocess.check_output(['git','-C','/workspace','status','--porcelain'],text=True).strip(),boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())))"""
        observed = json.loads(self.call(["python3", "-c", code, self.server_ip]))
        if (observed["system"] != "Linux" or observed["architecture"] != "aarch64" or observed["uid"] == 0
                or observed["release"].get("ID") != "ubuntu" or observed["release"].get("VERSION_ID") != "24.04"
                or observed["revision"] != revision or observed["dirty"]):
            raise ValueError("Current ordinary-user Ubuntu ARM candidate required")
        self.client_ip = observed["route"]["prefsrc"]
        if not ipaddress.ip_address(self.client_ip).is_private or self.client_ip == self.server_ip:
            raise ValueError("Distinct actual guest route required")
        self.uid, self.gid, self.revision = observed["uid"], observed["gid"], revision
        self.acquire_lease()
        # This observation is after queueing for the original shared VM lease,
        # before source staging, compilation, namespace or client ownership.
        def observe_guest():
            return json.loads(self.call(["python3", "-c",
                "import json,os,shutil;v=os.statvfs('/workspace');u=shutil.disk_usage('/workspace');print(json.dumps(dict(free_bytes=u.free,available_inodes=v.f_favail,total_bytes=u.total)))"]))
        budget=guest_budget(borrowed_lease=getattr(self,"lease_parent",None) is not None)
        try:
            storage=await_admission(observe_guest,sum(budget.values()))
        except StorageAdmissionError as error:
            self.storage_refused_before_entry=True
            atomic_json(self.directory/"storage-admission.json",dict(error.observation,budget=budget))
            raise
        atomic_json(self.directory/"storage-admission.json",dict(storage,budget=budget))
        self.topology = dict(kind="macos-linux-guest", server_platform="macos-arm64", client_platform="ubuntu24-arm64",
            client_emulated=False, mac_interface=name, actual_server_ip=self.server_ip, actual_client_ip=self.client_ip,
            guest=observed, run_id=self.run_id)
        if image is not False:
            self.image = image or IMAGE
            value = json.loads(self.call(["docker", "image", "inspect", self.image]))[0]
            if value["Id"] != self.image or value["Architecture"] != "arm64" or value["Os"] != "linux":
                raise ValueError("Exact installed ARM browser image required")
            self.topology["browser_image_id"] = self.image
        self.stage(programme_source_files(self.root))
        (self.directory / "actual-topology.json").write_text(json.dumps(self.topology, indent=2) + "\n")
        return self.topology

    def acquire_lease(self):
        if getattr(self, "lease_parent", None) is not None:
            if self.lease_parent.lease is None or self.lease_parent.lease.poll() is not None:
                raise ValueError("Original parent VM workload lease unavailable")
            code = """import json,sys;from pathlib import Path
root,child=map(Path,sys.argv[1:]);child.mkdir(parents=True,exist_ok=True,mode=0o700)
sys.path.insert(0,'/workspace/scripts');from lib.matrix_storage import initialize_guest_reserve
initialize_guest_reserve(child)
p=root/'lease-children.json';values=json.loads(p.read_text()) if p.exists() else []
assert str(child) not in values;values.append(str(child));t=p.with_suffix('.tmp');t.write_text(json.dumps(values));t.replace(p)"""
            self.call(["python3", "-c", code, self.lease_parent.remote_root, self.remote_root])
            return
        code = """import json,sys,time;from pathlib import Path
sys.path.insert(0,'/workspace/scripts')
from lib.vm_workload_lease import VmWorkloadLease
sys.path.insert(0,'/workspace/scripts/lib')
from mac_linux_guest import lease_owners_joined,atomic
root=Path(sys.argv[1]);root.mkdir(parents=True,exist_ok=True,mode=0o700)
with VmWorkloadLease(timeout=int(sys.argv[2])):
 from lib.matrix_storage import initialize_guest_reserve
 initialize_guest_reserve(root)
 print('original-vm-lease-acquired',flush=True)
 sys.stdin.buffer.read()
 while not lease_owners_joined(root):
  atomic(root/'lease-state.json',{'state':'blocked','reason':'original-guest-owner-join-unconfirmed'})
  time.sleep(.25)
 atomic(root/'lease-state.json',{'state':'released','original_owners_joined':True})
"""
        queue_timeout = 1200 * self.resource_cycles
        self.lease_attempt = dict(format="connect/mac-linux-lease-acquisition/1", resource_cycles=self.resource_cycles,
            queue_timeout_seconds=queue_timeout, host_wait_seconds=queue_timeout + 10,
            started_unix=time.time(), started_monotonic_seconds=time.monotonic(), acquired=False)
        self.lease = subprocess.Popen(self.ssh(["python3", "-c", code, self.remote_root, str(queue_timeout)]), cwd=self.root, env=self.env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(self.lease.stdout, selectors.EVENT_READ)
                if not selector.select(timeout=queue_timeout + 10) or self.lease.stdout.readline() != b"original-vm-lease-acquired\n":
                    self.lease.stdin.close()
                    self.lease.wait(timeout=5)
                    raise ValueError("Existing ARM VM workload lease unavailable")
        except BaseException:
            # Preserve the failed/interrupted acquisition before stopping its
            # exact original queue session. No guest workload exists yet.
            try:
                self._record_failed_lease()
            finally:
                self._close_preacquisition_lease()
            raise
        self.lease_attempt["acquired"] = True
        self.lease_attempt["acquired_unix"] = time.time()
        (self.directory / "lease-acquisition.json").write_text(json.dumps(self.lease_attempt) + "\n")

    def _record_failed_lease(self):
        if self.lease_failure_recorded:
            return
        code = self.lease.poll()
        group_joined = False
        if code is not None:
            try:
                os.killpg(self.lease.pid, 0)
            except ProcessLookupError:
                group_joined = True
            except PermissionError:
                pass
        details = dict(self.lease_attempt, ended_unix=time.time(), ended_monotonic_seconds=time.monotonic(),
            original_ssh_pid=self.lease.pid, original_ssh_exit_code=code,
            original_ssh_process_exited=code is not None, original_ssh_group_joined=group_joined,
            guest_job_created=self.owner is not None, guest_inputs_staged=bool(self.inputs))
        try:
            os.set_blocking(self.lease.stderr.fileno(), False)
            raw = self.lease.stderr.read(65537) or b""
            if raw:
                stderr_path = self.directory / "lease-acquisition.stderr"
                stderr_path.write_bytes(raw[:65536])
                details.update(stderr_sha256=hashlib.sha256(raw[:65536]).hexdigest(), stderr_truncated=len(raw) > 65536)
        except (OSError, ValueError):
            details["stderr_available"] = False
        self.lease_attempt = details
        (self.directory / "lease-acquisition.json").write_text(json.dumps(details) + "\n")
        self.lease_failure_recorded = True

    def _close_preacquisition_lease(self):
        if self.lease_cleanup is not None:
            return self.lease_cleanup
        result = dict(original_ssh_pid=self.lease.pid, original_ssh_exit_code=None,
            original_ssh_process_exited=False, original_ssh_group_joined=False,
            forced=False, stop_errors=[])
        try:
            self.lease.stdin.close()
        except (OSError, ValueError) as error:
            result["stop_errors"].append("stdin-close-" + type(error).__name__)
        try:
            for selected in (None, signal.SIGTERM, signal.SIGKILL):
                if selected is not None and self.lease.poll() is None:
                    result["forced"] = True
                    try:
                        if os.getpgid(self.lease.pid) != self.lease.pid:
                            raise RuntimeError("Original queue no longer leads its session")
                        os.killpg(self.lease.pid, selected)
                    except ProcessLookupError:
                        pass
                    except (OSError, RuntimeError) as error:
                        result["stop_errors"].append("group-signal-" + type(error).__name__)
                        # Signal only the original Popen if its group is denied.
                        # A group error must never bypass waiting on that child.
                        try:
                            self.lease.send_signal(selected)
                        except OSError as direct_error:
                            result["stop_errors"].append("child-signal-" + type(direct_error).__name__)
                try:
                    result["original_ssh_exit_code"] = self.lease.wait(timeout=5)
                    result["original_ssh_process_exited"] = True
                    break
                except (OSError, subprocess.TimeoutExpired) as error:
                    result["stop_errors"].append("child-wait-" + type(error).__name__)
        finally:
            if result["original_ssh_process_exited"]:
                try:
                    os.killpg(self.lease.pid, 0)
                except ProcessLookupError:
                    result["original_ssh_group_joined"] = True
                except PermissionError:
                    result["stop_errors"].append("group-observation-PermissionError")
                else:
                    # The original Popen has already been reaped. Its numeric
                    # PGID alone cannot safely authorize a later group signal.
                    # Preserve this residual custody as unconfirmed.
                    result["stop_errors"].append("original-leader-exited-group-still-present")
            for stream in (self.lease.stdout, self.lease.stderr):
                try:
                    stream.close()
                except (OSError, ValueError) as error:
                    result["stop_errors"].append("pipe-close-" + type(error).__name__)
        # This is current HOST queue custody only. It cannot establish a guest
        # workload join or claim release of an unobserved guest lease.
        self.lease_cleanup = result
        atomic_json(self.directory / "lease-cleanup.json", result)
        return result

    def owned_arguments(self, command, *, timeout, pass_stdin=False):
        identity = "command-" + uuid.uuid4().hex
        intent = {"run_id":self.run_id,"owner_id":identity,"command":list(map(str,command)),"timeout":timeout,"pass_stdin":pass_stdin}
        self.stage({".owners/"+identity+"-intent.json":((json.dumps(intent)+"\n").encode(),False)})
        return ["python3",self.remote_root/"scripts/lib/mac_linux_guest.py","owned-command",self.remote_root,identity]

    def owned_call(self, command, *, timeout):
        return self.call(self.owned_arguments(command,timeout=timeout),timeout=timeout+20)

    def mapped(self, path):
        return self.remote_root / Path(path).resolve().relative_to(self.root.resolve())

    def stage(self, files):
        self.call(["python3", GUEST, "receive", self.remote_root], data=archive_files(files), timeout=90)
        self.inputs.update({name: hashlib.sha256(body).hexdigest() for name, (body, _) in files.items()})
        (self.directory / "client-inputs.json").write_text(json.dumps(self.inputs, sort_keys=True) + "\n")

    def stage_paths(self, paths):
        files = {}
        for source in paths:
            source = Path(source)
            for path in ([source] if source.is_file() else source.rglob("*")):
                if path.is_symlink():
                    raise ValueError("Ordinary private client inputs required")
                if path.is_file():
                    files[str(path.resolve().relative_to(self.root.resolve()))] = (path.read_bytes(), bool(path.stat().st_mode & 0o100))
        self.stage(files)

    def stage_json(self, path, value):
        name = str(Path(path).resolve().relative_to(self.root.resolve()))
        rewritten = rewrite_paths(value, self.root, self.remote_root)
        self.stage({name: ((json.dumps(rewritten, indent=2) + "\n").encode(), False)})
        return rewritten

    def stage_native_config(self, path):
        path = Path(path)
        value = tomllib.loads(path.read_text())
        control = value.get("control", value)
        assets = []
        for key in ("trust_certificate", "signing_private_key", "server_public_key"):
            if key in control:
                asset = Path(control[key])
                if not asset.is_absolute():
                    asset = path.parent / asset
                assets.append(asset)
        self.stage_paths(assets)
        # Native clients use signed QUIC. The Mac's private administration HTTP
        # endpoint is deliberately not made into a guest administration route.
        if "control" in value:
            value.pop("application", None)
        value = rewrite_paths(value, self.root, self.remote_root)
        self.stage({str(path.resolve().relative_to(self.root.resolve())): (toml_bytes(value), False)})
        return self.mapped(path)

    def compile_native(self, *, media):
        code = """import hashlib,json,os,shutil,subprocess,sys;from pathlib import Path
root=Path(sys.argv[1]);media=sys.argv[2]=='1';os.chdir('/workspace');root.joinpath('.local/bin').mkdir(parents=True,exist_ok=True)
with (root/'dependencies.log').open('wb') as out:subprocess.run(['bash','scripts/prepare-dependencies.sh'],stdout=out,stderr=subprocess.STDOUT,check=True,timeout=600)
result={}
builds=[('connect',['cargo','build','--release','--locked','--message-format=json','-p','connect-server','--bin','connect'])]
if media:builds.append(('connect-external-rust-journey',['cargo','build','--release','--locked','--message-format=json','--manifest-path','examples/rust-sdk-journey/Cargo.toml']))
for name,command in builds:
 log=root/(name+'-compile.log')
 with log.open('wb') as out:subprocess.run(command,stdout=out,stderr=subprocess.STDOUT,check=True,timeout=3600)
 paths=[]
 for line in log.read_text().splitlines():
  try:v=json.loads(line)
  except ValueError:continue
  if v.get('reason')=='compiler-artifact' and v['target']['name']==name and v.get('executable'):
   assert not v['profile']['test'] and v['profile']['opt_level']=='3' and 'test-support' not in v['features'];paths.append(Path(v['executable']))
 assert len(paths)==1
 raw=paths[0].read_bytes();assert raw[:6]==bytes.fromhex('7f454c460201') and int.from_bytes(raw[18:20],'little')==183
 destination=root/'.local/bin'/name;shutil.copyfile(paths[0],destination);destination.chmod(0o700)
 result[name]={'path':str(destination),'sha256':hashlib.sha256(raw).hexdigest()}
result['node']=subprocess.check_output(['node','--version'],text=True).strip()
result['rust']=subprocess.check_output(['rustc','-Vv'],text=True)
print(json.dumps(result))"""
        result = json.loads(self.owned_call(["python3", "-c", code, self.remote_root, "1" if media else "0"], timeout=7200))
        if "rustc 1.97.1 " not in result["rust"] or "host: aarch64-unknown-linux-gnu" not in result["rust"] or int(result["node"].lstrip("v").split(".")[0]) < 22:
            raise ValueError("Pinned native ARM client toolchain required")
        self.client_binary = result["connect"]["path"]
        self.client_binary_sha256 = result["connect"]["sha256"]
        self.peer_binary = result.get("connect-external-rust-journey", {}).get("path")
        self.peer_binary_sha256 = result.get("connect-external-rust-journey", {}).get("sha256")
        self.native_runtime = result
        (self.directory / "guest-build.json").write_text(json.dumps(result, indent=2) + "\n")
        return result

    def native_command(self, command):
        arguments = list(map(str, command[1:]))
        if "--config" in arguments:
            position = arguments.index("--config") + 1
            arguments[position] = str(self.stage_native_config(Path(arguments[position])))
        return self.ssh(self.owned_arguments([self.client_binary, *arguments], timeout=25, pass_stdin=True))

    def prepare_sources(self, directory):
        self.source_directory = Path(directory)
        target = self.mapped(directory)
        required = sum(source_budget().values())
        storage = json.loads(self.call(["python3", "-c", "import json,shutil;print(json.dumps(shutil.disk_usage('/workspace')._asdict()))"]))
        storage.update(required_bytes=required, budget=source_budget(),
                       owner_run_id=self.run_id, workload_lease_held=True)
        atomic_json(self.directory / "source-storage.json", storage)
        if storage["free"] < required:
            raise ValueError("Finite ARM guest media source storage unavailable")
        # Raw I420 plus its executable must be produced on the actual client OS.
        # The source helper checks the exact finite space requirement first.
        code = """import json,sys;from pathlib import Path
sys.path.insert(0,str(Path(sys.argv[1])/'scripts'))
from lib.transport_av import prepare
target=Path(sys.argv[2])
# The new guest root has source code, not the host's nested artifact parents.
# Keep prepare's exclusive leaf creation: an earlier source run cannot be reused.
target.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
print(json.dumps(prepare(target)))"""
        value = json.loads(self.owned_call(["python3", "-c", code, self.remote_root, target], timeout=1200))
        directory.mkdir(mode=0o700, parents=True, exist_ok=False)
        (directory / "manifest.json").write_text(json.dumps(value, indent=2) + "\n")
        # Keep the exact remote manifest bytes, whose hash is used by the client.
        original = self.call(["cat", target / "manifest.json"])
        (directory / "manifest.json").write_bytes(original)
        return value

    def server_proof(self, pid, executable, expected_sha256, port):
        actual = subprocess.check_output(["/bin/ps", "-ww", "-p", str(pid), "-o", "comm="], text=True, timeout=5).strip()
        if Path(actual) != Path(executable) or hashlib.sha256(Path(actual).read_bytes()).hexdigest() != expected_sha256:
            raise ValueError("Original Mac server executable differs")
        endpoint = f"{self.server_ip}:{port}"
        sockets = subprocess.check_output(["/usr/sbin/lsof", "-nP", "-a", "-p", str(pid), "-iUDP@" + endpoint, "-Fpn"], text=True, timeout=5).splitlines()
        if not sockets or sockets[0] != "p" + str(pid) or "n" + endpoint not in sockets:
            raise ValueError("Original Mac server UDP listener unavailable")
        boot = subprocess.check_output(["/usr/sbin/sysctl", "-n", "kern.bootsessionuuid"], text=True, timeout=5).strip()
        return dict(kind="macos-linux-guest", run_id=self.run_id, source_revision=self.revision,
            server_platform="macos-arm64", server_pid=pid, server_binary_sha256=expected_sha256,
            server_endpoint=endpoint, server_owned_udp_bind_observed=True,
            kernel_identity_sha256=hashlib.sha256(boot.encode()).hexdigest(),
            guest_boot_id=self.topology["guest"]["boot_id"], client_ip=self.client_ip)

    def verify_binaries(self):
        expected = {self.client_binary: self.client_binary_sha256}
        if self.peer_binary:
            expected[self.peer_binary] = self.peer_binary_sha256
        code = "import hashlib,json,sys;from pathlib import Path;v=json.load(sys.stdin);print(json.dumps({p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in v}))"
        observed = json.loads(self.call(["python3", "-c", code], data=json.dumps(expected).encode()))
        return observed == expected

    def verify_inputs(self):
        code = "import hashlib,json,sys;from pathlib import Path;r=Path(sys.argv[1]);v=json.load(sys.stdin);print(json.dumps({p:hashlib.sha256((r/p).read_bytes()).hexdigest() for p in v}))"
        actual = json.loads(self.call(["python3", "-c", code, self.remote_root], data=json.dumps(self.inputs).encode(), timeout=30))
        return actual == self.inputs

    def run_native_programme(self, command, log, *, directory, server_pid, endpoint, credential, capture=None, budget=300, resource_cycles=1):
        from .native_matrix_idle import NativeMatrixIdle, NativeMatrixPlateau, health_reader
        remote = self
        class GuestIdle(NativeMatrixIdle):
            def request(self, phase):
                path = self.directory / ("resources-" + self.prefix + phase + ".json")
                if not path.exists():
                    return None
                if path.is_symlink() or path.stat().st_size > 4096:
                    raise ValueError("Bounded original guest resource request required")
                value = json.loads(path.read_text())
                from .native_programme_profile import original_worker_owner
                keys = {"phase", "monotonic_seconds", "programme_owner"} | ({"retirement"} if phase == "retired" else set())
                if (set(value) != keys or value["phase"] != phase
                        or not original_worker_owner(value.get("programme_owner"))):
                    raise ValueError("Original guest resource phase or programme Worker differs")
                return dict(value, original_monotonic_seconds=value["monotonic_seconds"],
                    monotonic_seconds=remote.host_time(value["monotonic_seconds"]))
        directory = Path(directory)
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        owned = self.launch(command, directory, log, timeout=budget, role="helper", capture=capture, resource_cycles=resource_cycles)
        sampler = MacLinuxSampler(self, server_pid, role="helper", resource_cycles=resource_cycles)
        idle = (NativeMatrixPlateau(directory, sampler, health_reader(endpoint, credential), child_class=GuestIdle)
                if resource_cycles == 3 else GuestIdle(directory, sampler, health_reader(endpoint, credential)))
        cutoff = time.monotonic() + budget + 10
        try:
            while owned.poll() is None:
                self.poll()
                if (self.latest.get("state") or {}).get("resources"):
                    idle.poll()
                if time.monotonic() >= cutoff:
                    raise ValueError("Original guest programme deadline")
                time.sleep(.05)
        finally:
            cleanup = self.close(release_lease=False)
        result = idle.finish()
        result["clock_alignment"] = dict(method="retained actual guest monotonic and bounded Mac round trip", observations=self.alignments)
        (directory / "resource-observation.json").write_text(json.dumps(result, indent=2) + "\n")
        resources = dict(format="connect/mac-linux-native-process-resources/1",
            collection_complete=bool(sampler.samples) and cleanup["original_joined"] and not cleanup["forced"],
            original_owner=cleanup, observation=sampler.summary(), scope="Actual separately owned Mac server and native Ubuntu ARM client tree")
        (directory / "process-resources.json").write_text(json.dumps(resources, indent=2) + "\n")
        return owned.returncode, resources, result

    def launch(self, command, output, log, *, timeout=300, container=None, role="browser", capture=None, resource_cycles=1):
        if type(resource_cycles) is not int or resource_cycles not in (1, 3):
            raise ValueError("One or three original guest cycles required")
        if not 1 <= timeout <= (900 if resource_cycles == 3 else 360):
            raise ValueError("Original per-cycle guest budget required")
        if self.owner is not None:
            raise ValueError("One original guest workload per owner")
        self.output = Path(output)
        self.output_relative = str(self.output.resolve().relative_to(self.root.resolve()))
        config = dict(run_id=self.run_id, source_revision=self.revision, command=list(map(str, command)),
                      output=self.output_relative, timeout_seconds=timeout, container=container, role=role, capture=capture, resource_cycles=resource_cycles)
        self.stage({"job.json": ((json.dumps(config) + "\n").encode(), False), "job-intent.json": ((json.dumps({"run_id":self.run_id})+"\n").encode(),False)})
        self.owned_output_profile = dict(format="connect/owned-job-output-transfer/1", run_id=self.run_id,
            output=self.output_relative, resource_cycles=resource_cycles, job_sha256=self.inputs["job.json"])
        self.log = Path(log).open("xb")
        self.owner = subprocess.Popen(self.ssh(["python3", self.remote_root / "scripts/lib/mac_linux_guest.py", "run", self.remote_root]),
            cwd=self.root, env=self.env, stdin=subprocess.DEVNULL, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        return self.owner

    def poll(self):
        responses = {}
        directories = [self.output, *(self.output / "cycles" / ("cycle-" + str(cycle))
                                     for cycle in (1, 2, 3))]
        for directory in directories:
            if directory.is_symlink() or directory.parent.is_symlink():
                raise ValueError("Original bridge directory cannot use a symlink")
            patterns = (("authority-*-response.json", "resources-*-response.json", "revoke-response.json")
                        if directory == self.output else ("authority-*-response.json", "resources-*-response.json"))
            for pattern in patterns:
                for path in directory.glob(pattern):
                    if path.is_symlink() or path.stat().st_size > 4096:
                        raise ValueError("Bounded private response required")
                    responses[path.relative_to(self.output).as_posix()] = json.loads(path.read_text())
        started = time.monotonic()
        value = json.loads(self.call(["python3", self.remote_root / "scripts/lib/mac_linux_guest.py", "exchange", self.remote_root],
            data=json.dumps(dict(directory=self.output_relative, responses=responses)).encode(), timeout=5))
        finished = time.monotonic()
        if finished - started > 1:
            raise ValueError("Cross-host clock observation exceeds one second")
        alignment = dict(host_before=started, host_after=finished, guest_monotonic_seconds=value["monotonic_seconds"])
        self.alignments.append(alignment)
        self.latest = value
        for name, request in value["requests"].items():
            path = self.output / name
            allowed = {*(f"authority-{index}-request.json" for index in (1, 2, 3)), "revoke-request.json",
                *(f"cycles/cycle-{cycle}/authority-{index}-request.json" for cycle in (1, 2, 3) for index in (1, 2, 3)),
                *(f"resources-{prefix}{phase}.json" for prefix in ("", "cycle-1-", "cycle-2-", "cycle-3-")
                  for phase in ("baseline", "terminal", "retired")),
                *(f"cycles/cycle-{cycle}/resources-{phase}.json" for cycle in (1, 2, 3)
                  for phase in ("baseline", "terminal", "retired"))}
            if (name not in allowed or any(parent.is_symlink() for parent in (path, path.parent, path.parent.parent))
                    or len(json.dumps(request).encode()) > 4096):
                raise ValueError("Bounded original cross-host request required")
            if path.exists() and json.loads(path.read_text()) != request:
                raise ValueError("Original cross-host request changed")
            if not path.exists():
                path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                path.write_text(json.dumps(request) + "\n")
                path.chmod(0o600)
        return value

    def host_time(self, guest_time):
        alignment = self.alignments[-1]
        age = alignment["guest_monotonic_seconds"] - guest_time
        if not 0 <= age < 30:
            raise ValueError("Current original guest monotonic observation required")
        return (alignment["host_before"] + alignment["host_after"]) / 2 - age

    def collect(self):
        raw = self.call(["python3", self.remote_root / "scripts/lib/mac_linux_guest.py", "collect", self.remote_root, self.output_relative], timeout=90)
        profile = getattr(self, "owned_output_profile", None)
        if profile is not None and (profile["run_id"] != self.run_id or profile["output"] != self.output_relative
                or profile["job_sha256"] != self.inputs.get("job.json")):
            raise ValueError("Original owned collection context changed")
        manifest = unpack_verified(raw, self.output, owned_job=profile)
        (self.directory / "output-transfer.json").write_text(json.dumps(manifest, sort_keys=True) + "\n")
        (self.directory / "clock-alignments.json").write_text(json.dumps(self.alignments) + "\n")
        return manifest

    def collect_directory(self, directory):
        relative = str(Path(directory).resolve().relative_to(self.root.resolve()))
        raw = self.call(["python3", self.remote_root / "scripts/lib/mac_linux_guest.py", "collect", self.remote_root, relative], timeout=90)
        return unpack_verified(raw, directory)

    def close(self, *, release_lease=True):
        if self.closed:
            if release_lease:
                self.release_lease()
            return self.receipt
        if (self.lease_attempt is not None and not self.lease_attempt["acquired"]
                and self.owner is None and not self.inputs and self.lease is not None):
            try:
                self._record_failed_lease()
            finally:
                cleanup = self._close_preacquisition_lease()
            self.receipt = dict(format="connect/mac-linux-owner/1", run_id=self.run_id, passed=False,
                preflight_only=True, guest_job_started=False, guest_sources_staged=False,
                guest_namespace_started=False, original_joined=False, forced=cleanup["forced"],
                original_lease_ssh_joined=cleanup["original_ssh_group_joined"],
                lease_acquisition=dict(self.lease_attempt),
                lease_cleanup=cleanup,
                cleanup_scope="Only the original pre-acquisition SSH process existed. No guest workload owner was created.",
                cleanup_failures=[] if cleanup["original_ssh_group_joined"] else ["original-lease-ssh-join-unconfirmed"])
            self.closed = True
            return self.receipt
        if (getattr(self, "storage_refused_before_entry", False)
                and self.owner is None and not self.inputs):
            script = "import sys,json;from pathlib import Path;sys.path.insert(0,'/workspace/scripts/lib');from mac_linux_guest import lease_owners_joined;print(json.dumps(lease_owners_joined(Path(sys.argv[1]))))"
            absent = json.loads(self.call(["python3", "-c", script, self.remote_root], timeout=10)) is True
            self.receipt = dict(format="connect/mac-linux-owner/1",run_id=self.run_id,passed=False,
                preflight_only=True,guest_job_started=False,original_joined=False,forced=False,
                unentered_custody_confirmed=absent,all_guest_owners_joined=absent,
                client_inputs_unchanged=True,cleanup_failures=[] if absent else ["unentered-guest-custody-unconfirmed"])
            self.closed = True
            if release_lease:
                self.release_lease()
            return self.receipt
        failures = []
        forced = self.owner is not None and self.owner.poll() is None
        code = None
        actual = {}
        if forced:
            try:
                self.call(["python3", self.remote_root / "scripts/lib/mac_linux_guest.py", "stop", self.remote_root], timeout=5)
            except Exception as error:
                failures.append("guest-stop-" + type(error).__name__)
        if self.owner is not None:
            try:
                code = self.owner.wait(timeout=20)
            except subprocess.TimeoutExpired:
                failures.append("ssh-owner-join-timeout")
                os.killpg(self.owner.pid, signal.SIGTERM)
                try:
                    code = self.owner.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(self.owner.pid, signal.SIGKILL)
                    code = self.owner.wait(timeout=5)
            finally:
                self.log.close()
            try:
                self.collect()
                actual = json.loads((self.output / "guest-owner.json").read_text())
            except Exception as error:
                failures.append("guest-artifact-" + type(error).__name__)
        self.receipt = dict(format="connect/mac-linux-owner/1", run_id=self.run_id,
            original_joined=code is not None, guest=actual, forced=forced or actual.get("forced", False),
            passed=code == 0 and actual.get("original_joined") is True and not forced and not actual.get("forced") and not failures,
            cleanup_failures=failures)
        try:
            self.receipt["client_inputs_unchanged"] = self.verify_inputs()
            if not self.receipt["client_inputs_unchanged"]:
                failures.append("client-input-integrity")
        except Exception as error:
            failures.append("client-input-integrity-" + type(error).__name__)
            self.receipt["client_inputs_unchanged"] = False
        guest_owners_joined = False
        try:
            script = "import sys,json;from pathlib import Path;sys.path.insert(0,str(Path(sys.argv[1])/'scripts/lib'));from mac_linux_guest import lease_owners_joined;print(json.dumps(lease_owners_joined(Path(sys.argv[1]))))"
            guest_owners_joined = json.loads(self.call(["python3", "-c", script, self.remote_root], timeout=10)) is True
            self.receipt["all_guest_owners_joined"] = guest_owners_joined
            if not guest_owners_joined:
                failures.append("original-guest-owners-unconfirmed")
        except Exception as error:
            failures.append("guest-ownership-" + type(error).__name__)
        if getattr(self, "source_directory", None) and guest_owners_joined:
            source = self.mapped(self.source_directory)
            # Reclaim only this run's finite raw sources after its job joined.
            # Their exact original manifest and byte digests remain in evidence.
            script = """import json,sys;from pathlib import Path
p=Path(sys.argv[1]);removed=[]
for i in range(6):
 f=p/('source-'+str(i)+'.i420')
 if f.exists():removed.append({'name':f.name,'bytes':f.stat().st_size});f.unlink()
print(json.dumps(removed))"""
            try:
                self.receipt["raw_source_cleanup"] = json.loads(self.call(["python3", "-c", script, source], timeout=30))
            except Exception as error:
                failures.append("raw-source-cleanup-" + type(error).__name__)
        self.receipt["passed"] &= not failures
        self.closed = True
        if release_lease:
            self.release_lease()
        return self.receipt

    def release_lease(self):
        if self.lease_cleanup is not None:
            return
        if self.lease is None or self.receipt.get("original_vm_lease_joined") is not None:
            if self.lease is None:
                self.receipt["borrowed_original_vm_lease"] = getattr(self, "lease_parent", None) is not None
            return
        try:
            self.lease.stdin.close()
            # The guest lock owner requires receipts and actual process/group/
            # container absence. An elapsed deadline never releases the lock.
            status = self.lease.wait(timeout=380)
            self.receipt["original_vm_lease_joined"] = status == 0
            self.receipt["passed"] &= status == 0
        except Exception as error:
            self.receipt["cleanup_failures"].append("vm-lease-join-" + type(error).__name__)
            self.receipt["original_vm_lease_joined"] = False
            self.receipt["passed"] = False
        finally:
            self.lease.stdout.close()
            self.lease.stderr.close()


class MacLinuxSampler:
    def __init__(self, remote, server_pid, *, role="browser", resource_cycles=1):
        self.remote, self.role = remote, role
        self.server = MacProcessResources({"server": server_pid}, roles=("server",), max_samples=300 if resource_cycles == 1 else 1800)
        self.samples = []
        self.root = None

    def sample(self):
        latest = self.remote.latest
        state = latest["state"]
        if state is None or "resources" not in state or not 0 <= latest["monotonic_seconds"] - state["sampled_at_monotonic_seconds"] <= 1.5:
            raise ValueError("Live original ARM guest sample unavailable")
        identity = (state["run_id"], state["sampled_root"], state["sampled_root_start_ticks"])
        if self.root is None:
            self.root = identity
        if identity != self.root or state["source_revision"] != self.remote.revision:
            raise ValueError("Guest process owner changed")
        groups = self.server.sample()["groups"]
        groups[self.role] = state["resources"]["groups"][self.role]
        value = dict(groups=groups, host_monotonic_seconds=time.monotonic(),
                     guest_monotonic_seconds=state["sampled_at_monotonic_seconds"], alignment=self.remote.alignments[-1])
        self.samples.append(value)
        return value

    def owner_identities(self):
        state = self.remote.latest["state"]
        if self.root != (state["run_id"], state["sampled_root"], state["sampled_root_start_ticks"]):
            raise ValueError("Original remote process changed")
        birth = state["sampled_root_start_ticks"]
        if type(birth) is not int or birth <= 0:
            raise ValueError("Original remote process birth unavailable")
        result = {role: dict(pid=identity[0], start_identity=identity[1], host_profile="macos-arm64")
                  for role, identity in self.server.roots.items()}
        result[self.role] = dict(pid=state["sampled_root"], start_identity=birth, host_profile="ubuntu24-arm64")
        return result

    def summary(self):
        return dict(scope="Actual Mac server libproc and original Ubuntu ARM guest process tree; retained raw monotonic clock alignments.",
                    samples=self.samples, server=self.server.summary(), run_id=self.remote.run_id)


class MacLinuxIdle(MatrixIdle):
    def __init__(self, directory, server_pid, remote, endpoint, credential, *, cycle_mode=False, original_baseline=None, sampler=None, resource_cycles=1):
        self.remote = remote
        super().__init__(directory, server_pid, -1, endpoint, credential,
                         sampler=sampler or MacLinuxSampler(remote, server_pid, resource_cycles=resource_cycles),
                         cycle_mode=cycle_mode, original_baseline=original_baseline, resource_cycles=resource_cycles)

    def request(self, phase):
        path = self.directory / ("resources-" + phase + ".json")
        if not path.exists():
            return None
        if path.is_symlink() or path.stat().st_size > 1024:
            raise ValueError("Bounded original guest resource request required")
        value = json.loads(path.read_text())
        if set(value) != {"phase", "monotonic_seconds"} or value["phase"] != phase:
            raise ValueError("Original guest resource phase differs")
        return self.remote.host_time(value["monotonic_seconds"])

    def finish(self):
        value = super().finish()
        value["clock_alignment"] = dict(method="retained guest monotonic sample within bounded Mac round trip",
            maximum_round_trip_seconds=1, observations=self.remote.alignments)
        value["scope"] = "Original Mac server and Ubuntu ARM guest browser owner; each process sampled on its actual host and terminal translated using retained clock alignments. No process restart or forced GC."
        (self.directory / "resource-observation.json").write_text(json.dumps(value, indent=2) + "\n")
        return value
