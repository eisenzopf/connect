"""Bounded numeric attribution for already-owned idle processes.

These snapshots are descriptive and never enter an idle verdict. They run only
after original baseline/final counters have been taken. Numeric OS reads and an
optional owned vmmap summary do not sample a stack, alter an allocator, or call
the server.
"""
import ctypes
from decimal import Decimal
import hashlib
import os
from pathlib import Path
import re
import struct
import stat
import subprocess
import time

from .operational_process_resources import stat_fields

MAX_THREADS = 256
MAX_DESCRIPTORS = 4096
MAX_KNOWN = 512
MAX_LIVE = 16
MAX_SECONDS = .05
MAX_MAP_BYTES = 1024 * 1024
MAX_MAP_REGIONS = 4096
MAX_MAP_OBJECTS = 128
MEMORY_FIELDS = {"Rss", "Pss", "Shared_Clean", "Shared_Dirty", "Private_Clean", "Private_Dirty",
    "Anonymous", "LazyFree", "Swap", "VmRSS", "RssAnon", "RssFile", "RssShmem", "VmData", "VmStk", "VmSwap"}
FD_KINDS = {0: "atalk", 1: "vnode", 2: "socket", 3: "shared-memory", 4: "semaphore", 5: "kqueue",
    6: "pipe", 7: "fsevents", 9: "netpolicy", 10: "channel", 11: "nexus"}


BROWSER_TYPES = {"renderer", "gpu-process", "utility", "zygote", "broker"}
BROWSER_UTILITIES = {"network.mojom.NetworkService", "audio.mojom.AudioService",
    "storage.mojom.StorageService", "video_capture.mojom.VideoCaptureService",
    "data_decoder.mojom.DataDecoderService", "media.mojom.VideoEncodeAcceleratorProvider"}
THREAD_LABELS = {"CrRendererMain", "Chrome_IOThread", "Chrome_ChildIOT", "Compositor",
    "VizCompositorTh", "ThreadPoolForeg", "ThreadPoolSingl", "AudioThread", "webrtc_network",
    "webrtc_worker", "webrtc_signali", "GeckoMain", "Socket Thread", "MediaDecoderSta"}


CHROME_EXECUTABLES = {"chrome", "chromium", "chrome-headless-shell", "headless_shell"}
BROWSER_EXECUTABLES = CHROME_EXECUTABLES | {"firefox", "node", "python3", "pulseaudio"}


def executable_label(value):
    leaf = value.removesuffix(" (deleted)").rsplit("/", 1)[-1]
    return leaf if leaf in BROWSER_EXECUTABLES else "unrecognized"


def stat_identity(info):
    # Target-label equality does not identify a shared-memory object. Distinct
    # same-named memfds need their actual observed device/inode/type identity.
    return hashlib.sha256(f"{info.st_dev}:{info.st_ino}:{stat.S_IFMT(info.st_mode)}".encode()).hexdigest()


def browser_labels(raw, *, executable=None):
    """Retain only two whitelisted switch values, never argv or unknown values."""
    if not raw or not raw.endswith(b"\0"):
        raise ValueError("Incomplete owned command metadata")
    args = raw.split(b"\0")[:-1]
    result = dict(state="available", process_type=None, utility_sub_type=None)
    nonempty = [arg for arg in args if arg]
    if executable in CHROME_EXECUTABLES and len(nonempty) == 1:
        # Chromium's Linux setproctitle flattens argv into one space-separated
        # title. Treat labels here as title observations, not original argv or
        # authority. Require the actual known executable and first child flag;
        # do not search arbitrary private argument text for a guessed role.
        parts = nonempty[0].split()
        if len(parts) > 1:
            result["representation"] = "flattened-process-title"
            if executable_label(parts[0].decode("utf8", errors="replace")) not in CHROME_EXECUTABLES \
                    or not parts[1].startswith(b"--type="):
                return dict(result, state="partial")
            args = parts[1:]
    for flag, key, allowed in [(b"--type", "process_type", BROWSER_TYPES),
            (b"--utility-sub-type", "utility_sub_type", BROWSER_UTILITIES)]:
        values = []
        for i, arg in enumerate(args):
            if arg == flag:
                if i + 1 == len(args):
                    raise ValueError("Incomplete owned switch")
                values.append(args[i + 1])
            elif arg.startswith(flag + b"="):
                values.append(arg[len(flag) + 1:])
        if len(values) > 1:
            raise ValueError("Ambiguous owned switch")
        if values:
            value = values[0].decode("ascii", errors="replace")
            result[key] = value if value in allowed else "unrecognized"
    return result


class ObservationDeadline(ValueError):
    pass


def bounded_bytes(path, maximum):
    with Path(path).open("rb") as source:
        value = source.read(maximum + 1)
    if len(value) > maximum:
        raise ValueError("Original diagnostic file bound")
    return value


def thread_name(value):
    # Both Tokio async and blocking workers can have this name; this is an
    # observed label, not evidence identifying which pool owns the thread.
    value = value.split(b"\0", 1)[0].strip()
    return dict(name_sha256=hashlib.sha256(value).hexdigest(),
        recognized_label="tokio-runtime-worker" if value in {b"tokio-runtime-worker", b"tokio-runtime-w"}
        else value.decode("ascii") if value in {label.encode("ascii") for label in THREAD_LABELS} else "other")


def memory_fields(raw):
    output = {}
    for line in raw.decode("ascii", errors="replace").splitlines():
        match = re.fullmatch(r"([A-Za-z_]+):\s+([0-9]+) kB", line)
        if match and match[1] in MEMORY_FIELDS:
            output[match[1] + "_bytes"] = int(match[2]) * 1024
    return output


def vmmap_summary(raw):
    """Keep the tool's actual category labels; writable is not private heap."""
    result = {}
    size = r"([0-9]+(?:\.[0-9]+)?)([KMGT]?)"
    def amount(match):
        return int(Decimal(match[1]) * (1024 ** {"": 0, "K": 1, "M": 2, "G": 3, "T": 4}[match[2]]))
    for line in raw.decode("utf8", errors="replace").splitlines():
        for prefix, category in [("ReadOnly portion of Libraries:", "readonly_libraries"), ("Writable regions:", "writable_regions")]:
            if line.startswith(prefix):
                for field in ("Total", "resident", "written", "swapped_out", "unallocated"):
                    match = re.search(r"\b" + field + "=" + size + r"(?=[ (]|$)", line)
                    if match:
                        result[category + "_" + field.lower() + "_bytes"] = amount(match)
        match = re.fullmatch(r"Physical footprint:\s+" + size, line)
        if match:
            result["physical_footprint_bytes"] = amount(match)
    return dict(numeric_categories=result, private_heap_bytes=None, anonymous_bytes=None,
        limitation="Writable/library summaries do not attribute live objects, private heap or every file-backed mapping.")


def mac_vmmap(sampler, pid, birth, directory, stage):
    """One owned, same-UID summary probe; no privilege or allocator operation."""
    if directory is None or not Path("/usr/bin/vmmap").is_file():
        return dict(state="unavailable", reason="bounded-vmmap-not-configured-or-installed")
    # SDK proc_bsdinfo pbi_uid; do not attempt a task-port probe of another UID.
    identity = sampler.read(pid, 3, 136)
    if len(identity) != 136 or struct.unpack_from("=I", identity, 20)[0] != os.getuid():
        return dict(state="unavailable", reason="original-same-uid-process-required")
    if sampler.identity(pid)["start"] != birth:
        return dict(state="unavailable", reason="original-process-changed")
    directory = Path(directory)
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    if directory.is_symlink() or directory.stat().st_uid != os.getuid() or directory.stat().st_mode & 0o077:
        raise ValueError("Original private diagnostics directory required")
    name = "vmmap-" + str(pid) + "-" + str(time.monotonic_ns()) + ".private.txt"
    path = directory / name
    observed = dict(state="unavailable", stage=stage, timeout_seconds=.5, artifact=name)
    started = time.monotonic()
    with path.open("xb") as log:
        path.chmod(0o600)
        owner = subprocess.Popen(["/usr/bin/vmmap", "-summary", str(pid)], stdin=subprocess.DEVNULL,
            stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        forced = False
        try:
            code = owner.wait(timeout=.5)
        except subprocess.TimeoutExpired:
            forced = True
            owner.kill()
            code = owner.wait()  # Reap the original probe; no replacement work.
        observed.update(original_probe_pid=owner.pid, original_probe_joined=True, probe_forced=forced,
            exit_code=code, elapsed_ms=(time.monotonic() - started) * 1000)
    if path.stat().st_size > 262144:
        observed["reason"] = "original-summary-output-bound"
        return observed
    raw = path.read_bytes()
    observed["artifact_sha256"] = hashlib.sha256(raw).hexdigest()
    if sampler.identity(pid)["start"] != birth:
        observed["reason"] = "original-process-changed"
    elif code != 0:
        observed["reason"] = "original-probe-timeout" if forced else "summary-unavailable-without-extra-privilege"
    else:
        parsed = vmmap_summary(raw)
        observed.update(parsed, state="partial" if parsed["numeric_categories"] else "unavailable",
            reason="bounded-summary-categories" if parsed["numeric_categories"] else "unrecognized-summary-categories")
    return observed


def linux_process(sampler, pid, started, check, *, browser=False):
    original = sampler._stat(pid)
    if original["start_ticks"] != started or original["state"] in {"Z", "X"}:
        return None
    base = sampler.proc / str(pid)
    result = dict(pid=pid, start_ticks=started, state=original["state"], threads=[],
        identity_before=dict(pid=pid, start_ticks=started))
    original_executable = None
    if browser:
        check()
        try:
            label = executable_label(os.readlink(base / "exe"))
            original_executable = stat_identity(os.stat(base / "exe"))
            result["executable_label"] = dict(state="available", label=label,
                stat_identity_sha256=original_executable)
        except OSError as error:
            label = None
            result["executable_label"] = dict(state="unavailable", error=type(error).__name__)
        try:
            result["browser_labels"] = browser_labels(bounded_bytes(base / "cmdline", 16384), executable=label)
        except (OSError, ValueError) as error:
            result["browser_labels"] = dict(state="unavailable", error=type(error).__name__)
    with os.scandir(base / "task") as entries:
        for entry in entries:
            check()
            if len(result["threads"]) >= MAX_THREADS:
                raise ValueError("Original thread diagnostic bound")
            tid = int(entry.name)
            try:
                value = stat_fields(bounded_bytes(Path(entry.path) / "stat", 8192).decode())
                thread = dict(tid=tid, start_ticks=value["start_ticks"], state=value["state"], cpu_ticks=value["cpu_ticks"],
                    **thread_name(bounded_bytes(Path(entry.path) / "comm", 256)))
                wait = bounded_bytes(Path(entry.path) / "wchan", 256).decode().strip()
                thread["wait_channel"] = wait if re.fullmatch(r"[A-Za-z0-9_]{1,128}", wait) else "unavailable"
                result["threads"].append(thread)
            except FileNotFoundError:
                result["retired_during_observation"] = result.get("retired_during_observation", 0) + 1
    result["descriptors"] = []
    fd_counts = {kind: 0 for kind in ("socket", "pipe", "anonymous-inode", "regular", "other")}
    fd_unavailable = 0
    with os.scandir(base / "fd") as entries:
        for inspected, entry in enumerate(entries, 1):
            check()
            if inspected > MAX_DESCRIPTORS:
                raise ValueError("Original descriptor diagnostic bound")
            try:
                value = os.readlink(entry.path)
            except FileNotFoundError:
                fd_unavailable += 1
                continue
            kind = "socket" if value.startswith("socket:[") else "pipe" if value.startswith("pipe:[") else "anonymous-inode" if value.startswith("anon_inode:") else "file"
            classification = kind
            file_identity = None
            size_bytes = None
            if kind == "file":
                check()
                try:
                    file_info = os.stat(entry.path)
                    classification = "regular" if stat.S_ISREG(file_info.st_mode) else "other"
                    file_identity = stat_identity(file_info)
                    if classification == "regular":
                        size_bytes = file_info.st_size
                    if os.readlink(entry.path) != value:
                        classification = "unavailable"
                        file_identity = None
                        size_bytes = None
                except OSError:
                    classification = "unavailable"
                    file_identity = None
                    size_bytes = None
            if classification == "unavailable":
                fd_unavailable += 1
            else:
                fd_counts[classification] += 1
            result["descriptors"].append(dict(fd=int(entry.name), kind=kind, classification=classification,
                stat_identity_sha256=file_identity, size_bytes=size_bytes,
                object_sha256=hashlib.sha256(value.encode()).hexdigest()))
    result["descriptor_classes"] = dict(state="partial" if fd_unavailable else "available",
        counts=None if fd_unavailable else fd_counts, observed_counts=fd_counts,
        unavailable_or_retired=fd_unavailable)
    for name in ["status", "smaps_rollup"]:
        check()
        try:
            result[name] = dict(state="available", **memory_fields(bounded_bytes(base / name, 65536)))
        except OSError as error:
            result[name] = dict(state="unavailable", error=type(error).__name__)
    check()
    final_identity = sampler._stat(pid)
    if final_identity["start_ticks"] != started or final_identity["state"] in {"Z", "X"}:
        raise ValueError("Original process identity changed during diagnostics")
    if original_executable is not None:
        check()
        if stat_identity(os.stat(base / "exe")) != original_executable:
            raise ValueError("Original executable changed during diagnostics")
        result["executable_identity_after_sha256"] = original_executable
    result["descriptor_target_hash_semantics"] = "readlink-label-only; use stat_identity_sha256 for observed device/inode/type"
    result["identity_after"] = dict(pid=pid, start_ticks=final_identity["start_ticks"])
    result["threads"].sort(key=lambda value: value["tid"])
    result["descriptors"].sort(key=lambda value: value["fd"])
    return result



def selected_smaps(raw, identities, check):
    """Aggregate numeric VMA RSS/PSS for selected actual FD inode hashes only."""
    if len(identities) > MAX_MAP_OBJECTS:
        raise ValueError("Original mapping target bound")
    observed = {identity: dict(observed_rss_bytes=0, observed_pss_bytes=0, observed_regions=0)
        for identity in identities}
    truncated = len(raw) > MAX_MAP_BYTES
    content = raw[:MAX_MAP_BYTES]
    if truncated:
        content = content.rsplit(b"\n", 1)[0]
    current = None
    fields = {}
    regions = 0
    incomplete = False
    header = re.compile(rb"^[0-9a-f]+-[0-9a-f]+ [rwxps-]{4} [0-9a-f]+ ([0-9a-f]+):([0-9a-f]+) ([0-9]+)(?:\s|$)")
    def finish():
        nonlocal incomplete
        if current in observed:
            row = observed[current]
            row["observed_regions"] += 1
            if "Rss" not in fields or "Pss" not in fields:
                incomplete = True
            row["observed_rss_bytes"] += fields.get("Rss", 0)
            row["observed_pss_bytes"] += fields.get("Pss", 0)
    for line in content.splitlines():
        check()
        match = header.match(line)
        if match:
            finish()
            regions += 1
            if regions > MAX_MAP_REGIONS:
                truncated = True
                current = None
                break
            device = os.makedev(int(match[1], 16), int(match[2], 16))
            inode = int(match[3])
            current = hashlib.sha256(f"{device}:{inode}:{stat.S_IFREG}".encode()).hexdigest()
            fields = {}
        else:
            match = re.fullmatch(rb"(Rss|Pss):\s+([0-9]+) kB", line)
            if match:
                fields[match[1].decode()] = int(match[2]) * 1024
    if not truncated:
        finish()
    if not regions:
        raise ValueError("Original mapping shape unavailable")
    complete = not truncated and not incomplete
    for row in observed.values():
        row["rss_bytes"] = row["observed_rss_bytes"] if complete else None
        row["pss_bytes"] = row["observed_pss_bytes"] if complete else None
    return dict(state="available" if complete else "partial", complete=complete,
        truncated=truncated, incomplete_selected_regions=incomplete,
        observed_region_headers=min(regions, MAX_MAP_REGIONS), objects=observed,
        limitation="One descriptive VMA snapshot; aliases can count shared pages more than once. Partial totals are unknown, not zero.")


def linux_fd_mappings(sampler, process, check):
    """One already-observed original renderer; same existing numeric deadline."""
    check()
    pid, birth = process["pid"], process["start_ticks"]
    first = sampler._stat(pid)
    if first["start_ticks"] != birth or first["state"] in {"Z", "X"}:
        raise ValueError("Original mapping process changed")
    expected = process.get("executable_label", {}).get("stat_identity_sha256")
    if expected is None or stat_identity(os.stat(sampler.proc / str(pid) / "exe")) != expected:
        raise ValueError("Original mapping executable unavailable")
    identities = sorted({fd["stat_identity_sha256"] for fd in process["descriptors"]
        if fd.get("classification") == "regular" and fd.get("stat_identity_sha256")})
    limited = len(identities) > MAX_MAP_OBJECTS
    selected = identities[:MAX_MAP_OBJECTS]
    if not selected:
        raise ValueError("Original mapping targets unavailable")
    check()
    with (sampler.proc / str(pid) / "smaps").open("rb") as source:
        raw = source.read(MAX_MAP_BYTES + 1)
    check()
    result = selected_smaps(raw, selected, check)
    check()
    last = sampler._stat(pid)
    if last["start_ticks"] != birth or last["state"] in {"Z", "X"} \
            or stat_identity(os.stat(sampler.proc / str(pid) / "exe")) != expected:
        raise ValueError("Original mapping identity changed during capture")
    result.update(identity_before=dict(pid=pid, start_ticks=birth), identity_after=dict(pid=pid, start_ticks=birth),
        executable_identity_sha256=expected, target_count=len(selected), targets_truncated=limited,
        unexamined_target_count=max(0, len(identities) - len(selected)),
        bytes_examined=min(len(raw), MAX_MAP_BYTES))
    if limited:
        result["state"] = "partial"
    return result

def mac_process(sampler, pid, started, check):
    if sampler.identity(pid)["start"] != started:
        return None
    task = sampler.read(pid, 4, 96)
    if len(task) != 96:
        raise ValueError("Original macOS task diagnostic shape")
    result = dict(pid=pid, start=list(started), virtual_bytes=struct.unpack_from("=Q", task, 0)[0],
        resident_bytes=struct.unpack_from("=Q", task, 8)[0], threads=[], descriptors=[])
    raw = sampler.read(pid, 6, (MAX_THREADS + 1) * 8)  # PROC_PIDLISTTHREADS
    if len(raw) % 8 or len(raw) > MAX_THREADS * 8:
        raise ValueError("Original macOS thread diagnostic bound")
    for (tid,) in struct.iter_unpack("=Q", raw):
        check()
        buffer = ctypes.create_string_buffer(112)  # SDK struct proc_threadinfo
        # PROC_PIDLISTTHREADS returns the handles consumed by
        # PROC_PIDTHREADINFO (not PROC_PIDTHREADID64INFO).
        length = sampler.lib.proc_pidinfo(pid, 5, tid, buffer, len(buffer))
        if length <= 0:
            result["retired_or_unavailable_threads"] = result.get("retired_or_unavailable_threads", 0) + 1
            continue
        if length != 112:
            raise ValueError("Original macOS thread diagnostic shape")
        value = buffer.raw
        result["threads"].append(dict(thread_handle=tid, user_ns=struct.unpack_from("=Q", value, 0)[0],
            system_ns=struct.unpack_from("=Q", value, 8)[0], run_state=struct.unpack_from("=i", value, 24)[0],
            sleep_seconds=struct.unpack_from("=i", value, 32)[0], **thread_name(value[48:112])))
    raw = sampler.read(pid, 1, (MAX_DESCRIPTORS + 1) * 8)
    if len(raw) % 8 or len(raw) > MAX_DESCRIPTORS * 8:
        raise ValueError("Original macOS descriptor diagnostic bound")
    result["descriptors"] = [dict(fd=fd, kind=FD_KINDS.get(kind, "other"), numeric_kind=kind)
        for fd, kind in struct.iter_unpack("=iI", raw)]
    if sampler.identity(pid)["start"] != started:
        raise ValueError("Original process identity changed during diagnostics")
    result["threads"].sort(key=lambda value: value["thread_handle"])
    result["descriptors"].sort(key=lambda value: value["fd"])
    return result


def idle_owner_diagnostics(sampler, *, roles=("server",), clock=time.monotonic, directory=None, stage=None):
    started = clock()
    report = dict(format="connect/idle-owner-diagnostics/1", state="unavailable", processes=[],
        requested_roles=list(roles), observed_at_monotonic_seconds=started, descriptive_only=True,
        limits=dict(seconds=MAX_SECONDS, live_processes=MAX_LIVE, threads_per_process=MAX_THREADS,
            descriptors_per_process=MAX_DESCRIPTORS, optional_vmmap_seconds=.5))
    # A Mac-server/Linux-client composite already owns the original Mac
    # collector. Never interpret a guest PID in the Mac namespace.
    if hasattr(sampler, "server"):
        sampler = sampler.server
        report["unavailable_roles"] = [role for role in roles if role != "server"]
        roles = tuple(role for role in roles if role == "server")
    def check():
        if clock() - started > MAX_SECONDS:
            raise ObservationDeadline("Original diagnostic time bound")
    try:
        if not hasattr(sampler, "known") or not hasattr(sampler, "roots"):
            raise ValueError("Original process identities unavailable")
        known = [(key, role) for key, role in sampler.known.items() if role in roles]
        if len(known) > MAX_KNOWN:
            raise ValueError("Original diagnostic identity bound")
        report["roots"] = {role: list(sampler.roots[role]) for role in roles}
        # Exact original roots first, then newest already-owned descendants.
        # No fresh /proc scan or child adoption occurs in this diagnostic hook.
        known.sort(key=lambda item: (item[0] != sampler.roots[item[1]],
            {"server": 0, "helper": 1, "browser": 2}[item[1]], -item[0][0]))
        for index, ((pid, birth), role) in enumerate(known):
            check()
            if len(report["processes"]) >= MAX_LIVE:
                report["unexamined_known_identities"] = [dict(pid=key[0], birth=key[1], role=kind)
                    for key, kind in known[index:]]
                report["truncated"] = True
                break
            try:
                observed = mac_process(sampler, pid, birth, check) if hasattr(sampler, "lib") else linux_process(sampler, pid, birth, check, browser=role == "browser")
            except ProcessLookupError:
                observed = None
            except FileNotFoundError:
                observed = None
            if observed is not None:
                if len(report["processes"]) >= MAX_LIVE:
                    raise ValueError("Original live diagnostic process bound")
                report["processes"].append(dict(role=role, **observed))
        check()
        report["original_root_states"] = {role: "observed" if any(
            value["role"] == role and value["pid"] == sampler.roots[role][0]
            for value in report["processes"]) else "retired-or-reused" for role in roles}
        report["state"] = "available" if not report.get("unavailable_roles") and not report.get("truncated") and all(value == "observed" for value in report["original_root_states"].values()) else "partial"
    except (OSError, ValueError, KeyError, TypeError) as error:
        report["state"] = "partial" if report["processes"] else "unavailable"
        report["error"] = type(error).__name__
    if not hasattr(sampler, "lib"):
        # Select only from the verified original processes already captured.
        # No new process discovery, timeline sample, probe task or time budget.
        renderers = [p for p in report["processes"] if p["role"] == "browser"
            and p.get("browser_labels", {}).get("process_type") == "renderer"
            and p.get("executable_label", {}).get("label") in CHROME_EXECUTABLES]
        if renderers:
            selected = max(renderers, key=lambda p: (sum(d.get("classification") == "regular"
                for d in p["descriptors"]), p["pid"]))
            try:
                selected["fd_mappings"] = linux_fd_mappings(sampler, selected, check)
            except (OSError, ValueError, KeyError, TypeError) as error:
                selected["fd_mappings"] = dict(state="unavailable", error=type(error).__name__,
                    objects=None, scope="same original renderer only")
        report["mapping_limits"] = dict(selected_processes=1, bytes=MAX_MAP_BYTES,
            regions=MAX_MAP_REGIONS, targets=MAX_MAP_OBJECTS,
            selection="largest regular-FD count among already observed original Chrome renderers",
            budget="inside unchanged numeric_seconds; optional partial/unavailable")
    report["numeric_elapsed_ms"] = report["elapsed_ms"] = (clock() - started) * 1000
    # The bounded task-port tool is separate from the 50ms numeric syscall
    # budget. It targets only the original Mac server, after counters and the
    # sampling deadline are fixed. No pause, GC or allocator action is requested.
    if hasattr(sampler, "lib") and stage in {"baseline", "idle-final"}:
        for value in report["processes"]:
            if value["role"] == "server" and value["pid"] == sampler.roots["server"][0]:
                try:
                    value["vmmap_summary"] = mac_vmmap(sampler, value["pid"], tuple(value["start"]), directory, stage)
                except (OSError, ValueError, subprocess.SubprocessError) as error:
                    value["vmmap_summary"] = dict(state="unavailable", error=type(error).__name__)
        report["elapsed_ms"] = (clock() - started) * 1000
    return report
