"""Collect a local native workload's owned Linux server/helper observations.

The helper supervisor stays unreaped through every process-group decision.
Collection failures retire the same command; they cannot abandon packet owners.
No process names, command lines, handles, credentials or media are published.
"""
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from .native_process_owner import live_workers
from .operational_process_resources import ProcessResources


def run_profiled(command, log, env, budget, *, server_pid, directory, cwd):
    if not sys.platform.startswith("linux") or not 0 < budget <= 300:
        raise ValueError("Local Linux bounded process profile required")
    directory = Path(directory)
    directory.mkdir(mode=0o700)
    completion = directory / "command-completion.json"
    supervisor = Path(__file__).with_name("phase_supervisor.py")
    began = time.monotonic()
    started_unix_ms = time.time_ns() // 1_000_000
    cutoff = began + budget
    status = None
    owner = sampler = None
    forced = joined = workers_joined = False
    failure = None
    try:
        with Path(log).open("x") as output:
            owner = subprocess.Popen([sys.executable, str(supervisor), str(completion), *command],
                cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=output,
                stderr=subprocess.STDOUT, start_new_session=True)
            sampler = ProcessResources(dict(server=server_pid, helper=owner.pid),
                roles=("server", "helper"), open_resources=True,
                max_samples=math.ceil(budget) + 10)
            next_sample = began
            while True:
                now = time.monotonic()
                if now >= next_sample:
                    sampler.sample()
                    next_sample = now + 1
                if completion.exists():
                    if completion.stat().st_size > 4096:
                        raise ValueError("Owned completion metadata bound")
                    receipt = json.loads(completion.read_text())
                    if set(receipt) != {"exit_code"} or type(receipt["exit_code"]) is not int:
                        raise ValueError("Owned command did not produce a valid exit receipt")
                    status = receipt["exit_code"]
                    break
                if now >= cutoff:
                    status = "timeout"
                    break
                time.sleep(min(.05, max(0, cutoff - now)))
            if status != "timeout":
                # Observation/cleanup only, under one fixed bound. No command
                # or media is restarted and no wait can renew this cutoff.
                cleanup_cutoff = min(cutoff, time.monotonic() + 5)
                while live_workers(owner.pid):
                    if time.monotonic() >= cleanup_cutoff:
                        raise ValueError("Original helper workers did not join")
                    time.sleep(.05)
                workers_joined = True
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        failure = type(error).__name__
        if status is None:
            status = "profile-unavailable"
    except BaseException as error:
        failure = type(error).__name__
        raise
    finally:
        if owner is not None:
            if not workers_joined:
                forced = True
                try:
                    os.killpg(owner.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if not owner.stdin.closed:
                owner.stdin.close()
            try:
                owner.wait(timeout=5)
                joined = True
            except subprocess.TimeoutExpired:
                forced = True
                os.killpg(owner.pid, signal.SIGKILL)
                owner.wait(timeout=5)
                joined = True
            # No further group signals after wait. Start-time checks prevent
            # this final numeric observation from adopting a reused PID.
            if sampler is not None:
                try:
                    sampler.sample()
                except (OSError, ValueError) as error:
                    failure = failure or type(error).__name__
        measured = sampler.summary() if sampler is not None else None
        complete = bool(joined and workers_joined and not forced and failure is None
            and measured is not None and measured["sample_count"] >= 2
            and all(group["unavailable_samples"] == 0
                and group["open_resource_unavailable_samples"] == 0
                for group in measured["groups"].values()))
        result = dict(format="connect/native-workload-process-profile/1",
            started_unix_ms=started_unix_ms, elapsed_seconds=time.monotonic() - began,
            exit_code=status, original_supervisor_joined=joined,
            original_workers_joined=workers_joined, forced=forced, failure=failure,
            collection_complete=complete, observation=measured,
            scope="One-second numeric observations of the original local Linux server and native workload command, including its Node controller, native SDK helpers, independent physical peers and the small Python supervisor. Includes setup and teardown; not a media-only CPU measurement. Sampled descendants can miss very short-lived children. RSS can double-count shared pages. Open descriptor counts expose no targets. Browser is absent. No capacity, resource-reclamation or complete-cell claim.",
            resource_reclamation_qualified=False, cell_qualified=False, task_complete=False)
        (directory / "process-resources.json").write_text(json.dumps(result, indent=2) + "\n")
    return status, dict(format=result["format"], started_unix_ms=started_unix_ms,
        elapsed_seconds=result["elapsed_seconds"],
        sample_count=measured["sample_count"] if measured is not None else 0,
        groups=measured["groups"] if measured is not None else {},
        original_supervisor_joined=joined, original_workers_joined=workers_joined,
        forced=forced, failure=failure, collection_complete=complete, scope=result["scope"], resource_reclamation_qualified=False,
        cell_qualified=False, task_complete=False)
