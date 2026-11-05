"""Source-bound browser/server idle observations on the same running fixture.

The browser keeps its empty page and original process alive. No GC, process
restart, new credentials or media is used to manufacture an idle baseline.
"""
import json
import math
import sys
import time
import urllib.request
from pathlib import Path

from .operational_process_resources import ProcessResources
from .matrix_macos_resources import MacProcessResources


POLICY = dict(profile="connect/matrix-idle/1", idle_seconds=10, sample_seconds=1,
              rss_retention_bytes=0, extra_descriptors=0, extra_threads=0,
              scope="No retained-resource allowance. CPU is measured, not a capacity limit.")


def owned_usage(health):
    operational = health["operational"]
    runtime = operational["runtime"]
    pools = operational["resource_pools"]
    if runtime is None or pools is None or not pools:
        raise ValueError("Missing live runtime or resource pools")
    result = {name: runtime[name] for name in ("physical_owners", "owned_tasks")}
    for pool in pools:
        if pool["state"] != "available" or pool["usage"] is None or pool["usage"]["draining"]:
            raise ValueError("Unavailable or draining pool is not idle")
        key = "pool:" + pool["kind"]
        if key in result:
            raise ValueError("Duplicate resource pool")
        result[key] = pool["usage"]["used"]
    if any(type(value) is not int or value < 0 for value in result.values()):
        raise ValueError("Invalid resource count")
    return result


def restoration(baseline, sample):
    failures = []
    if sample.get("error") or baseline.get("error"):
        return ["unavailable-observation"]
    if set(sample["usage"]) != set(baseline["usage"]):
        failures.append("changed-resource-pool-inventory")
    for key, value in sample["usage"].items():
        if key not in baseline["usage"] or value > baseline["usage"][key]:
            failures.append(key)
    for role in ("server", "browser"):
        current, original = sample["groups"][role], baseline["groups"][role]
        if (current["root_state"] != "available" or current["unavailable_processes"]
                or current["open_resource_unavailable_processes"]):
            failures.append(role + ":unavailable-original-owner")
            continue
        for field in ("live_processes", "sampled_threads", "sampled_open_descriptors", "sampled_rss_bytes"):
            if current[field] is None or original[field] is None or current[field] > original[field]:
                failures.append(role + ":" + field)
    return failures


def idle_verdict(report):
    """Recompute restoration from retained samples, not a producer's green flag."""
    try:
        if report["format"] != "connect/matrix-idle-observation/1" or report["policy"] != POLICY:
            return False
        baseline, samples, terminal = report["baseline"], report["samples"], report["terminal_monotonic_seconds"]
        if not 4 <= len(samples) <= 300 or baseline["monotonic_seconds"] >= terminal:
            return False
        if any(type(s["monotonic_seconds"]) not in (int, float) or not math.isfinite(s["monotonic_seconds"]) for s in samples):
            return False
        if any(a["monotonic_seconds"] >= b["monotonic_seconds"] for a, b in zip(samples, samples[1:])):
            return False
        final = samples[-1]
        if (final["stage"] != "idle-final" or not 10 <= final["monotonic_seconds"] - terminal <= 12
                or restoration(baseline, baseline) or restoration(baseline, final)):
            return False
        before = [s for s in samples if s["stage"] == "idle" and 0 <= s["monotonic_seconds"] - terminal <= 10]
        return len(before) >= 2 and all(not restoration(baseline, s) for s in before[-2:])
    except (KeyError, TypeError, ValueError):
        return False


class MatrixIdle:
    def __init__(self, directory, server_pid, browser_pid, endpoint, credential):
        self.directory = Path(directory)
        self.endpoint, self.credential = endpoint, credential
        collector = MacProcessResources if sys.platform == "darwin" else ProcessResources
        self.sampler = collector(dict(server=server_pid, browser=browser_pid),
            roles=("server", "browser"), open_resources=True, max_samples=300)
        self.samples, self.baselines = [], []
        self.terminal = None
        self.retired = False
        self.next_sample = 0
        self.pending_baseline = None
        self.done = False
        self.baseline = None
        self.result = None

    def take(self, stage):
        started = time.monotonic()
        try:
            request = urllib.request.Request(self.endpoint, data=b'{"op":"health"}',
                headers={"Content-Type": "application/json", "Connection": "close",
                         "Authorization": "Bearer " + self.credential})
            # No environment proxy can receive this fixture credential.
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            with opener.open(request, timeout=.5) as response:
                data = response.read(1024 * 1024 + 1)
            if len(data) > 1024 * 1024:
                raise ValueError("Health response bound")
            health = json.loads(data)["result"]["Ok"]
            if health["kind"] != "health":
                raise ValueError("Expected public health result")
            usage = owned_usage(health["data"])
            groups = self.sampler.sample()["groups"]
            sample = dict(stage=stage, monotonic_seconds=started, usage=usage, groups=groups)
        except (OSError, ValueError, KeyError, TypeError) as error:
            sample = dict(stage=stage, monotonic_seconds=started, error=type(error).__name__)
        self.samples.append(sample)
        self.next_sample = time.monotonic() + POLICY["sample_seconds"]
        return sample

    def respond(self, phase, value):
        target = self.directory / ("resources-" + phase + "-response.json")
        temporary = target.with_suffix(".tmp")
        temporary.write_text(json.dumps(value) + "\n")
        temporary.replace(target)

    def request(self, phase):
        path = self.directory / ("resources-" + phase + ".json")
        if not path.exists():
            return None
        if path.is_symlink() or path.stat().st_size > 1024:
            raise ValueError("Bounded original resource request required")
        value = json.loads(path.read_text())
        if set(value) != {"phase", "monotonic_seconds"} or value["phase"] != phase:
            raise ValueError("Invalid resource phase")
        when = value["monotonic_seconds"]
        if type(when) not in (int, float) or not math.isfinite(when) or not 0 <= time.monotonic() - when < 30:
            raise ValueError("Original Linux monotonic observation required")
        return when

    def poll(self):
        if self.done:
            return
        now = time.monotonic()
        if self.baseline is None:
            if self.pending_baseline is None:
                requested = self.request("baseline")
                if requested is None:
                    return
                self.pending_baseline = now
                self.baselines.append(self.take("baseline"))
            elif now - self.pending_baseline >= 1:
                self.baselines.append(self.take("baseline"))
                first, last = self.baselines
                if first.get("error") or last.get("error") or restoration(first, first) or restoration(last, last):
                    raise ValueError("Original baseline observations unavailable")
                # Fixed pre-workload high water, never fitted to a later leak.
                self.baseline = json.loads(json.dumps(last))
                for role in ("server", "browser"):
                    for field in ("live_processes", "sampled_threads", "sampled_open_descriptors", "sampled_rss_bytes"):
                        self.baseline["groups"][role][field] = max(s["groups"][role][field] for s in self.baselines)
                if first["usage"] != last["usage"]:
                    raise ValueError("Live resource baseline is not stable")
                self.respond("baseline", dict(passed=True, policy=POLICY))
            return
        if self.terminal is None:
            self.terminal = self.request("terminal")
            if self.terminal is not None:
                self.respond("terminal", dict(passed=True))
        if not self.retired and self.request("retired") is not None:
            if self.terminal is None:
                raise ValueError("Retirement without original terminal")
            self.retired = True
            self.next_sample = 0
        if now >= self.next_sample:
            self.take("idle" if self.retired else "workload")
        if self.retired and now >= self.terminal + POLICY["idle_seconds"]:
            final = self.take("idle-final")
            idle = [s for s in self.samples if s["stage"] == "idle"]
            within = [s for s in idle if s["monotonic_seconds"] <= self.terminal + POLICY["idle_seconds"]]
            observed = [(s, restoration(self.baseline, s)) for s in within]
            restored = next((s["monotonic_seconds"] - self.terminal for s, errors in observed if not errors), None)
            failures = restoration(self.baseline, final)
            stable = len(observed) >= 2 and all(not errors for _, errors in observed[-2:])
            passed = restored is not None and stable and not failures
            self.result = dict(format="connect/matrix-idle-observation/1", policy=POLICY,
                passed=passed, terminal_monotonic_seconds=self.terminal,
                observed_seconds=time.monotonic() - self.terminal,
                restored_within_seconds=restored, final_failures=failures,
                baseline=self.baseline, baseline_samples=self.baselines, samples=self.samples,
                process_summary=self.sampler.summary(), native_helper="absent",
                scope="Same live server and original container browser tree (Chrome, Node driver and owned PulseAudio fixture); numeric RSS/CPU/descriptors/threads and public live owner/pool counts. No process restart or forced GC. CPU is descriptive; no capacity claim. Browser sources/renderers/client joins are separate required evidence.",
                cell_qualified=False)
            self.result["passed"] = passed and idle_verdict(self.result)
            self.respond("retired", dict(passed=self.result["passed"]))
            self.done = True

    def finish(self):
        result = self.result or dict(format="connect/matrix-idle-observation/1", policy=POLICY,
            passed=False, failure="original-idle-observation-incomplete", samples=self.samples, cell_qualified=False)
        (self.directory / "resource-observation.json").write_text(json.dumps(result, indent=2) + "\n")
        return result
