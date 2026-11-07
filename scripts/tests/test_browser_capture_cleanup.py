"""Execute the real cleanup branch against an owned interface/capture model.

The pcap validator and its strict packet/drop rules are real. The process and
network effects are local fakes; these tests are not packet-capture evidence.
"""
import ast
import json
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
import uuid
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.matrix_browser_capture import BrowserCapture
from lib.transport_matrix import digest
from lib.matrix_plateau import POLICY as PLATEAU_POLICY, evaluate as evaluate_plateau

RUNNER = Path(__file__).resolve().parents[1] / "rehearse-gate6-browser-matrix.py"


def cleanup_program():
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
    return compile(ast.Module(body=initializers + original.finalbody[:last + 1], type_ignores=[]), str(RUNNER), "exec")


def pcap(transport, *, empty=False, truncated=False):
    data = struct.pack("<IHHiIII", 0xa1b2c3d4, 2, 4, 0, 0, 262144, 1)
    if empty:
        return data
    server, client = socket.inet_aton("10.203.0.1"), socket.inet_aton("10.203.0.2")
    port, protocol = (4433, 17) if transport == "chrome-webtransport" else (8443, 6)
    for source, target, sport, dport in [(client, server, 50000, port), (server, client, port, 50000)]:
        body = b"\0" * 12 + b"\x08\0" + struct.pack("!BBHHHBBH4s4s", 0x45, 0, 28, 1, 0, 64, protocol, 0, source, target)
        body += struct.pack("!HHHH", sport, dport, 8, 0)
        data += struct.pack("<IIII", 1, 0, len(body), len(body) + int(truncated)) + body
    return data


class CaptureCleanup(unittest.TestCase):
    def execute(self, *, transport="chrome-webtransport", empty=False, dropped=0, truncated=False, started=True, resource_cycles=1):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            events = []
            interface = SimpleNamespace(alive=True)
            capture = BrowserCapture(directory, interface="br-original", server_ip="10.203.0.1",
                native=4433, wss=8443, media_start=40000, transport=transport, env={})
            capture.path, capture.log = directory / "browser.pcap", directory / "capture.log"
            capture.path.write_bytes(pcap(transport, empty=empty, truncated=truncated)); capture.path.chmod(0o600)
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
                    count = 0 if empty else 2
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
                subprocess=subprocess, signal=signal, uuid=uuid, json=json,
                PLATEAU_POLICY=PLATEAU_POLICY, evaluate_plateau=evaluate_plateau)
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
        for options in (dict(empty=True), dict(dropped=1), dict(truncated=True)):
            with self.subTest(options=options):
                report, events = self.execute(**options)
                self.assertFalse(report["passed"]); self.assertFalse(report["packet_capture"]["observed"])
                self.assertTrue(report["packet_capture"]["original_joined"])
                self.assertTrue(report["client_network_removed"])
                self.assertEqual(events, ["traffic-joined", "capture-joined", "network-removed", "server-joined"])

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
