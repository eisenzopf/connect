"""Capture evidence rejects truncated, foreign and one-direction packet traces."""
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.transport_capture import inspect_pcap


def packet(source, target, payload=b"synthetic-ciphertext", *, address=b"\x7f\x00\x00\x01", protocol=17):
    udp = struct.pack("!HHHH", source, target, 8 + len(payload), 0) + payload
    ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), 1, 0x4000, 64, protocol, 0,
        address, b"\x7f\x00\x00\x01") + udp
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


if __name__ == "__main__":
    unittest.main()
