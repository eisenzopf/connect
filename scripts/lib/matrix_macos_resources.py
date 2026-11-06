"""Numeric libproc observations of explicitly owned macOS fixture trees."""
import ctypes
import os
import struct
import subprocess
import time


class MacProcessResources:
    def __init__(self, roots, *, roles, open_resources=True, max_samples=300):
        if set(roots) != set(roles) or len(set(roots.values())) != len(roles):
            raise ValueError("Distinct original process roots required")
        self.lib = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
        self.lib.proc_pidinfo.restype = ctypes.c_int
        self.roles, self.limit, self.samples = roles, max_samples, []
        self.known, self.roots, self.cpu = {}, {}, {}
        for role, pid in roots.items():
            identity = self.identity(pid)
            if identity["group"] != pid:
                raise ValueError("Original process must own its session")
            key = (pid, identity["start"])
            self.known[key] = role
            self.roots[role] = key
        self.started = time.monotonic()

    def read(self, pid, flavor, size):
        buffer = ctypes.create_string_buffer(size)
        ctypes.set_errno(0)
        length = self.lib.proc_pidinfo(pid, flavor, 0, buffer, size)
        if length <= 0:
            raise OSError(ctypes.get_errno(), "Original process observation unavailable")
        return buffer.raw[:length]

    def identity(self, pid):
        value = self.read(pid, 3, 136)  # PROC_PIDTBSDINFO, SDK sys/proc_info.h
        if len(value) != 136 or struct.unpack_from("=I", value, 12)[0] != pid:
            raise ValueError("BSD process identity shape differs")
        return dict(parent=struct.unpack_from("=I", value, 16)[0], group=struct.unpack_from("=I", value, 100)[0],
                    start=struct.unpack_from("=QQ", value, 120))

    def sample(self):
        if len(self.samples) >= self.limit:
            raise ValueError("Original observation bound exceeded")
        listing = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid="], text=True, timeout=2)
        children = {}
        for row in listing.splitlines():
            pid, parent = map(int, row.split())
            children.setdefault(parent, []).append(pid)
        values, missing, retired, unavailable = {}, set(), set(), {}
        pending, checked = list(self.known), set()
        while pending:
            key = pending.pop()
            if key in checked:
                continue
            checked.add(key)
            pid, start = key
            stage = "identity-before"
            try:
                identity = self.identity(pid)
                if identity["start"] != start:
                    retired.add(key)
                    continue
                stage = "task-info"
                task = self.read(pid, 4, 96)  # PROC_PIDTASKINFO
                if len(task) != 96:
                    raise ValueError("Task observation shape differs")
                stage = "descriptor-count"
                fds = self.read(pid, 1, 4097 * 8)  # PROC_PIDLISTFDS; no descriptor targets
                if len(fds) % 8 or len(fds) >= 4097 * 8:
                    raise ValueError("Original descriptor observation bound")
                stage = "identity-after"
                if self.identity(pid)["start"] != start:
                    retired.add(key)
                    continue
                values[key] = dict(rss=struct.unpack_from("=Q", task, 8)[0],
                    threads=struct.unpack_from("=i", task, 84)[0], fds=len(fds) // 8)
                self.cpu[key] = max(self.cpu.get(key, 0), sum(struct.unpack_from("=QQ", task, 16)) / 1e9)
                for child in children.get(pid, []):
                    try:
                        observed = self.identity(child)
                    except OSError:
                        continue
                    if observed["parent"] != pid:
                        continue
                    child_key = (child, observed["start"])
                    if len(self.known) >= 512 and child_key not in self.known:
                        raise ValueError("Original process tree observation bound")
                    self.known[child_key] = self.known[key]
                    pending.append(child_key)
            except OSError as error:
                try:
                    os.kill(pid, 0)
                    missing.add(key)
                except ProcessLookupError:
                    retired.add(key)
                except PermissionError:
                    missing.add(key)
                if key in missing:
                    # Numeric, bounded attribution only. A live PID with an
                    # unreadable libproc field must still fail complete usage
                    # observation; never substitute a zero resource count.
                    unavailable[key] = dict(pid=pid, start=list(start), stage=stage,
                        errno=error.errno, error_class=type(error).__name__)
        groups = {}
        for role in self.roles:
            keys = {key for key, value in self.known.items() if value == role}
            groups[role] = dict(live_processes=len(keys & values.keys()), unavailable_processes=len(keys & missing),
                retired_processes=len(keys & retired), sampled_rss_bytes=sum(v["rss"] for k, v in values.items() if k in keys),
                observed_cpu_seconds=sum(self.cpu.get(k, 0) for k in keys),
                root_state="available" if self.roots[role] in values else "unavailable" if self.roots[role] in missing else "retired",
                open_resource_unavailable_processes=len(keys & missing),
                unavailable_observations=[unavailable[key] for key in sorted(keys & missing)],
                sampled_open_descriptors=None if keys & missing else sum(v["fds"] for k, v in values.items() if k in keys),
                sampled_threads=sum(v["threads"] for k, v in values.items() if k in keys))
        sample = dict(elapsed_seconds=time.monotonic() - self.started, groups=groups)
        self.samples.append(sample)
        return sample

    def summary(self):
        """Summarize retained original observations without sampling again."""
        return dict(
            scope="Numeric libproc observations of the declared original macOS process trees. RSS sums may double-count shared pages; short-lived descendants between samples can be missed. No command lines, environment, names, keys, paths or media.",
            sample_count=len(self.samples),
            groups={role: dict(
                peak_sampled_rss_bytes=max((s["groups"][role]["sampled_rss_bytes"] for s in self.samples), default=None),
                observed_cpu_seconds=max((s["groups"][role]["observed_cpu_seconds"] for s in self.samples), default=None),
                peak_live_processes=max((s["groups"][role]["live_processes"] for s in self.samples), default=None),
                unavailable_samples=sum(s["groups"][role]["unavailable_processes"] > 0 for s in self.samples),
                peak_sampled_open_descriptors=max((s["groups"][role]["sampled_open_descriptors"] for s in self.samples
                    if s["groups"][role]["sampled_open_descriptors"] is not None), default=None),
                peak_sampled_threads=max((s["groups"][role]["sampled_threads"] for s in self.samples), default=None),
                open_resource_unavailable_samples=sum(s["groups"][role]["open_resource_unavailable_processes"] > 0
                    for s in self.samples),
            ) for role in self.roles},
            samples=self.samples)
