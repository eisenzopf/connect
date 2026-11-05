"""Capture evidence rejects truncated, foreign and one-direction packet traces."""
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
import os
import subprocess

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_capture import NativeCapture, inspect_pcap


def packet(source, target, payload=b"synthetic-ciphertext", *, address=b"\x7f\x00\x00\x01", destination=b"\x7f\x00\x00\x01", protocol=17):
    udp = struct.pack("!HHHH", source, target, 8 + len(payload), 0) + payload
    ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), 1, 0x4000, 64, protocol, 0,
        address, destination) + udp
    return b"\0" * 12 + b"\x08\x00" + ip


def pcap(packets, endian="<", *, link=1, truncated=False):
    data = struct.pack(endian + "IHHiIII", 0xa1b2c3d4, 2, 4, 0, 0, 262144, link)
    for item in packets:
        data += struct.pack(endian + "IIII", 1000, 17, len(item), len(item) + int(truncated)) + item
    return data


class CaptureBoundary(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.path = Path(temp.name) / "private.pcap"
        self.duplex = [packet(50123, 4433), packet(4433, 50123)]

    def inspect(self, data):
        self.path.write_bytes(data)
        return inspect_pcap(self.path, 4433)

    def test_actual_mac_linux_guest_capture_binds_exact_distinct_private_addresses(self):
        server, client = bytes([192,168,1,104]), bytes([192,168,5,1])
        self.path.write_bytes(pcap([packet(50123,4433,address=client,destination=server),packet(4433,50123,address=server,destination=client)]))
        with self.assertRaises(AssertionError):
            inspect_pcap(self.path,4433,"192.168.1.104","192.168.5.1")
        value=inspect_pcap(self.path,4433,"192.168.1.104","192.168.5.1",mac_linux_guest=True)
        self.assertEqual(value["packets"],2)
        with self.assertRaises(AssertionError):
            inspect_pcap(self.path,4433,"192.168.1.105","192.168.5.1",mac_linux_guest=True)

    def test_both_pcap_byte_orders_count_only_actual_udp_payloads(self):
        for endian in ("<", ">"):
            with self.subTest(endian=endian):
                result = self.inspect(pcap(self.duplex, endian))
                self.assertEqual(result["packets"], 2)
                self.assertEqual(result["to_server_packets"], 1)
                self.assertEqual(result["from_server_packets"], 1)
                self.assertEqual(result["udp_payload_bytes"], 2 * len(b"synthetic-ciphertext"))
                self.assertNotIn("payload", result)

    def test_truncated_header_body_and_snaplen_are_not_capture_proof(self):
        complete = pcap(self.duplex)
        for data in (complete[:10], complete[:-1], complete + b"\0", pcap(self.duplex, truncated=True)):
            with self.subTest(length=len(data)), self.assertRaises(AssertionError):
                self.inspect(data)

    def test_one_direction_or_empty_capture_does_not_prove_duplex(self):
        for packets in ([], [self.duplex[0]], [self.duplex[1]]):
            with self.subTest(packets=len(packets)), self.assertRaisesRegex(AssertionError, "no-duplex"):
                self.inspect(pcap(packets))

    def test_foreign_port_address_and_protocol_refuse_even_with_valid_packets(self):
        for foreign in (packet(50123, 9999), packet(50123, 4433, address=b"\x7f\x00\x00\x02"), packet(50123, 4433, protocol=6)):
            with self.subTest(packet=foreign.hex()), self.assertRaisesRegex(AssertionError, "foreign"):
                self.inspect(pcap([*self.duplex, foreign]))

    def test_media_capture_budget_is_explicit_and_does_not_relax_packet_scope(self):
        self.path.write_bytes(pcap(self.duplex * 4097))
        with self.assertRaisesRegex(AssertionError, "packet-bound"):
            inspect_pcap(self.path, 4433)
        result = inspect_pcap(self.path, 4433, media_workload=True)
        self.assertEqual(result["packets"], 8194)
        self.path.write_bytes(pcap([*self.duplex, packet(50123, 9999)]))
        with self.assertRaisesRegex(AssertionError, "foreign-port"):
            inspect_pcap(self.path, 4433, media_workload=True)

    def test_unknown_link_type_and_file_format_refuse(self):
        for data in (pcap(self.duplex, link=101), b"xxxx" + pcap(self.duplex)[4:]):
            with self.assertRaisesRegex(AssertionError, "capture-(link-profile|format)"):
                self.inspect(data)

    def test_macos_bsd_loopback_headers_require_actual_ipv4_family(self):
        for family in (b"\x02\0\0\0", b"\0\0\0\x02"):
            result = self.inspect(pcap([family + value[14:] for value in self.duplex], link=0))
            self.assertEqual(result["packets"], 2)
            self.assertEqual(result["link_type"], "BSD-loopback/IPv4/UDP")
        with self.assertRaisesRegex(AssertionError, "foreign-loopback"):
            self.inspect(pcap([b"\x1e\0\0\0" + value[14:] for value in self.duplex], link=0))

    def test_fragmented_ip_and_inconsistent_udp_length_refuse(self):
        for offset, value in ((20, b"\x20\x00"), (38, b"\x00\x08")):
            invalid = bytearray(self.duplex[0])
            invalid[offset:offset + 2] = value
            with self.subTest(offset=offset), self.assertRaisesRegex(AssertionError, "fragmented-ip|udp-length"):
                self.inspect(pcap([bytes(invalid), self.duplex[1]]))

    def test_capture_size_and_packet_count_are_bounded(self):
        with self.assertRaisesRegex(AssertionError, "packet-bound"):
            self.inspect(pcap([self.duplex[0]] * 8192 + [self.duplex[1]]))

    def test_remote_capture_requires_the_exact_owned_server_peer_and_direction(self):
        server, client = b"\x0a\xcb\x00\x01", b"\x0a\xcb\x00\x02"
        packets = [packet(50123, 4433, address=client, destination=server),
            packet(4433, 50123, address=server, destination=client)]
        self.path.write_bytes(pcap(packets))
        result = inspect_pcap(self.path, 4433, "10.203.0.1", "10.203.0.2")
        self.assertEqual(result["to_server_packets"], 1)
        self.assertEqual(result["from_server_packets"], 1)
        self.path.write_bytes(pcap([*packets, packet(4433, 50123, address=client, destination=server)]))
        with self.assertRaisesRegex(AssertionError, "foreign-address"):
            inspect_pcap(self.path, 4433, "10.203.0.1", "10.203.0.2")

    def test_unavailable_capture_is_retained_without_raising_or_fabricating_success(self):
        capture = NativeCapture(self.path.parent / "capture", 4433, os.environ.copy())
        with patch.object(capture, "start", side_effect=PermissionError("fixture capture denied")):
            failure = capture.start_observed()
        self.assertFalse(failure["observed"])
        self.assertTrue(failure["environment_failure"])
        self.assertEqual(failure["startup_failure"], "PermissionError")
        self.assertIs(capture.finish(), failure)

    def test_failed_capture_start_joins_its_original_process_before_continuing(self):
        capture = NativeCapture(self.path.parent / "capture", 4433, os.environ.copy())
        owner = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        self.addCleanup(lambda: owner.poll() is None and (owner.kill(), owner.wait()))
        capture.process = owner
        with patch.object(capture, "start", side_effect=AssertionError("capture-not-ready")):
            failure = capture.start_observed()
        self.assertIsNotNone(owner.poll())
        self.assertTrue(failure["original_joined"])
        self.assertFalse(failure["observed"])
        self.assertIs(capture.finish(), failure)


if __name__ == "__main__":
    unittest.main()
