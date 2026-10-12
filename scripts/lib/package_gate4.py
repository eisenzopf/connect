"""Gate 4 package assets and real installed-runtime checks from extracted source."""
import json
import subprocess
import sys

from .qualification_process import run_phase
from .qualification_report import check_test_results, environment

ASSETS = (
    "contracts/connect-task-v1.md", "contracts/connect-task-access-v1.md",
    "skills/connect/SKILL.md", "skills/connect/agents/openai.yaml",
    "skills/connect/references/json-cli.md", "docs/TASK_ACCESS.md",
    "docs/CODEX_TASK_HOST.md", "docs/TASK_RECOVERY.md", "docs/VAPI_TASK_HOST.md",
    "sdk/python/connect_sdk/task_operations.py", "sdk/typescript/dist/tasks.js",
    "examples/agent-presence/codex-inbox.py", "examples/agent-presence/incoming/host.py",
    "examples/agent-presence/incoming/intake.py", "examples/agent-presence/incoming/state.py",
)


def inventory(release):
    for name in ASSETS:
        if not (release / name).is_file():
            raise ValueError(f"Missing Gate 4 runtime asset: {name}")


def qualify(release, source, codex, directory):
    """Fixtures come from source; application, SDK and helper must come from release."""
    inventory(release)
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    env = environment(codex)
    env.update(CONNECT_BINARY=str(release / "bin/connect"),
               CONNECT_TEST_BINARY=str(release / "bin/connect"),
               CONNECT_TEST_RUNTIME_ROOT=str(release),
               CONNECT_TEST_EVIDENCE_DIR=str(directory / "host-state"),
               PYTHONPATH=str(release / "sdk/python"))
    report = {"status": "running", "scope": "installed application/SDK/helper, scripted local Responses; no inference or live providers",
              "runtime_root": str(release), "fixture_root": str(source), "steps": []}
    result_path = directory / "result.json"
    def save():
        result_path.write_text(json.dumps(report, indent=2) + "\n")
    save()
    try:
        report["codex_version"] = subprocess.check_output([str(codex), "--version"], env=env, text=True, timeout=15).strip()
        for name, pattern, timeout in (("agent-incoming-public", "test_public_intake.py", 120),
                                       ("agent-incoming-host", "test_installed_incoming.py", 240)):
            command = [sys.executable, "-m", "unittest", "discover", "-s",
                       str(source / "examples/agent-presence/tests"), "-p", pattern]
            result = run_phase(name, command, timeout, directory, release, env)
            report["steps"].append(result)
            try:
                check_test_results(result, directory)
            except (ValueError, OSError) as error:
                result.update(status="failed", verification_error=str(error))
                raise
            save()
            if result["status"] != "passed":
                raise RuntimeError(f"Installed Gate 4 phase failed: {name}")
        # The fixture seed is created through source-matched public APIs; every
        # task-host command and its MCP/media/recovery work uses the installed
        # binary in a private working directory, with loopback providers only.
        vapi_env = dict(env, CARGO_TARGET_DIR=str(source / "target/vapi-fixture"),
                        CARGO_BUILD_JOBS="3")
        command = ["cargo", "test", "--locked", "-p", "connect-server",
                   "--test", "gate4_vapi_host_chat", "--test", "gate4_vapi_host_voice",
                   "--", "--nocapture", "--test-threads=1"]
        result = run_phase("installed-vapi-host", command, 2400, directory, source, vapi_env)
        report["steps"].append(result)
        check_test_results(result, directory)
        if result["status"] != "passed" or result.get("rust_tests") != {"passed": 10, "failed": 0, "ignored": 0}:
            result["status"] = "failed"
            raise RuntimeError("Installed Vapi host did not execute all eight journeys and two process-ownership controls")
        report["vapi_runtime"] = {"binary": str(release / "bin/connect"),
                                  "scope": "local simulator; no cloud provider or model inference",
                                  "checks": "chat restart/effect replay, scoped task owner with separate administrator, exact-resource recovery, rejection before dispatch, callback expiry with retained conversation, native voice with MCP, ready/signal ordering and interrupted creation cleanup"}
        save()
        report["status"] = "passed"
    except BaseException as error:
        report.update(status="failed", error={"type": type(error).__name__, "detail": str(error)})
        raise
    finally:
        save()
    return report
