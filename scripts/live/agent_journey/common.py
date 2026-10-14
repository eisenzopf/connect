"""Bounded private evidence and installed-runtime verification for live agent checks."""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess

ROOT = Path(__file__).resolve().parents[3]
TURN_SECONDS = 300
OUTER_SECONDS = 1100
MAX_LOG = 4 * 1024 * 1024

class ProofFailure(Exception):
    """A fixed non-secret assertion label; never an arbitrary provider error."""


def require(condition, label):
    if not condition:
        raise ProofFailure(label)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def read_json(path, maximum=MAX_LOG):
    require(path.is_file() and path.stat().st_size <= maximum, "bounded-json-file")
    return json.loads(path.read_text())


def save(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")


def write_private(path, content):
    with path.open("x") as stream:
        stream.write(content)


def selected_environment():
    common = {
        name: os.environ[name]
        for name in ("PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "LANG")
        if name in os.environ
    }
    common["PYTHONDONTWRITEBYTECODE"] = "1"
    model = dict(common)
    for name in ("CODEX_HOME", "CODEX_API_KEY"):
        if name in os.environ:
            model[name] = os.environ[name]
    return common, model


def port(allocated):
    """Reserve a candidate unused by both TCP and UDP, avoiding duplicates."""
    for _ in range(30):
        with socket.socket() as tcp:
            tcp.bind(("127.0.0.1", 0))
            value = tcp.getsockname()[1]
            if value in allocated:
                continue
            with socket.socket(type=socket.SOCK_DGRAM) as udp:
                try:
                    udp.bind(("127.0.0.1", value))
                except OSError:
                    continue
                allocated.append(value)
                return value
    raise ProofFailure("unused-local-port")


def verify_runtime(runtime, qualification, expected_profile="gate4-local"):
    manifest = read_json(runtime / "manifest.json", 8 * MAX_LOG)
    report = read_json(qualification, 8 * MAX_LOG)
    require(report.get("result") == "passed", "passed-package-required")
    require(report.get("manifest") == manifest, "exact-package-manifest-required")
    require(manifest.get("target") == "aarch64-apple-darwin", "local-arm64-target")
    require(expected_profile in ("gate4-local", "gate5-local"), "known-runtime-profile")
    require(manifest.get("qualification_profile") == expected_profile, "matching-runtime-profile")
    require(manifest.get("source_revision"), "recorded-source-revision")
    files = manifest.get("files", {})
    require("docs/AGENT_ACCESS.md" in files, "new-regular-agent-package-required")
    for name, expected in files.items():
        relative = Path(name)
        require(not relative.is_absolute() and ".." not in relative.parts, "safe-manifest-path")
        path = runtime / relative
        require(path.is_file() and not path.is_symlink(), "ordinary-runtime-file")
        require(sha256(path) == expected, "runtime-file-hash")
    actual = {str(path.relative_to(runtime)) for path in runtime.rglob("*") if path.is_file()}
    require(actual == set(files) | {"manifest.json"}, "exact-runtime-inventory")
    for name in (
        "bin/connect", "sdk/python/connect_sdk/native.py",
        "skills/connect/SKILL.md", "skills/connect/references/json-cli.md",
    ):
        require(name in files, "installed-interface-asset")
    require(os.access(runtime / "bin/connect", os.X_OK), "installed-executable")
    return manifest


def ps_metadata(pid):
    result = subprocess.run(
        ["/bin/ps", "-o", "pid=,ppid=,pgid=,comm=", "-p", str(pid)],
        capture_output=True, text=True, timeout=3,
    )
    if result.returncode != 0 or not result.stdout.strip():
        return None
    fields = result.stdout.strip().split(maxsplit=3)
    require(len(fields) == 4, "owned-process-metadata")
    return {"pid": int(fields[0]), "ppid": int(fields[1]),
            "pgid": int(fields[2]), "executable": str(Path(fields[3]).resolve())}


