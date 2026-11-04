#!/usr/bin/env python3
"""Local ARM G6-058 execution ledger: probe, run selected workloads, merge hosts.

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
from lib.transport_matrix_execution import configure_execution, merge_hosts, record_browser_result, record_native_result, select_cells

ROOT = Path(__file__).resolve().parents[1]


def save(path, value):
    temp = path.with_suffix(path.suffix + ".tmp")
    temp.write_text(json.dumps(value, indent=2) + "\n")
    temp.replace(path)


def command_version(command):
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=10)
        return result.stdout.strip()[:2048] if result.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


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
    blockers = ["missing-tool-" + name for name in ("cargo", "rustc", "node", "npm", "ffmpeg", "tcpdump", "cc", "pkg-config")
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
    if free < 5 * 1024**3:
        blockers.append("less-than-five-GiB-workload-scratch-space")
    capture_permissions = None
    if system == "Darwin":
        devices = list(Path("/dev").glob("bpf[0-9]*"))
        accessible = sum(os.access(path, os.R_OK | os.W_OK) for path in devices)
        capture_permissions = dict(method="os.access(read|write) over current /dev/bpf devices",
                                   device_count=len(devices), accessible_device_count=accessible,
                                   capture_started=False)
        if not accessible:
            blockers.append("macos-bpf-capture-permission-unavailable")
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
                owned_systemd_vm=(system == "Linux" and os.getuid() != 0
                    and Path("/run/systemd/system").is_dir()
                    and os.environ.get("CONNECT_GATE6_OWNED_LINUX_VM") == "connect-gate6"),
                capture_access="not-proven-by-tool-version; actual capture required by workload")


def execute_selected(matrix, ids, output, execute):
    """Persist before/after each original attempt; keep going after failures."""
    for cell in select_cells(matrix, ids):
        if not cell["execution"]["runnable"]:
            continue
        cell["status"] = "running"
        cell["reason"] = "original-required-workload-running"
        save(output / "matrix.json", matrix)
        try:
            report, report_sha256, code = execute(cell)
            record = record_browser_result if cell["execution"].get("producer") in {"browser-admission", "browser-programme", "browser-quality"} else record_native_result
            record(matrix, cell, report, report_sha256, code)
        except Exception as error:
            cell.update(status="failed", reason="workload-execution-or-evidence-binding-failed")
            cell["execution"].update(attempted=True, failure_class=type(error).__name__)
            # Exception text may include credentials/paths; only private traceback retains it.
            (output / (cell["transport"] + "-" + cell["signing"] + "-" + cell["network"] + "-failure.log")).write_text(traceback.format_exc())
        except BaseException:
            cell.update(status="failed", reason="original-workload-interrupted")
            cell["execution"]["attempted"] = True
            raise
        finally:
            save(output / "matrix.json", matrix)
    return matrix


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("probe", "run", "merge"), required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private directory under .local")
    parser.add_argument("--cell", action="append", default=[], help="Exact G6-058/platform/network/transport/signing ID")
    parser.add_argument("--all-cells", action="store_true", help="Explicitly select all 32; unavailable producers remain not-run")
    parser.add_argument("--host-report", type=Path, action="append", default=[])
    parser.add_argument("--chrome", type=Path)
    parser.add_argument("--firefox", type=Path)
    parser.add_argument("--stage", choices=("media", "admission", "programme", "quality", "non-av"), default="media")
    parser.add_argument("--browser-image", help="Exact already installed ARM image for browser admission")
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") or os.environ.get("CI"):
        parser.error("This runner is for the owner's local ARM resources")
    if args.cell and args.phase != "run" or args.host_report and args.phase != "merge":
        parser.error("Cells belong to run; host reports belong to merge")
    if args.all_cells and (args.cell or args.phase != "run"):
        parser.error("--all-cells requires run and cannot be combined with --cell")
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / ".local").resolve()):
        parser.error("Results must stay under ignored .local")
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    os.chdir(ROOT)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    matrix = inventory(ROOT, revision)
    if args.all_cells:
        args.cell = [cell["id"] for cell in matrix["cells"]]
    matrix["execution_phase"] = args.phase
    if args.phase == "merge":
        if not args.host_report:
            parser.error("Supply current reports from local hosts")
        matrix = merge_hosts(matrix, [json.loads(p.read_text()) for p in args.host_report])
        save(output / "matrix.json", matrix)
        print(json.dumps(dict(required_cells=32, qualified_cells=0, host_reports=matrix["host_reports_received"])))
        return 0
    if args.stage in {"admission", "programme", "quality"} and args.phase == "run" and not args.browser_image:
        parser.error("Browser admission requires --browser-image with an exact installed image ID")
    configure_execution(matrix, probe(args.chrome, args.firefox, args.browser_image), args.stage)
    save(output / "matrix.json", matrix)
    if args.phase == "probe":
        print(json.dumps(dict(required_cells=32, runnable_workloads=sum(c["execution"]["runnable"] for c in matrix["cells"]),
                              qualified_cells=0, tests_executed=False)))
        return 0
    selected = select_cells(matrix, args.cell)
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=normal"], text=True).strip():
        raise ValueError("Commit the candidate before execution")
    state = dict(stage="preparing", controller_pid=os.getpid(), source_revision=revision,
                 source_guard_active=False, selected_cells=args.cell)
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
                            "--signing", cell["signing"], "--image", args.browser_image, "--output", str(directory)]
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
                                   host_capture=True)
            path = directory / ("evidence/data/rehearsal.json" if args.stage == "non-av" else "evidence/media/rehearsal.json")
            return json.loads(path.read_text()), digest(path), code

        execute_selected(matrix, args.cell, output, execute)
    finally:
        state.update(stage="complete", source_guard_active=False)
        save(output / "state.json", state)
        save(output / "matrix.json", matrix)
    print(json.dumps(dict(required_cells=32, selected_outcomes={c["id"]: c["status"] for c in selected},
                          qualified_cells=0, matrix_qualified=False)))
    return 1  # Full matrix acceptance is deliberately not available from components.


if __name__ == "__main__":
    raise SystemExit(main())
