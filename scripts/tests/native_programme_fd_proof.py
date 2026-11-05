"""Bounded before/after component, each on its own unchanged original Node PID.

No services, media or matrix qualification. Run only in the approved local ARM
CPU window; Mac Node26 and Linux Node22 use the same actual source witness.
"""
import hashlib
import json
import os
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.matrix_macos_resources import MacProcessResources
from lib.operational_process_resources import ProcessResources, stat_fields
from lib.native_matrix_process import NativeOwner


def main(output):
    output = Path(output); output.mkdir(mode=0o700, parents=True, exist_ok=False)
    results = []
    for mode in ("before", "after"):
        directory = output / mode; directory.mkdir(mode=0o700)
        log_path = directory / "original.log"
        owner = NativeOwner(["node", str(ROOT / "scripts/tests/fixtures/native_programme_fd_witness.mjs"), mode, str(directory)],
            directory / "owner", log_path, cwd=ROOT, env=os.environ.copy())
        observations, identity, sampler, failure = {}, None, None, None
        try:
            until = time.monotonic() + 15
            while owner.poll() is None:
                if owner.identity() is not None and sampler is None:
                    collector = MacProcessResources if sys.platform == "darwin" else ProcessResources
                    sampler = collector({"helper": owner.pid}, roles=("helper",), open_resources=True, max_samples=4)
                for phase in ("baseline", "final"):
                    path = directory / (phase + ".json")
                    if phase in observations or not path.exists() or sampler is None:
                        continue
                    assert path.stat().st_size <= 128
                    value = json.loads(path.read_text()); assert value["phase"] == phase
                    pid = value["pid"]; assert type(pid) is int and pid > 1
                    if sys.platform == "darwin":
                        current = sampler.identity(pid)
                        key = (pid, current["start"])
                        assert current["parent"] == owner.pid
                        raw = sampler.read(pid, 1, 4097 * 8); assert len(raw) % 8 == 0 and len(raw) < 4097 * 8
                        descriptors = len(raw) // 8
                    else:
                        current = stat_fields(Path(f"/proc/{pid}/stat").read_text())
                        key = (pid, current["start_ticks"]); assert current["parent"] == owner.pid
                        with os.scandir(f"/proc/{pid}/fd") as entries:
                            descriptors = sum(1 for _ in entries)
                        assert stat_fields(Path(f"/proc/{pid}/stat").read_text())["start_ticks"] == key[1]
                    if identity is None:
                        identity = key
                    assert identity == key, "Original Node identity changed"
                    observations[phase] = dict(node_descriptors=descriptors, original_node_pid=pid, group=sampler.sample())
                    (directory / (phase + "-observed.json")).write_text("{}\n")
                if time.monotonic() >= until or not owner.alive():
                    raise TimeoutError("Original framework component deadline")
                time.sleep(.01)
            assert owner.code == 0
        except BaseException as error:
            failure = type(error).__name__
        finally:
            cleanup = owner.close()  # Original unreaped supervisor owns every cleanup/group decision.
        raw = log_path.read_bytes(); assert len(raw) <= 256 * 1024
        rows = [json.loads(line) for line in raw.splitlines() if line.startswith(b'{')]
        result = dict(mode=mode, original_node_pid=identity[0] if identity else None, cleanup=cleanup,
            failure=failure, observations=observations, original_log_sha256=hashlib.sha256(raw).hexdigest(),
            original_result=rows[-1] if rows else None, passed=False)
        if failure is None:
            before, after = (observations[name]["group"]["groups"]["helper"] for name in ("baseline", "final"))
            assert before["root_state"] == after["root_state"] == "available"
            assert before["live_processes"] == after["live_processes"] == 2
            delta = observations["final"]["node_descriptors"] - observations["baseline"]["node_descriptors"]
            result["descriptor_delta"] = delta
            result["passed"] = ((delta > 0) if mode == "before" else (delta <= 0)) and rows[-1]["child"]["child_joined"]
            result["passed"] &= rows[-1]["pid"] == identity[0] and rows[-1]["architecture"] == "arm64"
            result["passed"] &= rows[-1]["node"].startswith("v26.") if sys.platform == "darwin" else rows[-1]["node"].startswith("v22.")
            result["passed"] &= cleanup["original_supervisor_joined"] and cleanup["original_workers_joined"] and not cleanup["forced"]
        results.append(result)
    report = dict(format="connect/native-programme-worker-fd-component/1", passed=all(v["passed"] for v in results),
        results=results, scope="Actual original-process descriptor and natural Worker/child ownership component; no RSS, idle or matrix pass.")
    (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1]))
