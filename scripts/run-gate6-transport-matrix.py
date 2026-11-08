#!/usr/bin/env python3
"""Local ARM G6-058 execution ledger: probe, run all 32 workloads, merge hosts.

Native workload failures are recorded per cell and do not stop later cells.
Browser/full-cell observer gaps stay explicit until their producers exist.
No CI dispatch, cloud provisioning, provider calls or inherited cell passes.
"""
import argparse
import contextlib
import datetime
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import traceback

from lib.transport_matrix import digest, inventory, load_contract
from lib.vm_workload_lease import GuardedVmWorkloadLease
from lib.matrix_storage import (MetadataReserve, StorageAdmissionError, atomic_json,
                                atomic_bytes, metadata_failure, preflight, request_host, ComponentStorageFailure)

_METADATA_RESERVE = None
from lib.transport_matrix_execution import configure_execution, full_run_preflight, merge_hosts, record_browser_result, record_native_result, select_cells
from lib.transport_matrix_verdict import evaluate_cell, blocked, summarize

ROOT = Path(__file__).resolve().parents[1]


def save(path, value):
    try:
        atomic_json(path, value, reserve=_METADATA_RESERVE)
    except OSError:
        metadata_failure(dict(path_name=path.name, value=value))


def command_version(command):
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=10)
        return result.stdout.strip()[:2048] if result.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def browser_runtime_arguments(cell, args):
    # Worker overrides were probed on the worker's host. A Mac server's remote
    # browser runs in the ARM Linux guest and uses its pinned Linux defaults;
    # forwarding a Mac .app path cannot select a guest executable.
    if cell["server"] == "macos-arm64" and cell["network"] == "remote-profile-fixture":
        return []
    return [value for name in ("chrome", "firefox") if getattr(args, name)
            for value in ("--" + name, str(getattr(args, name)))]


def probe(chrome=None, firefox=None, browser_image=None):
    system, machine = platform.system(), platform.machine()
    if (system, machine) == ("Darwin", "arm64"):
        server = "macos-arm64"
    elif (system, machine) == ("Linux", "aarch64"):
        release = platform.freedesktop_os_release()
        if release.get("ID") != "ubuntu" or release.get("VERSION_ID") != "24.04":
            raise ValueError("Ubuntu24.04 required")
        server = "ubuntu24-arm64"
    else:
        raise ValueError("Only owner's local Mac/Ubuntu ARM execution is supported")
    versions = {name: command_version([name, flag]) for name, flag in (
        ("rustc", "--version"), ("node", "--version"), ("ffmpeg", "-version"), ("tcpdump", "--version"))}
    blockers = ["missing-tool-" + name for name in ("cargo", "rustc", "node", "npm", "ffmpeg", "cc", "pkg-config")
                if shutil.which(name) is None]
    media_libraries = {name: command_version(["pkg-config", "--modversion", name])
                       for name in ("opus", "vpx", "openssl")}
    blockers.extend("missing-development-library-" + name
                    for name, version in media_libraries.items() if not version)
    if not (versions["rustc"] or "").startswith("rustc 1.97.1 "):
        blockers.append("pinned-rust-1.97.1-unavailable")
    if int((versions["node"] or "v0").lstrip("v").split(".")[0]) < 22:
        blockers.append("node-22-or-newer-unavailable")
    free = shutil.disk_usage(ROOT).free
    from lib.matrix_storage import source_budget
    if free < sum(source_budget().values()):
        blockers.append("finite-source-storage-unavailable-at-initial-probe")
    # Capture is independently required, but its absence must not prevent
    # authenticated control, movement and decoded-media observations.
    capture_blockers = [] if shutil.which("tcpdump") else ["missing-tool-tcpdump"]
    capture_permissions = None
    if system == "Darwin":
        devices = list(Path("/dev").glob("bpf[0-9]*"))
        accessible = sum(os.access(path, os.R_OK | os.W_OK) for path in devices)
        capture_permissions = dict(method="os.access(read|write) over current /dev/bpf devices",
                                   device_count=len(devices), accessible_device_count=accessible,
                                   capture_started=False)
        if not accessible:
            capture_blockers.append("macos-bpf-capture-permission-unavailable")
    expected = load_contract(ROOT)["expected_browsers_by_client_platform"][server]
    linux_chrome = Path("/ms-playwright/chromium-1248/chrome-linux-arm64/chrome")
    if not linux_chrome.is_file():
        linux_chrome = Path.home() / ".cache/ms-playwright/chromium-1248/chrome-linux-arm64/chrome"
    defaults = {"chrome": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" if system == "Darwin"
                else str(linux_chrome),
                "firefox": "/Applications/Firefox.app/Contents/MacOS/firefox" if system == "Darwin" else "firefox"}
    browsers = {}
    for name, supplied in (("chrome", chrome), ("firefox", firefox)):
        version = command_version([str(supplied or defaults[name]), "--version"])
        # Browser output is public version data. Paths and profile data are not published.
        actual = version.split()[-1] if version else None
        browsers[name] = dict(version=version, expected_version=expected[name],
                              matches_frozen_version=actual == expected[name])
    verified_image = None
    if browser_image is not None:
        if server != "ubuntu24-arm64" or not re.fullmatch(r"sha256:[0-9a-f]{64}", browser_image):
            raise ValueError("Supply an exact installed ARM Linux browser image ID")
        image = json.loads(subprocess.check_output(["docker", "image", "inspect", browser_image], text=True))[0]
        if image["Id"] != browser_image or image["Architecture"] != "arm64" or image["Os"] != "linux":
            raise ValueError("Browser image architecture or identity differs")
        version = command_version(["docker", "run", "--rm", "--network", "none", "--read-only",
            "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            browser_image, "/ms-playwright/chromium-1248/chrome-linux-arm64/chrome", "--version"])
        if version != "Google Chrome for Testing " + expected["chrome"]:
            raise ValueError("Browser image lacks the frozen Chrome version")
        verified_image = browser_image
        browsers["chrome"] = dict(version=version, expected_version=expected["chrome"], matches_frozen_version=True)
    return dict(server=server, system=system, architecture=machine, kernel=platform.release(),
                observed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), versions=versions,
                media_libraries=media_libraries,
                browsers=browsers, scratch_free_bytes=free, execution_blockers=blockers,
                browser_runtime_image_id=verified_image,
                capture_permission_observation=capture_permissions,
                capture_blockers=capture_blockers,
                owned_systemd_vm=(system == "Linux" and os.getuid() != 0
                    and Path("/run/systemd/system").is_dir()
                    and os.environ.get("CONNECT_GATE6_OWNED_LINUX_VM") == "connect-gate6"),
                capture_access="not-proven-by-tool-version; actual capture required by workload")


def execute_selected(matrix, ids, output, execute, *, storage_admit=None, drain_archive=None, drain_guest=None):
    """Persist before/after each original attempt; keep going after failures."""
    storage_backpressure = None
    for cell in select_cells(matrix, ids):
        full = matrix.get("requested_stage") == "full"
        if not cell["execution"]["runnable"]:
            if full and cell["server"] == matrix["execution_environment"]["server"]:
                cell.update(status="blocked", reason="environment-preflight")
                cell["check_results"] = {name: blocked(
                    ",".join(cell["execution"]["blockers"]), category="environment") for name in cell["required_checks"]}
                save(output / "matrix.json", matrix)
            continue
        if storage_backpressure is not None:
            cell.update(status="blocked",reason="original-closed-evidence-storage-backpressure")
            cell["execution"].update(attempted=False,component_owner_started=False,
                                      storage_backpressure=storage_backpressure)
            cell["check_results"] = {name: blocked("original-closed-evidence-storage-backpressure",category="environment")
                                     for name in cell["required_checks"]}
            save(output / "matrix.json",matrix)
            continue
        cell["status"] = "running"
        cell["reason"] = "original-required-workload-running"
        save(output / "matrix.json", matrix)
        lease = GuardedVmWorkloadLease() if platform.system() == "Linux" else contextlib.nullcontext()
        entered = False
        component_started = False
        try:
            lease.__enter__()
            entered = True
            if storage_admit is not None:
                try:
                    cell["execution"]["storage_admission"] = storage_admit(cell)
                except Exception as error:
                    if hasattr(lease, "complete_unentered"):
                        lease.complete_unentered()
                    storage_backpressure = dict(cell_id=cell["id"],reason="permanent-storage-admission-refusal")
                    cell.update(status="blocked", reason="storage-preflight-before-component-entry")
                    cell["execution"].update(attempted=False, component_owner_started=False,
                                              storage_admission=getattr(error, "observation", dict(
                                                  admitted=False, reasons=["storage-observation-" + type(error).__name__])))
                    cell["check_results"] = {name: blocked("storage-preflight", category="environment")
                                             for name in cell["required_checks"]}
                    continue
            component_started = True
            report, report_sha256, code = execute(cell)
            if hasattr(lease, "complete"):
                lease.complete(report)
            if full:
                failure = report.get("failure") or {}
                cell["execution"].update(attempted=True, component=dict(report_sha256=report_sha256,
                    exit_code=code, failure_phase=failure.get("phase"), failure_kind=failure.get("name")))
                evaluate_cell(cell, report, report_sha256, environment=matrix["execution_environment"], revision=matrix["source_revision"])
                from lib.matrix_archive import archive_joined
                directory = output / "cells" / cell["server"] / cell["network"] / cell["transport"] / cell["signing"]
                if directory.is_dir():
                    try:
                        if drain_guest is not None and cell["server"] == "macos-arm64" and cell["network"] == "remote-profile-fixture":
                            save(output / "matrix.json",matrix)
                            cell["execution"]["guest_payload_custody"] = drain_guest(cell,directory)
                        cell["execution"]["evidence_archive"] = archive_joined(directory, report)
                        if not cell["execution"]["evidence_archive"]:
                            storage_backpressure = dict(cell_id=cell["id"],reason="original-archive-custody-unconfirmed")
                        # Publish the actual terminal cell before its coordinator
                        # binds the stable dictionary and original archive.
                        save(output / "matrix.json", matrix)
                        if drain_archive is not None and cell["execution"]["evidence_archive"]:
                            cell["execution"]["archive_custody"] = drain_archive(cell, report)
                    except Exception as error:
                        # Preserve the expanded original on any archival doubt;
                        # storage housekeeping cannot rewrite a behavior verdict.
                        cell["execution"]["archive_failure"] = type(error).__name__
                        storage_backpressure = dict(cell_id=cell["id"],reason="original-archive-drain-" + type(error).__name__)
            else:
                record = record_browser_result if cell["execution"].get("producer") in {"browser-admission", "browser-programme", "browser-quality"} else record_native_result
                record(matrix, cell, report, report_sha256, code)
        except ComponentStorageFailure as error:
            fallback = error.fallback
            if (fallback["source_revision"] != matrix["source_revision"] or fallback["cell_id"] != cell["id"]
                    or fallback["full_report_saved"] is not False or fallback["full_media_verification_available"] is not False):
                raise ValueError("Original unavailable component identity differs") from error
            if hasattr(lease, "complete_storage_failure"):
                lease.complete_storage_failure(fallback)
            cell.update(status="failed",reason="original-component-report-storage-unavailable")
            cell["execution"].update(attempted=True,component_owner_started=True,storage_failure=fallback)
            storage_backpressure = dict(cell_id=cell["id"],reason="original-full-report-storage-unavailable")
            cell["check_results"] = {name: blocked("original-component-report-storage-unavailable",category="harness")
                                     for name in cell["required_checks"]}
        except Exception as error:
            cell.update(status="failed", reason="workload-execution-or-evidence-binding-failed")
            cell["execution"].update(attempted=component_started, component_owner_started=component_started,
                                      failure_class=type(error).__name__)
            if full:
                cell["check_results"] = {name: blocked("fixture-" + type(error).__name__, category="harness")
                                         for name in cell["required_checks"]}
            # Exception text may include credentials/paths; only private traceback retains it.
            failure_path = output / (cell["transport"] + "-" + cell["signing"] + "-" + cell["network"] + "-failure.log")
            try:
                atomic_bytes(failure_path, traceback.format_exc()[-65536:].encode(), reserve=_METADATA_RESERVE)
            except OSError:
                metadata_failure(dict(cell_id=cell["id"], failure_class=type(error).__name__,
                                      full_component_evidence_available=False))
        except BaseException:
            cell.update(status="failed", reason="original-workload-interrupted")
            cell["execution"]["attempted"] = component_started
            cell["execution"]["component_owner_started"] = component_started
            raise
        finally:
            if entered:
                lease.__exit__(None, None, None)
            if full:
                summarize(matrix)
            save(output / "matrix.json", matrix)
            if getattr(lease, "blocked", False):
                raise RuntimeError("Original ARM VM cleanup unconfirmed; workload lease retained")
    return matrix


def main():
    global _METADATA_RESERVE
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("probe", "run", "worker", "merge"), required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private directory under .local")
    parser.add_argument("--host-report", type=Path, action="append", default=[])
    parser.add_argument("--chrome", type=Path)
    parser.add_argument("--firefox", type=Path)
    parser.add_argument("--stage", choices=("full", "media", "admission", "programme", "quality", "non-av"), default="full",
                        help="Runs require full; partial stages are read-only probe views")
    parser.add_argument("--browser-image", help="Exact already installed ARM image for browser admission")
    parser.add_argument("--expected-revision", help="Coordinator binding for an internal full-host worker")
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") or os.environ.get("CI"):
        parser.error("This runner is for the owner's local ARM resources")
    if args.host_report and args.phase != "merge":
        parser.error("Host reports belong to merge")
    if args.phase in {"run", "worker"} and args.stage != "full":
        parser.error("Every G6-058 run must execute the full 32-cell workload; partial stages are probe-only")
    if args.phase == "run":
        from lib.transport_matrix_coordinator import run
        return run(ROOT, args)
    if args.phase == "worker" and not args.expected_revision:
        parser.error("Workers require the full-run coordinator's exact candidate")
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / ".local").resolve()):
        parser.error("Results must stay under ignored .local")
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    _METADATA_RESERVE = MetadataReserve(output / "metadata.reserve")
    os.chdir(ROOT)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if args.expected_revision and revision != args.expected_revision:
        raise ValueError("Worker candidate differs from coordinator")
    matrix = inventory(ROOT, revision)
    selected_ids = [cell["id"] for cell in matrix["cells"]]
    matrix["execution_phase"] = args.phase
    if args.phase == "merge":
        if not args.host_report:
            parser.error("Supply current reports from local hosts")
        matrix = merge_hosts(matrix, [json.loads(p.read_text()) for p in args.host_report])
        save(output / "matrix.json", matrix)
        print(json.dumps(dict(summary=summarize(matrix), host_reports=matrix["host_reports_received"])))
        return 0
    configure_execution(matrix, probe(args.chrome, args.firefox, args.browser_image), args.stage)
    save(output / "matrix.json", matrix)
    if args.phase == "probe":
        print(json.dumps(dict(required_cells=32, runnable_workloads=sum(c["execution"]["runnable"] for c in matrix["cells"]),
                              qualified_cells=0, tests_executed=False)))
        return 0
    wiring_preflight = full_run_preflight(matrix)
    save(output / "full-run-preflight.json", wiring_preflight)
    selected = select_cells(matrix, selected_ids)
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=normal"], text=True).strip():
        raise ValueError("Commit the candidate before execution")
    state = dict(stage="preparing", controller_pid=os.getpid(), source_revision=revision,
                 source_guard_active=False, selected_cells=selected_ids)
    save(output / "state.json", state)
    try:
        if any(c["execution"]["runnable"] for c in selected):
            with (output / "prepare.log").open("x") as log:
                subprocess.run(["bash", "scripts/prepare-dependencies.sh"], stdout=log, stderr=subprocess.STDOUT,
                               check=True, timeout=600)
                subprocess.run(["npm", "run", "build", "--prefix", "sdk/typescript"], stdout=log,
                               stderr=subprocess.STDOUT, check=True, timeout=180)
        state.update(stage="running", source_guard_active=True)
        save(output / "state.json", state)

        def execute(cell):
            directory = output / "cells" / cell["server"] / cell["network"] / cell["transport"] / cell["signing"]
            if cell["execution"]["producer"] in {"browser-admission", "browser-programme", "browser-quality"}:
                directory.parent.mkdir(parents=True, exist_ok=True)
                loader = importlib.util.spec_from_file_location("matrix_browser_fixture", ROOT / "scripts/rehearse-gate6-browser-matrix.py")
                module = importlib.util.module_from_spec(loader)
                loader.loader.exec_module(module)
                original_argv = sys.argv
                sys.argv = [str(module.__file__), "--stage", cell["execution"]["stage"], "--transport", cell["transport"],
                            "--signing", cell["signing"], "--network", cell["network"], "--output", str(directory)]
                if args.stage == "full":
                    sys.argv += ["--resource-cycles", "3"]
                if args.browser_image:
                    sys.argv += ["--image", args.browser_image]
                sys.argv += browser_runtime_arguments(cell, args)
                try:
                    code = module.main()
                finally:
                    sys.argv = original_argv
                path = directory / "component.json"
                return json.loads(path.read_text()), digest(path), code
            directory.mkdir(parents=True, exist_ok=False)
            loader = importlib.util.spec_from_file_location("matrix_native_fixture", ROOT / "scripts/check-transport-fixtures.py")
            module = importlib.util.module_from_spec(loader)
            loader.loader.exec_module(module)
            module.BASE = directory
            with (directory / "controller.log").open("x") as log, contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
                code = module.main(media_workload=args.stage != "non-av", data_workload=args.stage == "non-av", signing_algorithm=cell["signing"],
                                   network_profile="local" if cell["network"] == "local" else "remote",
                                   host_capture=True, native_replacement_commit_trace=args.stage == "full",
                                   native_authority_summary=args.stage == "full", resource_cycles=3 if args.stage == "full" else 1)
            path = directory / ("evidence/data/rehearsal.json" if args.stage == "non-av" else "evidence/media/rehearsal.json")
            return json.loads(path.read_text()), digest(path), code

        def storage_admit(cell):
            # Runs only after the original ARM lease; guest setup performs the
            # equivalent observation after its own existing lease acquisition.
            if cell["server"] == "ubuntu24-arm64":
                return preflight(output)
            from lib.matrix_storage import cell_budget, volume, await_admission
            return await_admission(lambda:volume(output),sum(cell_budget().values()),
                                   required_by_role={"backing":0})
        def drain_archive(cell, report):
            if cell["server"] != "ubuntu24-arm64":
                return dict(local_host_archive=True, remote_original_unlink=False)
            from lib.matrix_archive_drain import drain_original
            return drain_original(output, matrix["source_revision"], cell, report)
        def drain_guest(cell,directory):
            return request_host("drain-guest",dict(cell=cell,component_directory=str(directory),
                                                  source_revision=matrix["source_revision"]))
        execute_selected(matrix, selected_ids, output, execute,
                         storage_admit=storage_admit, drain_archive=drain_archive,drain_guest=drain_guest)
    finally:
        state.update(stage="complete", source_guard_active=False)
        save(output / "state.json", state)
        save(output / "matrix.json", matrix)
    print(json.dumps(dict(summary=summarize(matrix), matrix_qualified=matrix["matrix_qualified"])))
    save(output / "matrix.json", matrix)
    return 0 if all(c["status"] == "passed" for c in selected if c["server"] == matrix["execution_environment"]["server"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
