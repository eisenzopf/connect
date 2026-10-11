"""Prepare exact archived-source fault fixtures before bounded runtime checks."""
import hashlib
import json
import shutil

from .qualification_process import run_phase
from .qualification_report import environment


def prepare(source, directory, manifest):
    """Compilation has its own budget; installed behavior retains its 600s limit."""
    directory.mkdir(parents=True, mode=0o700, exist_ok=False)
    report = {"status": "running", "source_revision": manifest["source_revision"],
              "scope": "extracted candidate source; test-only binaries, never runtime assets"}
    result = directory / "result.json"
    env = environment()
    env.update(CARGO_TARGET_DIR=str(source / "target/vapi-fixture"), CARGO_BUILD_JOBS="3")
    command = ["cargo", "build", "--locked", "-p", "connect-server", "-p", "connect-review-loop",
               "--features", "connect-server/test-support,connect-review-loop/test-support",
               "--bin", "connect", "--bin", "connect-example"]
    try:
        lock = hashlib.sha256((source / "Cargo.lock").read_bytes()).hexdigest()
        if lock != manifest["cargo_lock_sha256"]:
            raise ValueError("Qualification fixture lock differs from the candidate")
        phase = run_phase("build", command, 2400, directory, source, env)
        report["phase"] = phase
        if phase["status"] != "passed":
            raise RuntimeError("Qualification fixture compilation failed")
        binaries = directory / "bin"
        binaries.mkdir(mode=0o700)
        hashes = {}
        for name, marker in (("connect", b"CONNECT_TEST_FAULT_STAGE"),
                             ("connect-example", b"CONNECT_EXAMPLE_FAULT_STAGE")):
            path = binaries / name
            shutil.copy2(source / "target/vapi-fixture/debug" / name, path)
            raw = path.read_bytes()
            if marker not in raw:
                raise ValueError("Prepared binary lacks its required test-only control")
            hashes[name] = hashlib.sha256(raw).hexdigest()
        report.update(status="passed", binaries_sha256=hashes)
    except BaseException as error:
        report.update(status="failed", error=type(error).__name__, detail=str(error))
        raise
    finally:
        result.write_text(json.dumps(report, indent=2) + "\n")
    return binaries, report
