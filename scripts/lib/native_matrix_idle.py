"""Native ten-second drain: original live server and client-supervisor trees.

Use the same no-retention limits as the browser matrix. The actual Node helper
marks terminal before closing clients and waits alive through the observation.
Public health is sampled only before the workload and after that terminal;
active media gets numeric process observations, without additional control IO.
"""
import copy
import hashlib
import json
import math
from pathlib import Path
import sys
import subprocess
import time
import urllib.request

from .matrix_idle import POLICY, owned_usage
from .matrix_plateau import POLICY as PLATEAU_POLICY, CYCLE_POLICY, cycle_restoration, cycle_idle_verdict, evaluate, runtime_capacity_configuration
from .matrix_macos_resources import MacProcessResources
from .matrix_owner_diagnostics import idle_owner_diagnostics
from .operational_process_resources import ProcessResources
from .native_matrix_process import NativeOwner, publish
from .native_programme_profile import original_worker_owner, worker_join_verified

ROLES = ("server", "helper")
FIELDS = ("live_processes", "sampled_threads", "sampled_open_descriptors", "sampled_rss_bytes")


def restoration(baseline, sample):
    if sample.get("error") or baseline.get("error"):
        return ["unavailable-observation"]
    failures = []
    if set(sample["usage"]) != set(baseline["usage"]):
        failures.append("changed-resource-pool-inventory")
    for key, value in sample["usage"].items():
        if type(value) is not int or value < 0 or key not in baseline["usage"] or value > baseline["usage"][key]:
            failures.append(key)
    if set(sample["groups"]) != set(ROLES) or set(baseline["groups"]) != set(ROLES):
        return failures + ["changed-original-process-inventory"]
    for role in ROLES:
        current, original = sample["groups"][role], baseline["groups"][role]
        if (current["root_state"] != "available" or current["unavailable_processes"]
                or current["open_resource_unavailable_processes"]):
            failures.append(role + ":unavailable-original-owner")
            continue
        for field in FIELDS:
            if (type(current[field]) is not int or type(original[field]) is not int
                    or current[field] < 0 or current[field] > original[field]):
                failures.append(role + ":" + field)
        cpu = current.get("observed_cpu_seconds")
        if type(cpu) not in (int, float) or not math.isfinite(cpu) or cpu < 0:
            failures.append(role + ":unavailable-cpu-observation")
    return failures


def native_idle_verdict(report):
    try:
        if report["format"] != "connect/native-matrix-idle-observation/2" or report["policy"] != POLICY:
            return False
        baseline, samples, terminal = report["baseline"], report["samples"], report["terminal_monotonic_seconds"]
        if (not 4 <= len(samples) <= 340 or type(terminal) not in (int, float)
                or not math.isfinite(terminal) or baseline["monotonic_seconds"] >= terminal):
            return False
        if any(type(s["monotonic_seconds"]) not in (int, float) or not math.isfinite(s["monotonic_seconds"]) for s in samples):
            return False
        if any(a["monotonic_seconds"] >= b["monotonic_seconds"] for a, b in zip(samples, samples[1:])):
            return False
        final = samples[-1]
        if (final["stage"] != "idle-final" or not 10 <= final["monotonic_seconds"] - terminal <= 12
                or restoration(baseline, baseline) or restoration(baseline, final)):
            return False
        retirement = report["retirement"]
        owner, original_terminal = report["programme_owner"], report["original_terminal_monotonic_seconds"]
        if (not original_worker_owner(owner)
                or not owner["started_monotonic_seconds"] <= report["original_baseline_monotonic_seconds"] < original_terminal
                or retirement.get("original_terminal_monotonic_seconds") != original_terminal
                or not worker_join_verified(retirement.get("programme_worker"), owner, original_terminal)):
            return False
        if (retirement.get("passed") is not True or retirement.get("original_helpers_joined") is not True
                or retirement.get("original_clients_closed") is not True
                or type(retirement.get("observed_helpers")) is not int or retirement["observed_helpers"] < 1
                or type(retirement.get("all_local_owners_elapsed_ms")) not in (int, float)
                or not 0 <= retirement["all_local_owners_elapsed_ms"] <= 10000):
            return False
        before = [s for s in samples if s["stage"] == "idle" and 0 <= s["monotonic_seconds"] - terminal <= 10]
        return len(before) >= 2 and all(not restoration(baseline, s) for s in before[-2:])
    except (KeyError, TypeError, ValueError):
        return False


def health_reader(endpoint, credential):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def read():
        request = urllib.request.Request(endpoint, data=b'{"op":"health"}',
            headers={"Content-Type": "application/json", "Connection": "close", "Authorization": "Bearer " + credential})
        with opener.open(request, timeout=.5) as response:
            content = response.read(1024 * 1024 + 1)
        if len(content) > 1024 * 1024:
            raise ValueError("Public health response bound")
        result = json.loads(content)["result"]["Ok"]
        if result["kind"] != "health":
            raise ValueError("Public health result required")
        return owned_usage(result["data"])
    return read


class NativeMatrixIdle:
    def __init__(self, directory, sampler, health, *, clock=time.monotonic, prefix="", cycle_baseline=None):
        if prefix not in ("", "cycle-1-", "cycle-2-", "cycle-3-"):
            raise ValueError("Explicit original resource cycle required")
        self.prefix, self.cycle_baseline = prefix, cycle_baseline
        self.directory, self.sampler, self.health, self.clock = Path(directory), sampler, health, clock
        self.samples, self.baselines = [], []
        self.baseline = self.terminal = self.retirement = self.result = None
        self.programme_owner = self.original_baseline = self.original_terminal = None
        self.next_sample = self.next_process = 0
        self.failure = None

    def restore(self, baseline, sample):
        return cycle_restoration(baseline, sample) if self.prefix else restoration(baseline, sample)

    def take(self, stage):
        when = self.clock()
        try:
            sample = dict(stage=stage, monotonic_seconds=when, usage=self.health(), groups=self.sampler.sample()["groups"])
        except (OSError, ValueError, KeyError, TypeError) as error:
            sample = dict(stage=stage, monotonic_seconds=when, error=type(error).__name__)
        self.samples.append(sample)
        self.next_sample = self.clock() + POLICY["sample_seconds"]
        if stage in {"baseline", "idle-final"}:
            # Diagnostic only, after the original quantitative sample. No extra
            # health request, workload owner, deadline, allowance or verdict.
            sample["owner_diagnostics"] = idle_owner_diagnostics(self.sampler, roles=("server", "helper"),
                directory=self.directory / "owner-diagnostics", stage=stage)
        return sample

    def request(self, phase):
        path = self.directory / ("resources-" + self.prefix + phase + ".json")
        if not path.exists():
            return None
        if path.is_symlink() or path.stat().st_size > 4096:
            raise ValueError("Original native resource request bound")
        value = json.loads(path.read_text())
        expected = {"phase", "monotonic_seconds", "programme_owner"} | ({"retirement"} if phase == "retired" else set())
        when = value.get("monotonic_seconds")
        if (set(value) != expected or value["phase"] != phase or type(when) not in (int, float)
                or not math.isfinite(when) or not 0 <= self.clock() - when < 30):
            raise ValueError("Original same-kernel native resource request required")
        if not original_worker_owner(value.get("programme_owner")):
            raise ValueError("Original programme Worker identity required")
        return dict(value, original_monotonic_seconds=when)

    def respond(self, phase, passed):
        path = self.directory / ("resources-" + self.prefix + phase + "-response.json")
        if not path.exists():
            publish(path, dict(passed=bool(passed)))

    def poll(self):
        try:
            self._poll()
        except (OSError, ValueError, KeyError, TypeError) as error:
            self.failure = type(error).__name__
        if self.failure:
            # Missing process/health observations must not cancel the media or
            # suppress its finally cleanup. Answer each original boundary false.
            for phase in ("baseline", "terminal", "retired"):
                if (self.directory / ("resources-" + self.prefix + phase + ".json")).exists():
                    self.respond(phase, False)

    def _poll(self):
        if self.failure:
            return
        now = self.clock()
        if self.baseline is None:
            if not self.baselines:
                request = self.request("baseline")
                if request is not None:
                    self.programme_owner = request["programme_owner"]
                    self.original_baseline = request["original_monotonic_seconds"]
                    if self.programme_owner["started_monotonic_seconds"] > self.original_baseline:
                        raise ValueError("Original Worker must exist before baseline")
                    self.baselines.append(self.take("baseline"))
            elif now - self.baselines[0]["monotonic_seconds"] >= 1:
                self.baselines.append(self.take("baseline"))
                first, last = self.baselines
                if self.restore(first, first) or self.restore(last, last) or first["usage"] != last["usage"]:
                    raise ValueError("Native pre-workload baseline unavailable or unstable")
                self.baseline = copy.deepcopy(last)
                for role in ROLES:
                    for field in FIELDS:
                        self.baseline["groups"][role][field] = max(s["groups"][role][field] for s in self.baselines)
                self.respond("baseline", True)
            return
        if self.terminal is None:
            request = self.request("terminal")
            if request is not None:
                if request["programme_owner"] != self.programme_owner:
                    raise ValueError("Original programme Worker changed at terminal")
                self.terminal = request["monotonic_seconds"]
                self.original_terminal = request["original_monotonic_seconds"]
                self.respond("terminal", True)
                self.next_sample = 0
            elif now >= self.next_process:
                process = self.sampler.sample()  # Same numeric call, no Health read during AV.
                if self.prefix:
                    self.samples.append(dict(stage="workload", monotonic_seconds=now,
                        groups=copy.deepcopy(process["groups"])))
                self.next_process = now + POLICY["sample_seconds"]
                return
        if self.terminal is None:
            return
        if self.retirement is None:
            request = self.request("retired")
            if request is not None:
                if request["programme_owner"] != self.programme_owner:
                    raise ValueError("Original programme Worker changed at retirement")
                self.retirement = request["retirement"]
        if self.result is not None:
            if self.retirement is not None:
                self.respond("retired", self.result["passed"])
            return
        if now < self.terminal + POLICY["idle_seconds"] and now >= self.next_sample:
            self.take("idle" if self.retirement is not None else "drain")
        if now >= self.terminal + POLICY["idle_seconds"]:
            final = self.take("idle-final")
            failures = self.restore(self.cycle_baseline or self.baseline, final)
            self.result = dict(format="connect/native-matrix-idle-observation/2", policy=CYCLE_POLICY if self.prefix else POLICY,
                passed=False, terminal_monotonic_seconds=self.terminal,
                programme_owner=self.programme_owner, original_baseline_monotonic_seconds=self.original_baseline,
                original_terminal_monotonic_seconds=self.original_terminal,
                observed_seconds=self.clock() - self.terminal, baseline=self.baseline,
                baseline_samples=self.baselines, samples=self.samples, retirement=self.retirement or {},
                final_failures=failures, process_summary=copy.deepcopy(self.sampler.summary()) if self.prefix else self.sampler.summary(),
                scope="Same live server and original native Node controller/supervisor tree. Existing native SDK helpers and physical codecs must join. Terminal is recorded before client close. Numeric RSS/descriptors/threads and public owner/pool usage have zero retained-resource allowance; CPU is descriptive. No process restart, forced GC or media during idle.",
                cell_qualified=False)
            if self.prefix:
                self.result["scope"] = "Original per-cycle ten-second idle with exact Session-resource restoration; process warmup and plateau are evaluated separately across three real cycles of the same original OS owners."
            self.result["passed"] = (cycle_idle_verdict(self.result, self.cycle_baseline) if self.prefix else native_idle_verdict(self.result))
            if self.retirement is not None:
                self.respond("retired", self.result["passed"])

    def finish(self):
        result = self.result or dict(format="connect/native-matrix-idle-observation/2", policy=POLICY,
            passed=False, failure=self.failure or "original-native-idle-observation-incomplete",
            samples=self.samples, cell_qualified=False)
        if self.failure:
            result.update(passed=False, failure=self.failure)
        (self.directory / "resource-observation.json").write_text(json.dumps(result, indent=2) + "\n")
        return result


def native_cycle_retirement_verified(retirement, owner, terminal):
    """Recompute original cleanup bounds independently of producer green flags."""
    try:
        elapsed = retirement["all_local_owners_elapsed_ms"]
        return (retirement.get("passed") is True and retirement.get("original_helpers_joined") is True
            and retirement.get("original_clients_closed") is True
            and type(retirement.get("observed_helpers")) is int and retirement["observed_helpers"] >= 1
            and type(elapsed) in (int, float) and math.isfinite(elapsed) and 0 <= elapsed <= 10000
            and worker_join_verified(retirement.get("programme_worker"), owner, terminal))
    except (KeyError, TypeError, ValueError):
        return False


class NativeMatrixPlateau:
    """Three disjoint boundary files; all raw per-cycle reports remain on disk."""
    def __init__(self, directory, sampler, health, *, child_class=NativeMatrixIdle, clock=time.monotonic):
        self.directory, self.sampler, self.health = Path(directory), sampler, health
        self.clock, self.child_class = clock, child_class
        self.children, self.cycles = [], []
        self.baseline, self.original_owners = None, None
        self.current = None
        self.failure = None

    def owners(self, programme):
        if hasattr(self.sampler, "owner_identities"):
            owners = self.sampler.owner_identities()
        else:
            if not hasattr(self.sampler, "roots"):
                raise ValueError("Original native owner identities unavailable")
            owners = {role:dict(pid=pid, birth=list(birth) if isinstance(birth, tuple) else birth)
                      for role, (pid, birth) in self.sampler.roots.items()}
        # The helper tree root is the original supervisor; its actual Node
        # descendant must also remain the identical programme process.
        owners["helper"]["programme_node_pid"] = programme["original_node_pid"]
        return owners

    def poll(self):
        if len(self.cycles) >= 3 or self.failure:
            return
        index = len(self.cycles) + 1
        if self.current is None:
            prefix = "cycle-" + str(index) + "-"
            if not (self.directory / ("resources-" + prefix + "baseline.json")).exists():
                return
            self.current = self.child_class(self.directory, self.sampler, self.health,
                clock=self.clock, prefix=prefix, cycle_baseline=self.baseline)
            self.children.append(self.current)
        child = self.current
        child.poll()
        if child.baseline is not None and self.baseline is None:
            self.baseline = copy.deepcopy(child.baseline)
        if child.failure and child.retirement is None:
            try:
                request = child.request("retired")
                terminal_request = child.request("terminal")
                if request and terminal_request and self.clock() >= terminal_request["monotonic_seconds"] + 10:
                    child.retirement = request["retirement"]
                    child.programme_owner = request["programme_owner"]
                    child.original_terminal = terminal_request["original_monotonic_seconds"]
                    child.result = child.finish()
            except (OSError, KeyError, ValueError, TypeError):
                return
        if child.result is None or child.retirement is None:
            return
        idle = child.finish()
        retirement = child.retirement
        owner = child.programme_owner
        try:
            owners = self.owners(owner)
            if self.original_owners is None:
                self.original_owners = copy.deepcopy(owners)
            receipt = retirement["programme_worker"]
            workload = retirement.get("workload_observation", {})
            sessions = retirement.get("original_session_retirement", {})
            session_rows = sessions.get("sessions", [])
            sessions_joined = (sessions.get("format") == "connect/original-session-retirement/1"
                and sessions.get("passed") is True and len(session_rows) == 2
                and all(v.get("status") in ("ended", "already-terminal") for v in session_rows)
                and len({v.get("original_operation_sha256") for v in session_rows}) == 2)
            cycle = dict(index=index, workload_id="native-cycle-" + str(index) + "-" + str(owner["original_node_pid"]),
                workload_started=workload.get("actual_connection_workload_observed") is True,
                workload_complete=(workload.get("programme_window_observed") is True and workload.get("seconds") == 62
                    and workload.get("routes") == 6 and workload.get("original_receive_tasks_joined") is True),
                workload_observation=workload, workload_evidence_sha256=receipt["report_sha256"],
                workload_evidence_kind="original-digest-verified-native-worker-programme-report",
                original_programme_owner=owner, original_worker_receipt=receipt,
                persistent_owners=owners,
                joins=dict(session_owners_joined=sessions_joined and native_cycle_retirement_verified(retirement, owner, child.original_terminal),
                    helper_work_joined=native_cycle_retirement_verified(retirement, owner, child.original_terminal),
                    forced=receipt["forced"],
                    evidence_sha256=hashlib.sha256(json.dumps(retirement, sort_keys=True, separators=(",", ":")).encode()).hexdigest()),
                idle_resources=idle)
            self.cycles.append(cycle)
            publish(self.directory / ("resource-observation-cycle-" + str(index) + ".json"), idle)
        except (KeyError, ValueError, TypeError) as error:
            self.failure = type(error).__name__
        self.current = None

    def finish(self):
        report = dict(format="connect/matrix-plateau-observation/1", policy=PLATEAU_POLICY,
            baseline=self.baseline, persistent_owners=self.original_owners, cycles=self.cycles,
            server_runtime_capacity=runtime_capacity_configuration((self.original_owners or {}).get("server")),
            failure=self.failure, passed=False, cell_qualified=False)
        report["evaluation"] = evaluate(report)
        report["passed"] = report["evaluation"]["passed"] and self.failure is None
        publish(self.directory / "resource-observation.json", report)
        return report


def run_native_matrix(command, log, env, budget, *, server_pid, directory, cwd, endpoint, credential, wrap=None, resource_cycles=1):
    if resource_cycles not in (1, 3) or type(resource_cycles) is not int:
        raise ValueError("One or three actual native cycles required")
    if type(budget) not in (int, float) or not math.isfinite(budget) or not 20 <= budget <= 300 * resource_cycles:
        raise ValueError("Bounded native workload and original idle interval required")
    directory = Path(directory)
    directory.mkdir(mode=0o700, exist_ok=True)
    began, cutoff = time.monotonic(), time.monotonic() + budget
    owner = idle = sampler = None
    code, failure = None, None
    cleanup = dict(original_supervisor_joined=False, original_workers_joined=False, forced=False)
    try:
        # Place the captured original deadline inside the ordinary supervised
        # command: a namespace's explicit env -i must not discard it. `env`
        # execs Node in that same original child PID; no second controller.
        owned_command = ["env", "CONNECT_NATIVE_PROGRAMME_DEADLINE_MONOTONIC_SECONDS=" + str(cutoff), *command]
        owner = NativeOwner(owned_command, directory / "owner", log, cwd=cwd, env=env, wrap=wrap)
        while owner.identity() is None:
            if time.monotonic() >= min(cutoff, began + 5) or not owner.alive():
                raise ValueError("Original native supervisor did not start")
            time.sleep(.02)
        collector = MacProcessResources if sys.platform == "darwin" else ProcessResources
        try:
            sampler = collector(dict(server=server_pid, helper=owner.pid), roles=ROLES, open_resources=True, max_samples=360 * resource_cycles)
        except (OSError, ValueError) as error:
            failure = type(error).__name__
            class UnavailableSampler:
                def sample(self):
                    raise ValueError("Original native process observations unavailable")
                def summary(self):
                    return dict(sample_count=0, groups={})
            sampler = UnavailableSampler()
        idle = (NativeMatrixPlateau if resource_cycles == 3 else NativeMatrixIdle)(directory, sampler, health_reader(endpoint, credential))
        while True:
            idle.poll()
            code = owner.poll()
            if code is not None:
                break
            if not owner.alive():
                raise ValueError("Original native owner exited without completion")
            if time.monotonic() >= cutoff:
                code = "timeout"
                break
            time.sleep(.02)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        failure = type(error).__name__
        code = code if code is not None else "profile-unavailable"
    finally:
        if owner is not None:
            cleanup = owner.close()
        observation = idle.finish() if idle else dict(format="connect/native-matrix-idle-observation/2",
            policy=POLICY, passed=False, failure=failure or "native-idle-observer-not-started")
        measured = sampler.summary() if sampler else None
        complete = (failure is None and cleanup["original_supervisor_joined"] and cleanup["original_workers_joined"]
            and not cleanup["forced"] and measured is not None and measured["sample_count"] >= 2
            and all(g["unavailable_samples"] == g["open_resource_unavailable_samples"] == 0 for g in measured["groups"].values()))
        profile = dict(format="connect/native-workload-process-profile/1", elapsed_seconds=time.monotonic() - began,
            sample_count=measured["sample_count"] if measured else 0, groups=measured["groups"] if measured else {},
            **cleanup, failure=failure, collection_complete=bool(complete), resource_reclamation_qualified=False,
            fd_observation_diagnostics=(measured.get("fd_observation_diagnostics", dict(reported=False))
                if measured else dict(reported=False)),
            scope="Original native workload process observations including the original Node parent and its programme Worker; the separate idle report requires the exact natural Worker join and recomputes resource restoration.", cell_qualified=False)
        (directory / "process-resources.json").write_text(json.dumps(profile, indent=2) + "\n")
    return code, profile, observation
