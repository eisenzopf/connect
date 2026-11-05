"""Private, bounded job owner inside the existing connect-gate6 ARM guest.

The Mac stages this source with its client SDK. No daemon, VM provisioning or
public endpoint is created; the original SSH supervisor owns the one job.
"""
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import signal
import subprocess
import sys
import tarfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from operational_process_resources import ProcessResources, stat_fields

LIMIT = 512 * 1024 * 1024


def atomic(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value) + "\n")
    temporary.replace(path)


def group_alive(group, started=None):
    if started is not None:
        try:
            if stat_fields(Path(f"/proc/{group}/stat").read_text())["start_ticks"] != started:
                return False
        except FileNotFoundError:
            pass
    for path in Path("/proc").glob("[0-9]*/stat"):
        try:
            value = stat_fields(path.read_text())
            if value["group"] == group and value["state"] not in {"Z", "X"}:
                return True
        except FileNotFoundError:
            pass
    return False


def container_present(name):
    result = subprocess.run(["docker", "ps", "--all", "--no-trunc", "--filter", "name=^/" + name + "$", "--format", "{{.ID}}"],
        capture_output=True, timeout=5, check=True)
    return bool(result.stdout.strip())


def lease_owners_joined(root):
    """Elapsed time never releases a lease for an unconfirmed original owner."""
    try:
        children = root / "lease-children.json"
        registered = json.loads(children.read_text()) if children.exists() else []
        if not isinstance(registered, list) or len(registered) > 8:
            return False
        roots = [root, *(checked_root(v) for v in registered)]
        for scope in roots:
            for intent in (scope / ".owners").glob("*-intent.json"):
                path = intent.with_name(intent.name.replace("-intent.json", "-receipt.json"))
                if not path.exists():
                    return False
                receipt = json.loads(path.read_text())
                if not receipt.get("original_joined") or group_alive(receipt["original_pid"], receipt.get("original_start_ticks")):
                    return False
            if (scope / "job-intent.json").exists():
                path = scope / "job-state.json"
                if not path.exists():
                    return False
                state = json.loads(path.read_text())
                if not state.get("original_joined"):
                    return False
                if state.get("original_pid") and group_alive(state["original_pid"], state.get("original_start_ticks")):
                    return False
                config = json.loads((scope / "job.json").read_text())
                if config.get("container") and container_present(config["container"]):
                    return False
        return True
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        return False


def owned_command(root, identity):
    intent = json.loads((root / ".owners" / (identity + "-intent.json")).read_text())
    assert intent["run_id"] == root.name and intent["owner_id"] == identity and 1 <= intent["timeout"] <= 7200
    child = None
    forced = False
    def stop(*_):
        nonlocal forced
        forced = True
        if child is not None and child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
    for signum in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(signum, stop)
    code = None
    try:
        child = subprocess.Popen(intent["command"], cwd=root, stdin=None if intent.get("pass_stdin") else subprocess.DEVNULL, start_new_session=True)
        started = stat_fields(Path(f"/proc/{child.pid}/stat").read_text())["start_ticks"]
        code = child.wait(timeout=intent["timeout"])
    finally:
        if child is not None:
            if child.poll() is None or group_alive(child.pid):
                forced = True
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            code = child.wait(timeout=5)
            until = time.monotonic() + 5
            while group_alive(child.pid) and time.monotonic() < until:
                time.sleep(.02)
            atomic(root / ".owners" / (identity + "-receipt.json"), dict(run_id=root.name, owner_id=identity,
                original_pid=child.pid, original_start_ticks=started, original_joined=not group_alive(child.pid, started), forced=forced, exit_code=code))
    return code if code is not None and not forced else 1


def checked_root(value):
    path = Path(value)
    if (not path.is_absolute() or not str(path).startswith("/workspace/.local/mac-remote-")
            or path.is_symlink() or ".." in path.parts):
        raise ValueError("Original private guest root required")
    return path


def regular_files(directory):
    result = []
    for path in sorted(directory.rglob("*")):
        if path.relative_to(directory).parts[0] in {"trusted-profile", "untrusted-profile"}:
            continue  # Ephemeral browser profiles are not evidence artifacts.
        if path.is_symlink():
            raise ValueError("No private symlink transfer")
        if path.is_file():
            result.append(path)
    if len(result) > 10000 or sum(path.stat().st_size for path in result) > LIMIT:
        raise ValueError("Private transfer bound")
    return result


def receive(root):
    raw = sys.stdin.buffer.read(LIMIT + 1)
    if len(raw) > LIMIT:
        raise ValueError("Private input bound")
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
        members = archive.getmembers()
        if len(members) > 10000 or sum(m.size for m in members) > LIMIT:
            raise ValueError("Private archive bound")
        names = set()
        for member in members:
            relative = PurePosixPath(member.name)
            if (relative.is_absolute() or ".." in relative.parts or not member.isfile()
                    or member.name in names or "\\" in member.name):
                raise ValueError("Ordinary relative private files required")
            names.add(member.name)
            target = root / str(relative)
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            if any(parent.is_symlink() for parent in (target, *target.parents)):
                raise ValueError("Private destination symlink")
            body = archive.extractfile(member).read()
            temporary = target.with_suffix(target.suffix + ".transfer")
            temporary.write_bytes(body)
            temporary.chmod(0o700 if member.mode & 0o100 else 0o600)
            temporary.replace(target)
    print(json.dumps({"files": len(names)}))


def collect(root, relative):
    directory = root / relative
    if not directory.resolve().is_relative_to(root.resolve()):
        raise ValueError("Original output directory required")
    paths = regular_files(directory)
    manifest = {str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}
    raw = (json.dumps(manifest, sort_keys=True) + "\n").encode()
    with tarfile.open(fileobj=sys.stdout.buffer, mode="w|") as archive:
        item = tarfile.TarInfo("TRANSFER-MANIFEST.json")
        item.size, item.mode = len(raw), 0o600
        archive.addfile(item, io.BytesIO(raw))
        for path in paths:
            archive.add(path, arcname=str(path.relative_to(directory)), recursive=False)


def exchange(root):
    request = json.load(sys.stdin)
    output = root / request["directory"]
    if not output.resolve().is_relative_to(root.resolve()):
        raise ValueError("Original output bridge required")
    allowed_responses = {f"authority-{i}-response.json" for i in (1, 2, 3)} | {
        f"resources-{phase}-response.json" for phase in ("baseline", "terminal", "retired")} | {"revoke-response.json"}
    for name, value in request.get("responses", {}).items():
        if name not in allowed_responses or len(json.dumps(value).encode()) > 4096:
            raise ValueError("Bounded original response required")
        target = output / name
        if target.exists() and json.loads(target.read_text()) != value:
            raise ValueError("Original response cannot be changed")
        if not target.exists():
            atomic(target, value)
    values = {}
    for name in [*(f"authority-{i}-request.json" for i in (1, 2, 3)),
                 *(f"resources-{phase}.json" for phase in ("baseline", "terminal", "retired")), "revoke-request.json"]:
        path = output / name
        if path.exists():
            if path.is_symlink() or path.stat().st_size > 4096:
                raise ValueError("Bounded original request required")
            values[name] = json.loads(path.read_text())
    state_path = root / "job-state.json"
    state = json.loads(state_path.read_text()) if state_path.exists() else None
    print(json.dumps(dict(monotonic_seconds=time.monotonic(), requests=values, state=state)))


def run(root):
    config = json.loads((root / "job.json").read_text())
    assert platform.system() == "Linux" and platform.machine() == "aarch64" and os.getuid() != 0
    assert config["run_id"] == root.name
    release = platform.freedesktop_os_release()
    assert release["ID"] == "ubuntu" and release["VERSION_ID"] == "24.04"
    assert 1 <= config["timeout_seconds"] <= 360
    stopped = False
    def stop(_signum, _frame):
        nonlocal stopped
        stopped = True
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(signum, stop)
    output = root / config["output"]
    output.mkdir(mode=0o700, parents=True, exist_ok=True)
    started = time.monotonic()
    owned = None
    sampler = None
    code = None
    forced = False
    state = dict(format="connect/mac-linux-original-job/1", run_id=config["run_id"],
        client_platform="ubuntu24-arm64", architecture=platform.machine(), ordinary_user=os.getuid(),
        source_revision=config["source_revision"], original_joined=False, forced=False,
        supervisor_pid=os.getpid(), supervisor_start_ticks=stat_fields(Path("/proc/self/stat").read_text())["start_ticks"])
    atomic(root / "job-state.json", state)
    capture = None
    with (output / "guest-job.log").open("xb") as log:
        try:
            if config.get("capture"):
                observed_capture = dict(config["capture"])
                kind = observed_capture.pop("kind", "native")
                if kind == "browser":
                    from lib.matrix_browser_capture import BrowserCapture
                    observed_capture.pop("client_ip", None)
                    capture = BrowserCapture(output / "packet-capture", env=os.environ.copy(), **observed_capture)
                    try:
                        capture.start()
                    except (OSError, RuntimeError):
                        capture.finish()
                else:
                    from transport_capture import NativeCapture
                    capture = NativeCapture(output / "packet-capture", env=os.environ.copy(),
                        media_workload=True, mac_linux_guest=True, **observed_capture)
                    capture.start_observed()
            owned = subprocess.Popen(config["command"], cwd=root, env=os.environ.copy(),
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            state["original_pid"] = owned.pid
            state["original_start_ticks"] = stat_fields(Path(f"/proc/{owned.pid}/stat").read_text())["start_ticks"]
            atomic(root / "job-state.json", state)
            while owned.poll() is None:
                if stopped or time.monotonic() - started > config["timeout_seconds"]:
                    forced = True
                    break
                if sampler is None:
                    pid = owned.pid
                    if config.get("container"):
                        inspected = subprocess.run(["docker", "inspect", config["container"]], capture_output=True, timeout=3)
                        if inspected.returncode == 0:
                            container = json.loads(inspected.stdout)[0]
                            if container["State"]["Running"]:
                                assert container["Config"]["Labels"]["connect.gate6.owner"] == config["run_id"]
                                pid = container["State"]["Pid"]
                            else:
                                pid = None
                        else:
                            pid = None
                    if pid is not None:
                        role = config.get("role", "browser")
                        assert role in {"browser", "helper"}
                        sampler = ProcessResources({role: pid}, roles=(role,),
                            open_resources=True, max_samples=600)
                        state["sampled_root"] = pid
                if sampler is not None:
                    state["resources"] = sampler.sample()
                    state["sampled_at_monotonic_seconds"] = time.monotonic()
                state["monotonic_seconds"] = time.monotonic()
                atomic(root / "job-state.json", state)
                time.sleep(.5)
            if forced:
                if config.get("container"):
                    subprocess.run(["docker", "stop", "--time", "5", config["container"]],
                        stdout=log, stderr=log, timeout=10, check=False)
                if owned.poll() is None:
                    os.killpg(owned.pid, signal.SIGTERM)
            code = owned.wait(timeout=10)
        finally:
            if owned is not None and owned.poll() is None:
                forced = True
                os.killpg(owned.pid, signal.SIGKILL)
                code = owned.wait(timeout=5)
            if owned is not None and not config.get("container"):
                try:
                    os.killpg(owned.pid, 0)
                except ProcessLookupError:
                    pass
                else:
                    forced = True
                    os.killpg(owned.pid, signal.SIGKILL)
            if config.get("container"):
                if container_present(config["container"]):
                    inspection = subprocess.run(["docker", "inspect", config["container"]], capture_output=True, timeout=3, check=True)
                    observed = json.loads(inspection.stdout)[0]
                    assert observed["Config"]["Labels"]["connect.gate6.owner"] == config["run_id"]
                    subprocess.run(["docker", "rm", "-f", config["container"]], stdout=log, stderr=log, timeout=10, check=True)
                    forced = True
            state.update(exit_code=code, forced=forced, original_joined=owned is not None and owned.poll() is not None and not group_alive(owned.pid)
                         and (not config.get("container") or not container_present(config["container"])),
                         monotonic_seconds=time.monotonic())
            if sampler is not None:
                atomic(output / "guest-process-observation.json", sampler.summary())
            if capture is not None:
                captured = capture.finish()
                captured["capture_location"] = dict(platform="ubuntu24-arm64", interface=config["capture"]["interface"],
                    actual_client_ip=config["capture"].get("peer_ip", config["capture"].get("client_ip")), actual_server_ip=config["capture"]["server_ip"])
                atomic(output / ("browser-capture-observation.json" if config["capture"].get("kind") == "browser" else "native-capture-observation.json"), captured)
                state["capture_owner_created"] = capture.process is not None
                state["capture_original_joined"] = capture.process is None or captured.get("original_joined") is True
                state["original_joined"] &= state["capture_original_joined"]
            atomic(root / "job-state.json", state)
            atomic(output / "guest-owner.json", state)
    return code if code is not None and not forced else 1


def main():
    os.umask(0o077)
    action, root = sys.argv[1], checked_root(sys.argv[2])
    if action == "receive": receive(root)
    elif action == "collect": collect(root, sys.argv[3])
    elif action == "exchange": exchange(root)
    elif action == "run": return run(root)
    elif action == "owned-command": return owned_command(root, sys.argv[3])
    elif action == "stop":
        state = json.loads((root / "job-state.json").read_text())
        pid = state["supervisor_pid"]
        if state["run_id"] != root.name or Path(f"/proc/{pid}").stat().st_uid != os.getuid():
            raise ValueError("Original guest supervisor required")
        if stat_fields(Path(f"/proc/{pid}/stat").read_text())["start_ticks"] != state["supervisor_start_ticks"]:
            raise ValueError("Original guest supervisor replaced")
        os.kill(pid, signal.SIGTERM)
    else: raise ValueError("Unknown original guest action")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
