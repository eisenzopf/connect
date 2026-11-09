"""Serialize measured workloads on the one existing owner ARM VM."""
import fcntl
import json
import os
from pathlib import Path
import selectors
import signal
import stat
import subprocess
import sys
import time

try:
    from .matrix_storage import MetadataReserve, GUARD_RESERVE_BYTES, atomic_json, metadata_failure
except ImportError:  # Original standalone guardian entry point.
    from matrix_storage import MetadataReserve, GUARD_RESERVE_BYTES, atomic_json, metadata_failure


class VmWorkloadLease:
    def __init__(self, *, directory=None, timeout=1200):
        self.directory = Path(directory or f"/tmp/connect-gate6-workload-{os.getuid()}")
        self.timeout = timeout
        self.file = None

    def __enter__(self):
        self.directory.mkdir(mode=0o700, exist_ok=True)
        observed = self.directory.lstat()
        if not stat.S_ISDIR(observed.st_mode) or observed.st_uid != os.getuid() or observed.st_mode & 0o077:
            raise ValueError("Original private VM workload lease required")
        descriptor = os.open(self.directory / "lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        self.file = os.fdopen(descriptor, "r+")
        cutoff = time.monotonic() + self.timeout
        while True:
            try:
                fcntl.flock(self.file, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except BlockingIOError:
                if time.monotonic() >= cutoff:
                    self.file.close()
                    self.file = None
                    raise TimeoutError("Existing ARM VM workload lease deadline")
                time.sleep(.05)

    def __exit__(self, *_):
        if self.file is not None:
            fcntl.flock(self.file, fcntl.LOCK_UN)
            self.file.close()
            self.file = None


def _native_clients_joined(report, profile):
    if not isinstance(profile, dict):
        return False
    if profile.get("format") == "connect/mac-linux-native-process-resources/1":
        # This producer records the same actual guest owner as the remote
        # retention path. A failed programme is not an unknown physical owner.
        from .mac_linux_fixture import remote_browser_owner_joined
        owner = profile.get("original_owner")
        if not remote_browser_owner_joined(owner):
            return False
        guest = owner["guest"]
        revision = report.get("source_revision")
        capture = report.get("packet_capture", {})
        if not isinstance(capture, dict):
            return False
        if capture.get("owner_created") is True and guest.get("capture_owner_created") is not True:
            return False
        if ("capture_owner_created" in guest or "capture_original_joined" in guest):
            if (type(guest.get("capture_owner_created")) is not bool
                    or guest.get("capture_original_joined") is not True):
                return False
        return (profile.get("collection_complete") is True
            and profile.get("forced", False) is False
            and profile.get("forced_cleanup", False) is False
            and owner == report.get("remote_client_owner")
            and isinstance(revision, str) and len(revision) == 40
            and all(c in "0123456789abcdef" for c in revision)
            and guest.get("source_revision") == revision)
    if ("original_owner" in profile or profile.get("format") not in
            (None, "connect/native-workload-process-profile/1")):
        return False  # Unknown schemas cannot borrow current local flat flags.
    return (profile.get("original_supervisor_joined") is True
        and profile.get("original_workers_joined") is True)


def cleanup_evidence(report):
    """Only original owner joins permit another measured workload on this VM."""
    if not isinstance(report, dict):
        return dict(confirmed=False, reason="original-cell-report-unavailable")
    servers = report.get("servers", [])
    server_cleanup = report.get("failure_cleanup", {})
    servers_joined = all(
        (v.get("original_process_group_joined") is True
            and type(v.get("original_exit_code")) is int
            and type(v.get("original_pid")) is int and v["original_pid"] > 1)
        if "original_process_group_joined" in v else v.get("original_joined") is True
        for v in servers)
    if server_cleanup and server_cleanup.get("original_group_retired") is not True:
        servers_joined = False
    browser = report.get("format") == "connect/browser-matrix-component/1"
    client = report.get("native_process_profile", {})
    clients_joined = (report.get("browser_owner_started") is False or report.get("browser_owner_joined") is True if browser else
        _native_clients_joined(report, client))
    if (not browser and report.get("native_programme_started") is False
            and "native_process_profile" not in report):
        clients_joined = True  # The producer has not entered its programme owner boundary.
    namespace = report.get("network_namespace")
    namespace_joined = namespace is None or (namespace.get("original_namespace_removed") is True and
        namespace.get("original_veth_removed") is True and namespace.get("remaining_client_processes") == 0)
    capture = report.get("packet_capture", {})
    capture_joined = not capture or capture.get("original_joined") is True or capture.get("owner_created") is False
    authority = report.get("current_authority", {})
    authority_joined = all(
        (value.get("executed") is False and value.get("launcher_attempted") is False
            and "process_owner" not in value) or
        (isinstance(value.get("process_owner"), dict)
            and value["process_owner"].get("original_supervisor_joined") is True
            and value["process_owner"].get("original_workers_joined") is True)
        for name, value in authority.items() if name in {"expiry", "revocation"}) if not browser else True
    return dict(confirmed=servers_joined and clients_joined and namespace_joined and capture_joined and authority_joined,
        servers_joined=servers_joined, clients_joined=clients_joined, namespace_joined=namespace_joined,
        capture_joined=capture_joined, authority_joined=authority_joined)


class GuardedVmWorkloadLease:
    """Separate lock owner survives a lost Linux worker until joins are proven."""
    def __init__(self):
        self.owner = None
        self.evidence = dict(confirmed=False, reason="original-worker-ended-without-cleanup-evidence")
        self.blocked = False

    def __enter__(self):
        self.owner = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--guard"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True)
        with selectors.DefaultSelector() as selector:
            selector.register(self.owner.stdout, selectors.EVENT_READ)
            if not selector.select(timeout=1210) or self.owner.stdout.readline() != b"acquired\n":
                self.owner.stdin.close()
                try:
                    self.owner.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(self.owner.pid, signal.SIGTERM)
                    self.owner.wait(timeout=5)
                raise TimeoutError("Existing ARM workload guardian lease unavailable")
        return self

    def complete(self, report):
        self.evidence = cleanup_evidence(report)

    def complete_storage_failure(self, fallback):
        if fallback.get("format") != "connect/original-component-storage-failure/1":
            raise ValueError("Original component storage failure required")
        self.evidence = dict(fallback["cleanup_evidence"], storage_failure=dict(
            source_revision=fallback["source_revision"],cell_id=fallback["cell_id"],
            original_full_report_sha256=fallback["original_full_report_sha256"],full_report_saved=False))

    def complete_unentered(self):
        self.evidence = dict(confirmed=True, component_owner_started=False,
                             reason="storage-preflight-refused-before-original-component-entry")

    def __exit__(self, *_):
        self.owner.stdin.write((json.dumps(self.evidence) + "\n").encode())
        self.owner.stdin.close()
        with selectors.DefaultSelector() as selector:
            selector.register(self.owner.stdout, selectors.EVENT_READ)
            self.blocked = not selector.select(timeout=5) or self.owner.stdout.readline() != b"released\n"
        if not self.blocked:
            self.owner.wait(timeout=5)
            self.owner.stdout.close()


def guard():
    with VmWorkloadLease() as lease:
        reserve = MetadataReserve(lease.directory / ("guardian-" + str(os.getpid()) + ".reserve"),
                                  GUARD_RESERVE_BYTES)
        print("acquired", flush=True)
        try:
            raw = sys.stdin.buffer.read(16385)
            if len(raw) > 16384:
                raise ValueError("Bounded original cleanup evidence required")
            evidence = json.loads(raw)
            if not isinstance(evidence, dict) or type(evidence.get("confirmed")) is not bool:
                raise ValueError("Original typed cleanup evidence required")
        except (OSError, ValueError):
            evidence = dict(confirmed=False, reason="original-worker-loss")
        path = lease.directory / ("guardian-" + str(os.getpid()) + ".json")
        value = dict(guardian_pid=os.getpid(), cleanup=evidence,
            state="released" if evidence.get("confirmed") is True else "blocked")
        durable = False
        try:
            atomic_json(path, value, reserve=reserve, limit=32768)
            durable = True
        except Exception as error:
            value["vm_metadata_failure"] = type(error).__name__
            try:
                acknowledgement = metadata_failure(value)
                durable = acknowledgement.get("host_durable") is True
            except Exception as fallback_error:
                value["host_metadata_failure"] = type(fallback_error).__name__
        try:
            reserve.release()
        except Exception as error:
            value["reserve_cleanup_failure"] = type(error).__name__
        if evidence.get("confirmed") is not True or not durable:
            try:
                print("blocked", flush=True)
            except (OSError, ValueError):
                # Lost original worker also closes this acknowledgement pipe.
                # That cannot release custody of its unknown original owners.
                pass
            # This named guardian intentionally retains custody. An external
            # controller must establish actual owner absence before retiring it.
            while True:
                time.sleep(30)
        print("released", flush=True)


if __name__ == "__main__":
    if sys.argv[1:] != ["--guard"]:
        raise SystemExit("Original guard mode required")
    guard()
