"""Execute the real cleanup branch against an owned interface/capture model.

The pcap validator and its strict packet/drop rules are real. The process and
network effects are local fakes; these tests are not packet-capture evidence.
"""
import ast
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
import uuid
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.matrix_browser_capture import BrowserCapture
from lib.transport_matrix import digest
from lib.matrix_storage import write_component
from lib.matrix_plateau import POLICY as PLATEAU_POLICY, evaluate as evaluate_plateau, runtime_capacity_configuration

RUNNER = Path(__file__).resolve().parents[1] / "rehearse-gate6-browser-matrix.py"


def cleanup_program(*, complete=False):
    main = next(node for node in ast.parse(RUNNER.read_text()).body
                if isinstance(node, ast.FunctionDef) and node.name == "main")
    original = next(node for node in main.body if isinstance(node, ast.Try) and node.finalbody)
    # Run the actual original cleanup through log-handle closure. The following
    # source-hash/report serialization is outside this resource-owner boundary.
    last = next(index for index, node in enumerate(original.finalbody)
                if isinstance(node, ast.For) and isinstance(node.target, ast.Name) and node.target.id == "handle")
    # The extracted finally block runs in the same resource-state scope as
    # production main. Reuse its actual pure declarations, rather than inventing
    # cycle defaults or bypassing the three-cycle missing-evidence refusal.
    required = {"idle_observers", "idle_results", "cycle_count", "cycle_workloads"}
    initializers, found = [], set()
    for node in main.body[:main.body.index(original)]:
        if not isinstance(node, ast.Assign):
            continue
        names = {value.id for target in node.targets for value in ast.walk(target)
                 if isinstance(value, ast.Name)}
        if names & required:
            if not names <= required:
                raise AssertionError("Resource declaration includes an unmodeled owner")
            initializers.append(node)
            found.update(names)
    if found != required:
        raise AssertionError("Original pre-cleanup resource declarations changed")
    helpers = [node for node in ast.parse(RUNNER.read_text()).body
               if isinstance(node, ast.FunctionDef) and node.name in
               {"record_cleanup_failure", "close_original_group"}]
    if len(helpers) != 2:
        raise AssertionError("Original cleanup declarations missing")
    body = original.finalbody if complete else original.finalbody[:last + 1]
    return compile(ast.Module(body=helpers + initializers + body, type_ignores=[]), str(RUNNER), "exec")


def pcap(transport, *, empty=False, truncated=False, foreign=False):
    data = struct.pack("<IHHiIII", 0xa1b2c3d4, 2, 4, 0, 0, 262144, 1)
    if empty:
        return data
    server, client = socket.inet_aton("10.203.0.99" if foreign else "10.203.0.1"), socket.inet_aton("10.203.0.2")
    port, protocol = (4433, 17) if transport == "chrome-webtransport" else (8443, 6)
    for source, target, sport, dport in [(client, server, 50000, port), (server, client, port, 50000)]:
        body = b"\0" * 12 + b"\x08\0" + struct.pack("!BBHHHBBH4s4s", 0x45, 0, 28, 1, 0, 64, protocol, 0, source, target)
        body += struct.pack("!HHHH", sport, dport, 8, 0)
        data += struct.pack("<IIII", 1, 0, len(body), len(body) + int(truncated)) + body
    return data


class CaptureCleanup(unittest.TestCase):
    def execute(self, *, transport="chrome-webtransport", empty=False, dropped=0, truncated=False, started=True, resource_cycles=1, captured_pairs=1, foreign=False):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            events = []
            interface = SimpleNamespace(alive=True)
            capture = BrowserCapture(directory, interface="br-original", server_ip="10.203.0.1",
                native=4433, wss=8443, media_start=40000, transport=transport, env={}, resource_cycles=resource_cycles)
            capture.path, capture.log = directory / "browser.pcap", directory / "capture.log"
            raw = pcap(transport, empty=empty, truncated=truncated, foreign=foreign)
            capture.path.write_bytes(raw[:24] + raw[24:] * captured_pairs); capture.path.chmod(0o600)
            capture.binary = directory / "fixture-binary"; capture.binary.write_bytes(b"synthetic capture owner")
            capture.binary_hash = digest(capture.binary)
            capture.log.write_text("")

            class CaptureProcess:
                pid = 31337
                returncode = None

                def poll(self):
                    return self.returncode

                def terminate(self, code):
                    self.returncode = code
                    count = 0 if empty else 2 * captured_pairs
                    capture.log.write_text(("tcpdump: pcap_loop: The interface disappeared\n" if code else "") +
                        f"{count} packets captured\n{count} packets received by filter\n{dropped} packets dropped by kernel\n")

                def wait(self, timeout):
                    self.assert_terminal()
                    events.append("capture-joined")
                    return self.returncode

                def assert_terminal(self):
                    if self.returncode is None:
                        raise AssertionError("Capture owner was not asked to stop")

            process = CaptureProcess()
            capture.process = process if started else None

            def killpg(pid, requested):
                if pid == 31338 and requested == 0:
                    raise ProcessLookupError
                self.assertEqual(pid, process.pid); self.assertEqual(requested, signal.SIGINT)
                process.terminate(0 if interface.alive else 1)

            def remove_network(command, **options):
                self.assertEqual(command, ["docker", "network", "rm", "original-network"])
                self.assertEqual(options, dict(capture_output=True, timeout=15))
                events.append("network-removed"); interface.alive = False
                if started and process.returncode is None:
                    process.terminate(1)
                return SimpleNamespace(returncode=0)

            class TrafficOwner:
                pid = 31338

                def poll(self):
                    return 0

                def wait(self, timeout):
                    events.append("traffic-joined")
                    return 0

            report = dict(passed=True)
            scope = dict(remote=None, browser_owner=TrafficOwner(), network_name="original-network",
                capture=capture, idle=SimpleNamespace(finish=lambda: dict(passed=True)),
                args=SimpleNamespace(stage="quality", transport=transport, resource_cycles=resource_cycles), report=report,
                stop_server=lambda: events.append("server-joined"), server=None, log_handles=[],
                subprocess=subprocess, signal=signal, uuid=uuid, json=json, os=os, time=time,
                PLATEAU_POLICY=PLATEAU_POLICY, evaluate_plateau=evaluate_plateau,
                runtime_capacity_configuration=runtime_capacity_configuration)
            with patch("lib.matrix_browser_capture.os.killpg", side_effect=killpg), patch.object(subprocess, "run", side_effect=remove_network):
                exec(cleanup_program(), scope)
            return report, events

    def test_original_duplex_capture_joins_before_its_interface_is_removed(self):
        for resource_cycles in (1, 3):
            for transport in ("chrome-webtransport", "chrome-wss-webrtc", "firefox-wss-webrtc"):
                with self.subTest(transport=transport, resource_cycles=resource_cycles):
                    report, events = self.execute(transport=transport, resource_cycles=resource_cycles)
                    self.assertTrue(report["packet_capture"]["observed"])
                    self.assertEqual(report["packet_capture"]["counts"]["packets"], 2)
                    self.assertEqual(report["packet_capture"]["kernel_dropped_packets"], 0)
                    self.assertEqual(events, ["traffic-joined", "capture-joined", "network-removed", "server-joined"])
                    self.assertTrue(report["browser_owner_joined"]); self.assertTrue(report["client_network_removed"])
                    if resource_cycles == 1:
                        self.assertTrue(report["passed"])
                        self.assertEqual(report["idle_resources"], dict(passed=True))
                    else:
                        # Capture and cleanup facts survive independently. This
                        # model ran no three-cycle media and cannot qualify it.
                        self.assertFalse(report["passed"])
                        idle = report["idle_resources"]
                        self.assertEqual(idle["format"], "connect/matrix-plateau-observation/1")
                        self.assertEqual(idle["policy"], PLATEAU_POLICY)
                        self.assertFalse(idle["passed"])
                        self.assertEqual(idle["cycles"], [])
                        ids = idle["original_cycle_ids"]
                        self.assertEqual([row["index"] for row in ids], [1, 2, 3])
                        self.assertEqual(len({row["workload_id"] for row in ids}), 3)

    def test_rejected_capture_keeps_failure_and_still_joins_fixture_owners(self):
        for resource_cycles in (1, 3):
            for options in (dict(empty=True), dict(dropped=1), dict(truncated=True), dict(foreign=True)):
                with self.subTest(options=options, resource_cycles=resource_cycles):
                    report, events = self.execute(resource_cycles=resource_cycles, **options)
                    self.assertFalse(report["passed"]); self.assertFalse(report["packet_capture"]["observed"])
                    self.assertTrue(report["packet_capture"]["original_joined"])
                    self.assertTrue(report["client_network_removed"])
                    self.assertEqual(events, ["traffic-joined", "capture-joined", "network-removed", "server-joined"])

        for stage in ("browser", "capture", "network", "idle", "server", "log", "unconfirmed"):
            with self.subTest(cleanup_failure=stage):
                self.assert_exceptional_report({stage})
        with self.subTest(cleanup_failure="all-original-owners"):
            self.assert_exceptional_report({"browser", "capture", "network", "idle", "server", "log"})
        self.assert_actual_original_group_retirement()

    def assert_exceptional_report(self, failed_stages):
        """Execute the complete real finally, including its real atomic report."""
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            events, waits = [], []
            primary = dict(phase="actual-browser-quality", name="ValueError", line=173)
            report = dict(passed=False, failure=dict(primary), servers=[])

            class Owner:
                def __init__(self, pid, alive):
                    self.pid, self.returncode = pid, None if alive else 0

                def poll(self):
                    return self.returncode

                def wait(self, timeout):
                    waits.append((self.pid, timeout))
                    events.append("traffic-joined" if self.pid == 424242 else "fallback-joined")
                    if self.returncode is None:
                        raise subprocess.TimeoutExpired(["original-owner"], timeout)
                    return self.returncode

            browser = Owner(424242, "browser" in failed_stages)
            report["browser_owner_identity"] = dict(original_pid=browser.pid,
                original_pgid=None, group_observation_error="PermissionError")
            capture_owner = Owner(424244, "capture" in failed_stages)
            server_owner = Owner(424243, "server" in failed_stages)
            owners = {owner.pid: owner for owner in (browser, capture_owner, server_owner)}

            def killpg(pid, requested):
                if pid == browser.pid and "unconfirmed" in failed_stages:
                    raise PermissionError("Original group observation unavailable")
                owner = owners[pid]
                if requested == 0:
                    if owner.returncode is not None:
                        raise ProcessLookupError
                    return
                events.append("original-signal-" + str(requested))
                # Deterministic real TimeoutExpired path: TERM cannot complete
                # this modeled owner; only the SAME group's KILL changes it.
                if requested == signal.SIGKILL:
                    owner.returncode = -signal.SIGKILL

            class Capture:
                process = capture_owner if "capture" in failed_stages else None
                stream = None

                def finish(self):
                    events.append("capture")
                    if "capture" in failed_stages:
                        raise OSError("original capture close failed")
                    return dict(observed=True, original_joined=True)

            class Idle:
                def finish(self):
                    events.append("idle")
                    if "idle" in failed_stages:
                        raise OSError("original sampler metadata failed")
                    return dict(passed=True)

            class Handle:
                def __init__(self, index):
                    self.index = index

                def close(self):
                    events.append("log-" + str(self.index))
                    if self.index == 1 and "log" in failed_stages:
                        raise OSError("original log close failed")

            def remove_network(argv, **kwargs):
                events.append("network")
                self.assertEqual(argv, ["docker", "network", "rm", "original-network"])
                self.assertEqual(kwargs, dict(capture_output=True, timeout=15))
                if "network" in failed_stages:
                    raise subprocess.TimeoutExpired(argv, 15)
                return SimpleNamespace(returncode=0)

            def stop_server():
                events.append("server")
                if "server" in failed_stages:
                    raise OSError("original server close failed")

            def persist(path, value, *, reserve):
                events.append("component")
                write_component(path, value, reserve=reserve)

            executable = directory / "original-connect"
            executable.write_bytes(b"owned fixture executable")
            report["binary_sha256"] = digest(executable)
            scope = dict(remote=None, browser_owner=browser, linux=False,
                cidfile=directory / "absent-container", network_name="original-network",
                capture=Capture(), idle=Idle(), server=server_owner if "server" in failed_stages else None,
                args=SimpleNamespace(stage="quality", transport="firefox-wss-webrtc",
                                     resource_cycles=1, rtc_source_frame_witness=False),
                report=report, stop_server=stop_server, log_handles=[Handle(1), Handle(2)],
                subprocess=subprocess, signal=signal, uuid=uuid, json=json, os=os, time=time,
                PLATEAU_POLICY=PLATEAU_POLICY, evaluate_plateau=evaluate_plateau,
                runtime_capacity_configuration=runtime_capacity_configuration,
                before={"original": 1}, sources=lambda: {"original": 1},
                sdk_before={"original-sdk": 1}, sdk_inputs=lambda: {"original-sdk": 1},
                executable=executable, digest=digest, output=directory, component_reserve=None,
                state={}, save=lambda: events.append("state"), write_component=persist)
            with patch.object(os, "killpg", side_effect=killpg), patch.object(subprocess, "run", side_effect=remove_network):
                exec(cleanup_program(complete=True), scope)
            saved = json.loads((directory / "component.json").read_text())
            self.assertEqual(saved["failure"], primary)
            self.assertFalse(saved["passed"])
            self.assertTrue(saved["source_inputs_unchanged"])
            self.assertTrue(saved["sdk_dist_inputs_unchanged"])
            self.assertTrue(saved["executable_unchanged"])
            for label in ("traffic-joined", "capture", "network", "idle", "server", "log-1", "log-2", "component", "state"):
                self.assertIn(label, events)
            self.assertLess(events.index("capture"), events.index("network"))
            self.assertLess(events.index("server"), events.index("component"))
            self.assertTrue(saved["browser_owner_identity"]["wait_returned"])
            self.assertIsNone(saved["browser_owner_identity"]["original_pgid"])
            self.assertEqual(saved["browser_owner_identity"]["group_observation_error"], "PermissionError")
            self.assertEqual(saved["browser_owner_identity"]["owned_pgid"], browser.pid)
            self.assertEqual(saved["browser_owner_identity"]["group_identity_basis"], "original-Popen-start-new-session")
            self.assertEqual(saved["browser_owner_identity"]["original_process_group_joined"],
                             "unconfirmed" not in failed_stages)
            self.assertEqual(saved["browser_owner_joined"], "unconfirmed" not in failed_stages)
            self.assertEqual(saved["browser_owner_identity"]["original_pid"], browser.pid)
            if "browser" in failed_stages:
                self.assertEqual(saved["browser_exit_code"], -signal.SIGKILL)
                self.assertTrue(saved["browser_forced_cleanup"])
                self.assertTrue(saved["browser_owner_identity"]["forced"])
            self.assertTrue(all(0 <= timeout <= (10 if pid == browser.pid else 5) for pid, timeout in waits))
            self.assertTrue(saved["cleanup_failures"])

    def assert_actual_original_group_retirement(self):
        # Real Popen/PGID custody, no browser/media claim. This must send TERM
        # before waiting; the old branch waited on this sleeping owner forever.
        module = ast.parse(RUNNER.read_text())
        helper = next(node for node in module.body if isinstance(node, ast.FunctionDef)
                      and node.name == "close_original_group")
        scope = dict(os=os, signal=signal, subprocess=subprocess, time=time)
        exec(compile(ast.Module(body=[helper], type_ignores=[]), str(RUNNER), "exec"), scope)
        child = subprocess.Popen([sys.executable, "-c", "import time; print('ready',flush=True); time.sleep(30)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            start_new_session=True, text=True)
        try:
            self.assertEqual(child.stdout.readline(), "ready\n")
            receipt = scope["close_original_group"](child, timeout=10)
            self.assertTrue(receipt["interrupted"])
            self.assertTrue(receipt["wait_returned"])
            self.assertTrue(receipt["original_joined"])
            self.assertEqual(receipt["original_exit_code"], -signal.SIGTERM)
            self.assertFalse(receipt["forced"])
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)
            child.stdout.close()

        # A waited leader does not prove its original group is gone. The SAME
        # group is signalled even when wait() already returned a natural zero.
        owner = SimpleNamespace(pid=424245, poll=lambda: 0, wait=lambda timeout: 0)
        alive, requested = [True], []
        def original_group(pid, requested_signal):
            self.assertEqual(pid, owner.pid)
            if requested_signal == 0:
                if not alive[0]:
                    raise ProcessLookupError
            else:
                requested.append(requested_signal)
                alive[0] = False
        with patch.object(os, "killpg", side_effect=original_group):
            receipt = scope["close_original_group"](owner, timeout=10)
        self.assertEqual(requested, [signal.SIGTERM])
        self.assertTrue(receipt["original_joined"])
        self.assertTrue(receipt["interrupted"])

    def test_three_cycle_capture_scales_only_aggregate_packet_evidence_budget(self):
        for resource_cycles, pairs, observed in ((1, 100000, False), (3, 100000, True), (3, 300000, False)):
            with self.subTest(resource_cycles=resource_cycles, packets=2*pairs):
                report, events = self.execute(resource_cycles=resource_cycles, captured_pairs=pairs)
                capture = report["packet_capture"]
                self.assertEqual(capture["observed"], observed)
                self.assertEqual(capture["packet_limit"], 200000*resource_cycles)
                self.assertEqual(capture["maximum_pcap_bytes"], 256*1024**2*resource_cycles)
                self.assertEqual(capture["original_packets_captured"], 2*pairs)
                self.assertEqual(capture["kernel_dropped_packets"], 0)
                self.assertTrue(capture["original_joined"])
                self.assertEqual(events, ["traffic-joined", "capture-joined", "network-removed", "server-joined"])
                if resource_cycles == 3:
                    self.assertFalse(report["passed"], "Capture cannot qualify missing real three-cycle media evidence")
        for invalid in (0, 2, 4, True, "3"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                BrowserCapture(Path("unused"), interface="lo", server_ip="127.0.0.1", native=4433,
                    wss=8443, media_start=40000, transport="chrome-webtransport", env={}, resource_cycles=invalid)

    def test_unstarted_capture_never_becomes_proof_and_still_removes_owned_network(self):
        for resource_cycles in (1, 3):
            with self.subTest(resource_cycles=resource_cycles):
                report, events = self.execute(started=False, resource_cycles=resource_cycles)
                self.assertFalse(report["passed"]); self.assertFalse(report["packet_capture"]["observed"])
                self.assertFalse(report["packet_capture"]["owner_created"])
                self.assertTrue(report["client_network_removed"])
                self.assertEqual(events, ["traffic-joined", "network-removed", "server-joined"])


if __name__ == "__main__":
    unittest.main()
