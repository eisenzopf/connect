#!/usr/bin/env python3
"""Explicit fresh local native fixture; no accounts, model, telephone or provider."""
import argparse
import hashlib
import json
import os
import pathlib
import socket
import signal
import subprocess
import time
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[3]


def sha(path):
    with pathlib.Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def private(path, value):
    with pathlib.Path(path).open("x", encoding="utf8") as stream:
        os.chmod(path, 0o600)
        stream.write(value)
        stream.flush()
        os.fsync(stream.fileno())


def group_states(pid):
    result = subprocess.check_output(["ps", "-axo", "pid=,pgid=,state="], timeout=5, text=True)
    return [(int(parts[0]), parts[2]) for line in result.splitlines()
            if len(parts := line.split()) == 3 and int(parts[1]) == pid]


def owned_worker(command, log, *, timeout_seconds=90, environment=None, cwd=None, stderr=None, input_bytes=None):
    """Keep leader unreaped until group cleanup; never signal a reused PGID."""
    assert 1 <= timeout_seconds <= 120
    child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT if stderr is None else stderr,
                             start_new_session=True, env=environment, cwd=cwd,
                             stdin=subprocess.DEVNULL if input_bytes is None else subprocess.PIPE)
    pending = memoryview(input_bytes or b"")
    if input_bytes is not None:
        os.set_blocking(child.stdin.fileno(), False)
    forced, deadline = False, time.monotonic() + timeout_seconds
    try:
        while not os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT):
            if time.monotonic() >= deadline or os.fstat(log.fileno()).st_size > 4 * 1024 * 1024:
                forced = True
                break
            if child.stdin is not None and not child.stdin.closed:
                if not pending:
                    child.stdin.close()
                else:
                    try:
                        pending = pending[os.write(child.stdin.fileno(), pending[:4096]):]
                    except BlockingIOError:
                        pass
            time.sleep(0.025)
        forced |= bool(pending)
        states = group_states(child.pid)
        forced |= any(not state.startswith("Z") for _, state in states)
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            # Darwin skips zombies. Require an actual snapshot of this still
            # unreaped leader and no live group member, as the Rust host does.
            states = group_states(child.pid)
            assert any(pid == child.pid for pid, _ in states)
            assert all(state.startswith("Z") for _, state in states)
        return child.wait(timeout=5), forced
    finally:
        if child.stdin is not None and not child.stdin.closed:
            child.stdin.close()
        # Only the unreaped exception branch may signal the group here.
        if child.returncode is None:
            # An exception before wait() still leaves our leader unreaped, so
            # its group identity remains reserved for this failure cleanup.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
            child.kill()
            child.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=pathlib.Path)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--model-config", type=pathlib.Path,
                        help="Separately reviewed G6-073 model configuration; omitted means simulated model")
    args = parser.parse_args()
    binary, output = args.executable.resolve(), args.output.absolute()
    assert binary.is_file() and os.access(binary, os.X_OK)
    assert sha(binary) == args.expected_sha256, "Select a source-bound executable"
    if args.model_config:
        model_config = json.loads(args.model_config.read_text())
        assert model_config.get("task") == "G6-073" and model_config.get("approved_model_execution") is True
    assert output.is_absolute() and ".local" in output.resolve().parts
    assert output.parent.is_dir() and not output.exists()
    output.mkdir(mode=0o700)
    names = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT).decode().split("\0")
    names = sorted({name for name in names if name and (ROOT / name).is_file() and
                    (name.startswith(("crates/", "sdk/", "contracts/", "scripts/", "dependencies/", "patches/", "examples/")) or
                     name in ["Cargo.toml", "Cargo.lock", "AGENTS.md"])})
    names += [name for name in [".deps/rvoip/.git/connect-files.json", ".deps/rvoip/.git/connect-prepared.json"] if (ROOT / name).is_file()]
    snapshot = lambda: {name: sha(ROOT / name) for name in names}
    before = snapshot()
    private(output / "source-inputs.json", json.dumps(before, indent=2) + "\n")
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    installation = output / "installation"
    with (output / "init.log").open("xb") as log:
        os.chmod(log.name, 0o600)
        subprocess.run([str(binary), "init", "--directory", str(installation), "--example", "review-loop",
                        "--bind", f"127.0.0.1:{port}"], stdout=log, stderr=subprocess.STDOUT, check=True, timeout=30)
    identities = {}
    for actor in ["agent", "reviews"]:
        original = (installation / f"{actor}.toml").read_text()
        identities[actor] = tomllib.loads(original)["identity_id"]
        private(installation / f"{actor}.sdk.toml", 'tenant_id = "tnt_default"\n[control]\n' + original)
    wrapper = output / "owned-helper"
    pid_file = output / "helper-pids"
    private(wrapper, "#!/usr/bin/env python3\nimport os,sys\nos.umask(0o077)\nwith open(" + repr(str(pid_file)) + ", 'a') as stream: stream.write(str(os.getpid())+'\\n')\nos.execv(" + repr(str(binary)) + ", [" + repr(str(binary)) + ", *sys.argv[1:]])\n")
    os.chmod(wrapper, 0o700)
    private(output / "input.json", json.dumps({"profile": "parley-worker-native-rehearsal/1",
            "agentConfig": str(installation / "agent.sdk.toml"), "ownerConfig": str(installation / "reviews.sdk.toml"),
            "agentIdentity": identities["agent"], "ownerIdentity": identities["reviews"], "executable": str(wrapper),
            "workerDirectory": str(output / "worker"), "modelDirectory": str(output / "model"),
            "modelConfig": str(args.model_config.resolve()) if args.model_config else None}, indent=2) + "\n")
    started = time.monotonic()
    server_log = (output / "server.log").open("xb")
    os.chmod(server_log.name, 0o600)
    server = subprocess.Popen([str(binary), "serve", "--config", str(installation / "server.toml")],
                              stdout=server_log, stderr=subprocess.STDOUT)
    runtime_code, cleanup_forced, worker_forced, error = None, False, False, None
    try:
        deadline = time.monotonic() + 30
        while b"ready transport=quic" not in (output / "server.log").read_bytes():
            assert server.poll() is None and time.monotonic() < deadline, "original server did not become ready"
            time.sleep(0.025)
        with (output / "runtime.log").open("xb") as log:
            os.chmod(log.name, 0o600)
            runtime_code, worker_forced = owned_worker(["node", str(pathlib.Path(__file__).with_suffix(".mjs")), str(output / "input.json")], log)
    except Exception as failure:
        error = type(failure).__name__ + ": " + str(failure)
    finally:
        if server.poll() is None:
            server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            cleanup_forced = True
            server.kill()
            server.wait(timeout=5)
        server_log.close()
    helpers_joined = True
    for pid in pid_file.read_text().splitlines() if pid_file.exists() else []:
        try:
            os.kill(int(pid), 0)
            helpers_joined = False
        except ProcessLookupError:
            pass
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as check:
        check.bind(("127.0.0.1", port))
        listener_reclaimed = True
    after = snapshot()
    report = {"profile": "parley-worker-native-summary/1", "runtime_exit": runtime_code,
              "server_exit": server.returncode, "forced_server_cleanup": cleanup_forced,
              "forced_worker_group_cleanup": worker_forced,
              "helpers_joined": helpers_joined, "listener_reclaimed": listener_reclaimed,
              "seconds": time.monotonic() - started, "source_count": len(before),
              "changed_inputs": [key for key in before if before[key] != after[key]],
              "binary_sha256": sha(binary), "binary_unchanged": sha(binary) == args.expected_sha256,
              "error": error, "actual_model_requested": args.model_config is not None,
              "scope": "Local signed EdDSA native messages/events, app approval and original operation recovery; model result requires its separately retained host outcome. No media qualification."}
    private(output / "summary.json", json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    assert runtime_code == 0 and server.returncode == 0 and not cleanup_forced and not worker_forced and helpers_joined
    assert listener_reclaimed and before == after and report["binary_unchanged"] and error is None


if __name__ == "__main__":
    main()
