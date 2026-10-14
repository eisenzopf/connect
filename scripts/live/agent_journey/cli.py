"""Run one bounded installed-agent acceptance journey with explicit inference consent."""
import argparse
import json
import os
from pathlib import Path
import signal
import sys
from .common import OUTER_SECONDS, ProofFailure, require, save
from .runtime import Runtime
from .scenario import Scenario
from .model import Model
from .observe import Observer


class Harness(Runtime, Scenario, Model, Observer):
    """Compose setup, inference and independent observation without runtime glue."""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime", type=Path)
    parser.add_argument("--qualification-report", type=Path, required=True)
    parser.add_argument("--codex-binary", type=Path, required=True)
    parser.add_argument("--allow-model-inference", action="store_true")
    parser.add_argument("--profile", choices=("gate4-local", "gate5-local"), default="gate4-local",
                        help="Require exact matching installed profile, never infer from package name")
    args = parser.parse_args()
    require(args.allow_model_inference, "explicit-inference-selection-required")
    runtime, codex = args.runtime.resolve(strict=True), args.codex_binary.resolve(strict=True)
    require(codex.is_file() and os.access(codex, os.X_OK), "selected-actual-codex-executable")
    os.umask(0o077)
    harness = Harness(runtime, codex, args.qualification_report.resolve(strict=True), args.profile)
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(runtime / "sdk/python"))
    from connect_sdk import NativeClient
    import connect_sdk.native as native
    require(Path(native.__file__).resolve() == runtime / "sdk/python/connect_sdk/native.py", "installed-native-sdk-import")

    def deadline(_signum, _frame):
        raise ProofFailure("outer-deadline")

    previous = signal.signal(signal.SIGALRM, deadline)
    signal.setitimer(signal.ITIMER_REAL, OUTER_SECONDS)
    failure = None
    try:
        harness.run(NativeClient)
    except BaseException as error:
        failure = error
        harness.report.update(status="failed", phase=harness.phase,
                              error_class=type(error).__name__,
                              failure_label=str(error) if isinstance(error, ProofFailure) else "private-operation-failed",
                              original_identity_conversations_host_intents_retained=True)
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
        try:
            harness.stop_server("server-final")
            for process in harness.processes:
                harness.stop_owned(process)
        except BaseException as error:
            failure = error
            harness.report.update(status="failed", cleanup_error_class=type(error).__name__)
        harness.report["owned_process_cleanup"] = harness.cleanup
        harness.report["delivered_cli_commands"] = harness.commands
        save(harness.directory / "result.json", harness.report)
    print(json.dumps({"status": harness.report["status"], "result": str(harness.directory / "result.json")}), flush=True)
    return 0 if failure is None else 1

