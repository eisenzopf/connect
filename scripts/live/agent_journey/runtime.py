"""Owned local processes and immutable evidence."""
import datetime
import os
from pathlib import Path
import signal
import subprocess
import time
import uuid
from .common import (ROOT, TURN_SECONDS, OUTER_SECONDS, MAX_LOG, require, verify_runtime,
                     selected_environment, port, sha256, save, read_json, ps_metadata)


class Runtime:
    def __init__(self, runtime, codex, qualification, expected_profile="gate4-local"):
        self.runtime, self.codex = runtime, codex
        self.manifest = verify_runtime(runtime, qualification, expected_profile)
        self.binary = runtime / "bin/connect"
        self.common_env, self.model_env = selected_environment()
        # NativeClient inherits this harness process environment. Keep operator
        # and provider secrets out of every public helper subprocess as well.
        os.environ.clear()
        os.environ.update(self.common_env)
        timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
        self.directory = ROOT / (".local/" + expected_profile.replace("-local", "") + "-agent-access") / ("identity-model-" + timestamp)
        self.directory.mkdir(mode=0o700, parents=True, exist_ok=False)
        self.state = self.directory / "state"
        self.installation = self.directory / "host"
        self.phase = "verified-runtime"
        self.deadline = time.monotonic() + OUTER_SECONDS
        self.server = None
        self.server_log = None
        self.processes = []
        self.cleanup = []
        self.commands = []
        self.tasks, self.conversations, self.facts = {}, {}, {}
        self.observations = {}
        allocated = []
        self.quic_port, self.mcp_port = port(allocated), port(allocated)
        self.run_id = uuid.uuid4().hex
        self.report = {
            "status": "running", "scope": "local macOS arm64; actual Codex inference; no live communication providers",
            "source_revision": self.manifest["source_revision"],
            "qualification_profile": expected_profile,
            "binary_sha256": self.manifest["files"]["bin/connect"],
            "qualification_report_sha256": sha256(qualification),
            "runtime_manifest_sha256": sha256(runtime / "manifest.json"),
            "driver_files_sha256": {
                str(path.relative_to(ROOT)): sha256(path)
                for path in [ROOT / "scripts/live/agent-journey.py",
                             *sorted(Path(__file__).parent.glob("*.py"))]
            },
            "actual_codex_sha256": sha256(codex),
            "host_turn_budget_seconds": TURN_SECONDS,
            "outer_budget_seconds": OUTER_SECONDS,
            "access_mode": "regular-agent-credential",
            "process_observation": "A private exec shim records owned PID/group and exact exec/resume mode, then execs the selected actual Codex with unchanged arguments and environment. It performs no tools, context or task work.",
        }
        save(self.directory / "preflight.json", self.report)


    def remaining(self, maximum):
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, "outer-deadline")
        return min(maximum, remaining)


    def command(self, name, arguments, maximum=30, model=False, child_record=None):
        self.phase = name
        self.commands.append({"name": name, "interface": arguments[:2]})
        output = self.directory / (name + ".stdout")
        error = self.directory / (name + ".stderr")
        with output.open("xb") as stdout, error.open("xb") as stderr:
            process = subprocess.Popen(
                [str(self.binary), *map(str, arguments)], cwd=self.directory,
                env=self.model_env if model else self.common_env,
                stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                start_new_session=True,
            )
            self.processes.append(process)
            try:
                status = process.wait(timeout=self.remaining(maximum))
            except BaseException:
                self.stop_owned(process, child_record)
                raise
        require(status == 0, "delivered-cli-command-failed")
        require(output.stat().st_size <= MAX_LOG, "bounded-cli-output")
        return output.read_bytes()


    def stop_owned(self, process, child_record=None):
        """Only signal a known live owned wrapper and its verified direct child."""
        if process.poll() is not None:
            return
        if child_record is not None and child_record.is_file():
            recorded = read_json(child_record, 4096)
            child = ps_metadata(int(recorded["pid"]))
            if child is not None:
                require(child["ppid"] == process.pid and child["pgid"] == child["pid"],
                        "exact-owned-host-before-cleanup")
                require(child["executable"] == str(self.codex), "actual-owned-codex-before-cleanup")
                os.killpg(child["pgid"], signal.SIGTERM)
                # Keep the direct owner alive until it can reap its own host.
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    still = ps_metadata(child["pid"])
                    if still is not None and still["ppid"] == process.pid and still["pgid"] == child["pgid"]:
                        os.killpg(still["pgid"], signal.SIGKILL)
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
        self.cleanup.append({"pid": process.pid, "exit_code": process.returncode,
                             "owned_process_group_cleanup": True})


    def start_server(self, name):
        self.phase = name
        require(self.server is None, "no-existing-server-owner")
        path = self.directory / (name + ".log")
        self.server_log = path.open("xb")
        self.server = subprocess.Popen(
            [str(self.binary), "serve", "--config", str(self.state / "server.toml")],
            cwd=self.directory, env=self.common_env, stdin=subprocess.DEVNULL,
            stdout=self.server_log, stderr=self.server_log, start_new_session=True,
        )
        save(self.directory / (name + "-owner.json"), {"pid": self.server.pid})
        ready_by = time.monotonic() + self.remaining(15)
        while True:
            require(self.server.poll() is None, "server-live-before-readiness")
            require(path.stat().st_size <= MAX_LOG, "bounded-server-log")
            text = path.read_text(errors="replace")
            if "ready transport=quic" in text and f"http://127.0.0.1:{self.mcp_port}/mcp" in text:
                return
            require(time.monotonic() < ready_by, "server-readiness-deadline")
            time.sleep(0.1)


    def stop_server(self, name):
        if self.server is None:
            return
        process, self.server = self.server, None
        self.stop_owned(process)
        save(self.directory / (name + "-exit.json"), {"exit_code": process.returncode})
        if self.server_log is not None:
            self.server_log.close()
            self.server_log = None


