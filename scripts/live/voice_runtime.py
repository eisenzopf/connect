"""Select verified installed SDK/media assets and save private rehearsal intent."""
import hashlib
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace


def save(path, value):
    """Publish one complete private record before any corresponding side effect."""
    temporary = path.with_name(path.name + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def load(source_root, runtime_root, executable):
    """Installed mode cannot silently import checkout SDKs or select another binary."""
    root = (runtime_root or source_root).resolve()
    if runtime_root:
        manifest = json.loads((root / "manifest.json").read_text())
        names = [name for name in manifest["files"] if name.startswith("sdk/python/connect_sdk/")]
        names += ["bin/connect", "examples/conversation-client/pcmu.py"]
        if not names or "sdk/python/connect_sdk/__init__.py" not in names:
            raise ValueError("Installed Python SDK is missing")
        for name in names:
            path = (root / name).resolve()
            if (not path.is_relative_to(root) or not path.is_file()
                    or hashlib.sha256(path.read_bytes()).hexdigest() != manifest["files"].get(name)):
                raise ValueError("Installed rehearsal asset differs from its manifest")
        binary = root / "bin/connect"
        if executable and Path(executable).resolve() != binary.resolve():
            raise ValueError("Installed rehearsal must use the selected package binary")
        evidence = {"source_revision": manifest["source_revision"], "candidate": manifest["candidate"],
                    "binary_sha256": manifest["files"]["bin/connect"], "runtime": "installed"}
    else:
        binary = Path(executable) if executable else root / "target/debug/connect"
        evidence = {"source_revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
            "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root)),
            "runtime": "source"}
    evidence["driver_files_sha256"] = {
        name: hashlib.sha256((source_root / "scripts/live" / name).read_bytes()).hexdigest()
        for name in ("native-voice.py", "task_voice.py", "voice_runtime.py", "voice_scenario.py")
    }
    sys.path[:0] = [str(root / "sdk/python"), str(root / "examples/conversation-client")]
    sdk = importlib.import_module("connect_sdk")
    pcmu = importlib.import_module("pcmu")
    for name, module in tuple(sys.modules.items()):
        if name == "pcmu" or name == "connect_sdk" or name.startswith("connect_sdk."):
            if not Path(module.__file__).resolve().is_relative_to(root):
                raise ValueError("Rehearsal imported an asset from another installation")
    return SimpleNamespace(client=sdk.NativeClient, audio=sdk.NativeAudioClient,
                           encode=pcmu.encode, decode=pcmu.decode, binary=str(binary), evidence=evidence)
