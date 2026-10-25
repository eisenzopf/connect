"""Bounded process observations for one owned browser fixture, never personal Chrome."""
import datetime
import json
from pathlib import Path
import subprocess
import threading
import time


def cpu_seconds(value):
    days, clock = (value.split('-', 1) if '-' in value else ('0', value))
    total = 0.0
    for part in clock.split(':'):
        total = total * 60 + float(part)
    return int(days) * 86400 + total


class ResourceSampler:
    def __init__(self, process_id):
        self.root = process_id
        self.samples = []
        self.errors = []
        self.browser_groups = set()
        self.known = {process_id: 'server_test_host'}
        self.max_cpu = {}
        self.started = time.monotonic()
        self.started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _take(self):
        run = subprocess.run(['ps', '-axo', 'pid=,ppid=,pgid=,time=,rss=,comm='], capture_output=True, text=True, timeout=2, check=True)
        rows = {}
        for line in run.stdout.splitlines():
            fields = line.strip().split(maxsplit=5)
            if len(fields) == 6:
                pid, parent, group, cpu, rss, command = fields
                rows[int(pid)] = dict(pid=int(pid), parent=int(parent), group=int(group), cpu=cpu_seconds(cpu), rss_bytes=int(rss)*1024, command=command)
        owned = {self.root}
        for _ in range(16):
            added = {pid for pid, row in rows.items() if row['parent'] in owned}
            if added.issubset(owned):
                break
            owned.update(added)
        for pid in owned:
            row = rows.get(pid)
            if row and 'chrome' in row['command'].lower():
                self.browser_groups.add(row['group'])
        owned.update(pid for pid, row in rows.items() if row['group'] in self.browser_groups and 'chrome' in row['command'].lower())
        result = []
        for pid in sorted(owned):
            row = rows.get(pid)
            if not row:
                continue
            kind = 'server_test_host' if pid == self.root else ('browser' if 'chrome' in row['command'].lower() else 'control_driver_or_other_owned_child')
            self.known[pid] = kind
            self.max_cpu[pid] = max(self.max_cpu.get(pid, 0), row['cpu'])
            result.append(dict(pid=pid, kind=kind, rss_bytes=row['rss_bytes'], cumulative_cpu_seconds=row['cpu']))
        self.samples.append(dict(elapsed_seconds=time.monotonic()-self.started, processes=result))

    def _run(self):
        while not self.stop.is_set() and len(self.samples) < 125:
            try:
                self._take()
            except Exception as error:
                self.errors.append(type(error).__name__)
            self.stop.wait(1)

    def finish(self, path):
        self.stop.set()
        self.thread.join(timeout=3)
        groups = {}
        for kind in ('server_test_host', 'browser', 'control_driver_or_other_owned_child'):
            peak_rss = max((sum(p['rss_bytes'] for p in sample['processes'] if p['kind'] == kind) for sample in self.samples), default=0)
            groups[kind] = dict(observed_cumulative_cpu_seconds=sum(cpu for pid, cpu in self.max_cpu.items() if self.known[pid] == kind), peak_observed_rss_bytes=peak_rss, observed_process_count=sum(value == kind for value in self.known.values()))
        report = dict(scope='One-second owned-process observations across the whole exact test, including setup/negotiation/measurement/teardown. Server is the Rust test host containing the actual production server; browser descendants/groups are separate. CPU sums maximum observed lifetime CPU per PID; processes exiting between samples may be undercounted. RSS is the maximum sampled group sum, not an allocation trace. No capacity or native-helper claim.', started_at=self.started_at, elapsed_seconds=time.monotonic()-self.started, samples=self.samples, groups=groups, errors=self.errors, native_helper='Not present in this browser/browser topology.')
        Path(path).write_text(json.dumps(report, indent=2)+'\n')
        return dict(valid=not self.errors and len(self.samples) >= 40 and groups['server_test_host']['observed_process_count'] == 1 and groups['browser']['observed_process_count'] > 0, sample_count=len(self.samples), groups=groups, errors=self.errors)
