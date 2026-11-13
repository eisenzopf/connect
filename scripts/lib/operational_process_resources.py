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
MAX_TASKS_PER_PROCESS = 4096
MAX_SAMPLES = 180
MAX_FD_DIAGNOSTICS_PER_OUTCOME = 16


def stat_fields(raw):
    # comm can contain spaces and parentheses; parse only numeric fields after it.
    tail = raw[raw.rfind(")") + 2:].split()
    if len(tail) < 22:
        raise ValueError("Incomplete process metadata")
    return dict(state=tail[0], parent=int(tail[1]), group=int(tail[2]),
        cpu_ticks=int(tail[11]) + int(tail[12]), start_ticks=int(tail[19]),
        rss_pages=max(0, int(tail[21])), threads=max(0, int(tail[17])))


class ProcessResources:
    def __init__(self, roots, proc=Path("/proc"), clock_ticks=None, page_size=None,
                 *, roles=ROLES, open_resources=False, max_samples=MAX_SAMPLES):
        if (not roles or len(set(roles)) != len(roles) or not set(roles) <= set(ROLES)
                or set(roots) != set(roles) or len(set(roots.values())) != len(roles)):
            raise ValueError("Distinct roots for the exact declared process roles required")
        if type(open_resources) is not bool or type(max_samples) is not int or not 1 <= max_samples <= 1800:
            raise ValueError("Invalid process observation bounds")
        self.roles = tuple(roles)
        self.open_resources = open_resources
        self.max_samples = max_samples
        self.proc = Path(proc)
        self.clock_ticks = clock_ticks or os.sysconf("SC_CLK_TCK")
        self.page_size = page_size or os.sysconf("SC_PAGE_SIZE")
        self.known = {}
        self.roots = {}
        self.max_cpu = {}
        self.samples = []
        self.fd_diagnostics = {"retired": [], "unavailable": []}
        self.fd_diagnostic_overflow = {"retired": 0, "unavailable": 0}
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

    @staticmethod
    def _error_metadata(error):
        if error is None:
            return None
        kind = ("FileNotFoundError" if isinstance(error, FileNotFoundError) else
                "PermissionError" if isinstance(error, PermissionError) else
                "ValueError" if isinstance(error, ValueError) else "OSError")
        number = getattr(error, "errno", None)
        return dict(kind=kind, errno=number if type(number) is int else None)

    @staticmethod
    def _identity_metadata(value):
        if value is None:
            return None
        return {name: value[name] for name in ("start_ticks", "state", "threads")}

    def _identity_after(self, key, first=None):
        """A zombie leader alone can leave live threads with a shared FD table.

        Accept a terminal identity only with a single task and a SECOND fresh
        same-birth terminal/single-task stat. A live/unknown confirmation is
        unavailable. This is sampled retirement, never an original-owner join.
        """
        pid, started = key
        result = dict(status="unavailable", after=first, confirmation=None, identity_error=None)
        if first is None:
            try:
                first = self._stat(pid)
                result["after"] = first
            except FileNotFoundError as error:
                result.update(status="retired", identity_error=self._error_metadata(error))
                return result
            except (OSError, ValueError) as error:
                result["identity_error"] = self._error_metadata(error)
                return result
        if first["start_ticks"] != started:
            result["status"] = "retired"  # The original identity, not the reused PID.
        elif first["state"] not in ("Z", "X"):
            result["status"] = "current"
        elif first["threads"] == 1:
            try:
                second = self._stat(pid)
                result["confirmation"] = second
                if (second["start_ticks"] != started or
                        (second["state"] in ("Z", "X") and second["threads"] == 1)):
                    result["status"] = "retired"
            except FileNotFoundError as error:
                result.update(status="retired", identity_error=self._error_metadata(error))
            except (OSError, ValueError) as error:
                result["identity_error"] = self._error_metadata(error)
        return result

    def _fd_diagnostic(self, key, role, before, identity, error, phase, status):
        # First16 exceptional observations per outcome; normal retirement noise
        # cannot consume the unavailable-outcome slots. No names/paths/messages.
        events = self.fd_diagnostics[status]
        if len(events) >= MAX_FD_DIAGNOSTICS_PER_OUTCOME:
            self.fd_diagnostic_overflow[status] += 1
            return
        events.append(dict(sample_index=len(self.samples), role=role, pid=key[0],
            original_start_ticks=key[1], phase=phase, disposition=status,
            before=self._identity_metadata(before),
            after=self._identity_metadata(identity.get("after")),
            confirmation=self._identity_metadata(identity.get("confirmation")),
            error=self._error_metadata(error), identity_error=identity.get("identity_error")))

    def sample(self):
        if len(self.samples) >= self.max_samples:
            raise ValueError("Process observation sample budget exceeded")
        values = {}
        unavailable = set()
        retired = set()
        resource_unavailable = set()
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
            except (OSError, ValueError) as error:
                unavailable.add(key)
                if self.open_resources:
                    resource_unavailable.add(key)
                    self._fd_diagnostic(key, role, None, {}, error,
                        "initial-process-metadata", "unavailable")
                continue
            if value["start_ticks"] != started:
                retired.add(key)
                continue
            if value["state"] in ("Z", "X"):
                identity = self._identity_after(key, value)
                if identity["status"] == "retired":
                    retired.add(key)
                else:
                    unavailable.add(key)
                    if self.open_resources:
                        resource_unavailable.add(key)
                        self._fd_diagnostic(key, role, value, identity, None,
                            "initial-terminal-identity", "unavailable")
                continue
            values[key] = value
            self.max_cpu[key] = max(self.max_cpu.get(key, 0), value["cpu_ticks"])
            if self.open_resources:
                try:
                    # Count numeric handles only; never inspect targets, paths
                    # or socket endpoints. Refuse an unbounded enumeration.
                    with os.scandir(self.proc / str(pid) / "fd") as entries:
                        count = 0
                        for _ in entries:
                            count += 1
                            if count > 4096:
                                raise ValueError("Process descriptor observation budget exceeded")
                except ValueError as error:
                    self._fd_diagnostic(key, role, value, {}, error,
                        "fd-enumeration", "unavailable")
                    raise  # The descriptor budget remains a hard failure.
                except OSError as error:
                    identity = self._identity_after(key)
                    # Only an absent FD directory can be explained by proven
                    # original retirement. Permission/EIO/unknown stays failed.
                    status = ("retired" if isinstance(error, FileNotFoundError)
                              and identity["status"] == "retired" else "unavailable")
                    self._fd_diagnostic(key, role, value, identity, error,
                        "fd-enumeration", status)
                    if status == "retired":
                        values.pop(key)
                        retired.add(key)
                        continue
                    resource_unavailable.add(key)
                else:
                    identity = self._identity_after(key)
                    if identity["status"] == "current":
                        value["open_descriptors"] = count
                    else:
                        status = identity["status"]
                        self._fd_diagnostic(key, role, value, identity, None,
                            "fd-post-identity", status)
                        if status == "retired":
                            values.pop(key)
                            retired.add(key)
                            continue
                        resource_unavailable.add(key)
            try:
                # A Worker-created subprocess belongs to the creating Linux
                # task's children list, not necessarily the process leader's.
                # Sample every bounded original task, then recheck the process
                # start identity before adopting any observed child.
                children = set()
                with os.scandir(self.proc / str(pid) / "task") as entries:
                    tasks = []
                    for entry in entries:
                        if entry.name.isdecimal():
                            tasks.append(entry.name)
                            if len(tasks) > MAX_TASKS_PER_PROCESS:
                                raise ValueError("Owned task observation budget exceeded")
                for task in tasks:
                    try:
                        observed = (self.proc / str(pid) / "task" / task / "children").read_text().split()
                    except FileNotFoundError:
                        continue  # Original task joined during this sample.
                    children.update(int(child) for child in observed)
                    if len(children) > MAX_PROCESSES:
                        raise ValueError("Owned child observation budget exceeded")
                if self._stat(pid)["start_ticks"] != started:
                    continue  # Never adopt children from a reused process ID.
                for child in children:
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
        for role in self.roles:
            keys = {k for k, r in self.known.items() if r == role}
            groups[role] = dict(
                live_processes=len(keys & values.keys()),
                unavailable_processes=len(keys & unavailable),
                retired_processes=len(keys & retired),
                sampled_rss_bytes=sum(v["rss_pages"] * self.page_size for k, v in values.items() if k in keys),
                observed_cpu_seconds=sum(self.max_cpu.get(k, 0) for k in keys) / self.clock_ticks,
                root_state="available" if self.roots[role] in values else
                    "unavailable" if self.roots[role] in unavailable else "retired")
            if self.open_resources:
                groups[role].update(
                    open_resource_unavailable_processes=len(keys & resource_unavailable),
                    sampled_open_descriptors=None if keys & resource_unavailable else
                        sum(v["open_descriptors"] for k, v in values.items() if k in keys),
                    sampled_threads=None if keys & unavailable else
                        sum(v["threads"] for k, v in values.items() if k in keys))
        sample = dict(elapsed_seconds=round(time.monotonic() - self.started, 6), groups=groups)
        self.samples.append(sample)
        return sample

    def summary(self):
        result = dict(
            scope="Sampled RSS and observed lifetime CPU for the exact declared owned Linux process trees. Browser includes its owned Node control driver when present. RSS sums may double-count shared pages; short-lived descendants between samples can be missed. No command lines, environment, names, keys, paths or media.",
            sample_count=len(self.samples),
            groups={role: dict(
                peak_sampled_rss_bytes=max((s["groups"][role]["sampled_rss_bytes"] for s in self.samples), default=None),
                observed_cpu_seconds=max((s["groups"][role]["observed_cpu_seconds"] for s in self.samples), default=None),
                peak_live_processes=max((s["groups"][role]["live_processes"] for s in self.samples), default=None),
                unavailable_samples=sum(s["groups"][role]["unavailable_processes"] > 0 for s in self.samples),
            ) for role in self.roles},
            samples=self.samples)
        if self.open_resources:
            result["fd_observation_diagnostics"] = dict(
                format="connect/numeric-fd-observation-diagnostics/1", reported=True,
                maximum_events_per_outcome=MAX_FD_DIAGNOSTICS_PER_OUTCOME,
                events={key: list(value) for key, value in self.fd_diagnostics.items()},
                overflow=dict(self.fd_diagnostic_overflow),
                scope="Exceptional original PID/birth/state/thread/errno metadata only; no names, paths, FD targets, or original-owner join inference.")
            for role in self.roles:
                group = result["groups"][role]
                observed = [s["groups"][role] for s in self.samples]
                group.update(
                    peak_sampled_open_descriptors=max((s["sampled_open_descriptors"] for s in observed
                        if s["sampled_open_descriptors"] is not None), default=None),
                    peak_sampled_threads=max((s["sampled_threads"] for s in observed
                        if s["sampled_threads"] is not None), default=None),
                    open_resource_unavailable_samples=sum(s["open_resource_unavailable_processes"] > 0
                        for s in observed))
        return result
