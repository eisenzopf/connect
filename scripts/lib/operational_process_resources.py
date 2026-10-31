"""Numeric observations of explicitly owned Linux server/helper/browser roots.

No command lines, environment, paths or process names enter observations.
RSS sums can count shared pages more than once. Short-lived descendants between
samples can be missed; these are sampled process observations, not an allocation
trace or a capacity/performance qualification.
"""
from pathlib import Path
import os
import time

ROLES = ("server", "helper", "browser")
MAX_PROCESSES = 128
MAX_SAMPLES = 180


def stat_fields(raw):
    # comm can contain spaces and parentheses; parse only numeric fields after it.
    tail = raw[raw.rfind(")") + 2:].split()
    if len(tail) < 22:
        raise ValueError("Incomplete process metadata")
    return dict(state=tail[0], parent=int(tail[1]), group=int(tail[2]),
        cpu_ticks=int(tail[11]) + int(tail[12]), start_ticks=int(tail[19]),
        rss_pages=max(0, int(tail[21])))


class ProcessResources:
    def __init__(self, roots, proc=Path("/proc"), clock_ticks=None, page_size=None):
        if set(roots) != set(ROLES) or len(set(roots.values())) != 3:
            raise ValueError("Three distinct owned process roots required")
        self.proc = Path(proc)
        self.clock_ticks = clock_ticks or os.sysconf("SC_CLK_TCK")
        self.page_size = page_size or os.sysconf("SC_PAGE_SIZE")
        self.known = {}
        self.roots = {}
        self.max_cpu = {}
        self.samples = []
        self.started = time.monotonic()
        for role, pid in roots.items():
            value = self._stat(pid)
            # The profile launches each leader with start_new_session=True.
            if value["group"] != pid:
                raise ValueError("Process root must own its new session")
            key = (pid, value["start_ticks"])
            self.known[key] = role
            self.roots[role] = key

    def _stat(self, pid):
        return stat_fields((self.proc / str(pid) / "stat").read_text())

    def sample(self):
        if len(self.samples) >= MAX_SAMPLES:
            raise ValueError("Process observation sample budget exceeded")
        values = {}
        unavailable = set()
        retired = set()
        # Revalidate start time before reading children; PID reuse cannot adopt a
        # later unrelated process as this profile's root or a known descendant.
        pending = list(self.known)
        checked = set()
        while pending:
            key = pending.pop()
            if key in checked:
                continue
            checked.add(key)
            pid, started = key
            role = self.known[key]
            try:
                value = self._stat(pid)
            except FileNotFoundError:
                retired.add(key)
                continue
            except (OSError, ValueError):
                unavailable.add(key)
                continue
            if value["start_ticks"] != started or value["state"] in ("Z", "X"):
                retired.add(key)
                continue
            values[key] = value
            self.max_cpu[key] = max(self.max_cpu.get(key, 0), value["cpu_ticks"])
            try:
                children = (self.proc / str(pid) / "task" / str(pid) / "children").read_text().split()
                for child in children:
                    child = int(child)
                    child_value = self._stat(child)
                    if child_value["parent"] != pid:
                        continue
                    child_key = (child, child_value["start_ticks"])
                    previous = self.known.get(child_key)
                    if previous is not None and previous != role:
                        raise ValueError("Owned process roles overlap")
                    if child_key not in self.known:
                        if len(self.known) >= MAX_PROCESSES:
                            raise ValueError("Owned process observation budget exceeded")
                        self.known[child_key] = role
                    pending.append(child_key)
            except FileNotFoundError:
                pass  # A descendant may finish between stat and children.
            except OSError:
                unavailable.add(key)
        groups = {}
        for role in ROLES:
            keys = {k for k, r in self.known.items() if r == role}
            groups[role] = dict(
                live_processes=len(keys & values.keys()),
                unavailable_processes=len(keys & unavailable),
                retired_processes=len(keys & retired),
                sampled_rss_bytes=sum(v["rss_pages"] * self.page_size for k, v in values.items() if k in keys),
                observed_cpu_seconds=sum(self.max_cpu.get(k, 0) for k in keys) / self.clock_ticks,
                root_state="available" if self.roots[role] in values else
                    "unavailable" if self.roots[role] in unavailable else "retired")
        sample = dict(elapsed_seconds=round(time.monotonic() - self.started, 6), groups=groups)
        self.samples.append(sample)
        return sample

    def summary(self):
        return dict(
            scope="Sampled RSS and observed lifetime CPU for three explicitly owned Linux process trees. Browser includes its owned Node control driver. RSS sums may double-count shared pages; short-lived descendants between samples can be missed. No command lines, environment, names, keys, paths or media.",
            sample_count=len(self.samples),
            groups={role: dict(
                peak_sampled_rss_bytes=max((s["groups"][role]["sampled_rss_bytes"] for s in self.samples), default=None),
                observed_cpu_seconds=max((s["groups"][role]["observed_cpu_seconds"] for s in self.samples), default=None),
                peak_live_processes=max((s["groups"][role]["live_processes"] for s in self.samples), default=None),
                unavailable_samples=sum(s["groups"][role]["unavailable_processes"] > 0 for s in self.samples),
            ) for role in ROLES},
            samples=self.samples)
