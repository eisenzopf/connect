"""One local run owns both ARM hosts and all 32 original cell attempts."""
import datetime
import html
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time
import uuid

from .transport_matrix import inventory
from .transport_matrix_execution import merge_hosts, configure_execution
from .transport_matrix_verdict import blocked, summarize


VM_ENV = dict(PATH="/opt/cargo/bin:/opt/node/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    RUSTUP_HOME="/opt/rustup", CARGO_HOME="/opt/cargo", CARGO_BUILD_JOBS="4",
    CARGO_TARGET_DIR="/workspace/.local/target-arm64", CARGO_PROFILE_RELEASE_DEBUG="0",
    CARGO_NET_OFFLINE="true", CONNECT_GATE6_OWNED_LINUX_VM="connect-gate6")
IMAGE = "sha256:2856c7b7ae4eea13ba0fb4a5605b4f182557f0b1842f45efebd2c03ea66e9e6c"


def write(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def render(matrix, path):
    summary = summarize(matrix)
    esc = lambda value: html.escape(str(value))
    rows = []
    for cell in matrix["cells"]:
        checks = cell.get("check_results", {})
        detail = "".join(f'<li class="{esc(r["status"])}"><b>{esc(r["status"])}</b> {esc(name)}'
            f'{": " + esc(r["reason"]) if r.get("reason") else ""}</li>' for name, r in checks.items())
        rows.append(f'<tr><td>{esc(cell["id"].removeprefix("G6-058/"))}</td><td>{esc(cell["status"])}</td>'
                    f'<td><details><summary>{sum(r["status"] == "passed" for r in checks.values())}/{len(cell["required_checks"])} requirements passed</summary><ul>{detail}</ul></details></td></tr>')
    page = '<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="5"><title>G6-058 full matrix run</title>'
    page += '<style>body{font:16px system-ui;margin:32px;max-width:1400px;color:#18202b}table{border-collapse:collapse;width:100%}td,th{text-align:left;padding:10px;border-bottom:1px solid #ddd}li{margin:8px 0}.passed{color:#146b3a}.failed{color:#b51c28}.blocked{color:#895500}summary{cursor:pointer}code{overflow-wrap:anywhere}</style>'
    page += f'<h1>G6-058 — full 32-cell run</h1><p>Candidate <code>{esc(matrix["source_revision"])}</code>. Owner’s Mac Studio and local Ubuntu ARM VM.</p>'
    page += '<p>' + esc(json.dumps(summary["cells"])) + '</p><p>Requirements: ' + esc(json.dumps(summary["checks"])) + '</p>'
    page += '<p>Blocked means a prerequisite or observation is unavailable. It is not a pass or an executed behavior test. Every run schedules all 32 cells and evaluates all 432 requirement verdicts.</p>'
    page += '<table><thead><tr><th>Server / network / transport / signing</th><th>Cell</th><th>Requirements</th></tr></thead><tbody>' + ''.join(rows) + '</tbody></table>'
    temporary = path.with_suffix(".tmp")
    temporary.write_text(page)
    temporary.replace(path)



def refresh_reports(root, revision, workers, reports, read_report):
    """A terminal decision requires snapshots read after every original exit.

    A worker can write its final report and exit while this loop is reading
    another host. Capture exits first so that case always gets another read.
    An unreadable final snapshot is explicit missing evidence, never the last
    cached (possibly still-running) snapshot presented as a completed run.
    """
    exited = {host: process.poll() is not None for host, process in workers.items()}
    for host in workers:
        try:
            reports[host] = read_report(host)
        except (OSError, ValueError, subprocess.SubprocessError):
            if not exited[host]:
                continue
            reason = ("host-final-snapshot-unavailable" if host in reports else
                      "host-worker-failed-before-fixture")
            report = configure_execution(inventory(root, revision), dict(server=host,
                execution_blockers=[reason], browsers={}), "full")
            for cell in report["cells"]:
                if cell["server"] == host:
                    cell.update(status="blocked", reason=reason)
                    cell["check_results"] = {name: blocked(reason, category="environment")
                                            for name in cell["required_checks"]}
            reports[host] = report
    return all(exited.values())


def run(root, args):
    if (platform.system(), platform.machine()) != ("Darwin", "arm64"):
        raise ValueError("Start the complete two-host run on the owner's Mac Studio")
    output = args.output.resolve()
    if not output.is_relative_to((root / ".local").resolve()):
        raise ValueError("Results must stay under ignored .local")
    if subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True).strip():
        raise ValueError("Commit the complete candidate before running the matrix")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    base = inventory(root, revision)
    run_id = "g6-058-" + uuid.uuid4().hex
    remote_output = Path("/workspace/.local") / run_id
    state = dict(format="connect/g6-058-full-run/1", run_id=run_id, stage="running",
        source_revision=revision, controller_pid=os.getpid(), selected_cells=[c["id"] for c in base["cells"]],
        source_guard_active=True, started_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
    write(output / "state.json", state)
    workers, logs, reports = {}, [], {}
    worker = ["--phase", "worker", "--stage", "full", "--expected-revision", revision]
    mac_command = [sys.executable, str(root / "scripts/run-gate6-transport-matrix.py"), *worker, "--output", str(output / "macos-arm64")]
    for name in ("chrome", "firefox"):
        if getattr(args, name):
            mac_command += ["--" + name, str(getattr(args, name))]
    linux_command = ["colima", "--profile", "connect-gate6", "ssh", "--", "env",
        *(key + "=" + value for key, value in VM_ENV.items()), "python3", "/workspace/scripts/run-gate6-transport-matrix.py",
        *worker, "--output", str(remote_output), "--browser-image", args.browser_image or IMAGE]
    try:
        for host, command in (("macos-arm64", mac_command), ("ubuntu24-arm64", linux_command)):
            log = (output / (host + ".log")).open("xb")
            logs.append(log)
            workers[host] = subprocess.Popen(command, cwd=root, stdout=log, stderr=subprocess.STDOUT,
                                              stdin=subprocess.DEVNULL, start_new_session=True)
        def read_report(host):
            if host == "macos-arm64":
                return json.loads((output / host / "matrix.json").read_text())
            receipt = subprocess.run(["colima", "--profile", "connect-gate6", "ssh", "--", "cat", str(remote_output / "matrix.json")],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10, check=True)
            return json.loads(receipt.stdout)

        last = None
        while True:
            workers_done = refresh_reports(root, revision, workers, reports, read_report)
            merged = merge_hosts(base, list(reports.values()))
            merged.update(run_id=run_id, execution_phase="run")
            write(output / "matrix.json", merged)
            render(merged, output / "status.html")
            current = json.dumps(merged["summary"], sort_keys=True)
            if current != last:
                print(current, flush=True)
                last = current
            if workers_done:
                break
            time.sleep(2)
        state.update(stage="complete", source_guard_active=False,
            worker_exit_codes={host: process.wait() for host, process in workers.items()},
            linux_evidence_directory=str(remote_output), summary=merged["summary"],
            matrix_qualified=merged["matrix_qualified"])
        return 0 if merged["matrix_qualified"] else 1
    finally:
        for process in workers.values():
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=30)
        for log in logs:
            log.close()
        if state["stage"] != "complete":
            state.update(stage="interrupted", source_guard_active=False)
        write(output / "state.json", state)
