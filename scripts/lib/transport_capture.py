"""Owned Linux-loopback packet capture for a single synthetic native fixture."""
import hashlib
import ipaddress
import os
from pathlib import Path
import platform
import re
import signal
import socket
import struct
import subprocess
import time


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inspect_pcap(path, server_port, server_ip="127.0.0.1", peer_ip="127.0.0.1", *, media_workload=False, mac_linux_guest=False):
    """Validate every captured Ethernet/IPv4/UDP record, without emitting bodies."""
    assert type(media_workload) is bool
    assert type(server_port) is int and 1 <= server_port <= 65535
    assert type(mac_linux_guest) is bool
    assert (mac_linux_guest and server_ip != peer_ip and all(ipaddress.ip_address(ip).is_private and not ipaddress.ip_address(ip).is_loopback for ip in (server_ip, peer_ip))) or (server_ip, peer_ip) in {("127.0.0.1", "127.0.0.1"), ("10.203.0.1", "10.203.0.2")}
    server_address, peer_address = socket.inet_aton(server_ip), socket.inet_aton(peer_ip)
    packet_limit = 200000 if media_workload else 8192
    maximum_bytes = (256 if media_workload else 64) * 1024 * 1024
    assert 24 <= path.stat().st_size <= maximum_bytes
    counts = dict(packets=0, to_server_packets=0, from_server_packets=0,
        captured_bytes=0, udp_payload_bytes=0, to_server_udp_bytes=0, from_server_udp_bytes=0)
    with path.open("rb") as stream:
        magic = stream.read(4)
        assert magic in {b"\xd4\xc3\xb2\xa1", b"\xa1\xb2\xc3\xd4"}, "capture-format"
        endian = "<" if magic[0] == 0xd4 else ">"
        major, minor, zone, accuracy, snaplen, link = struct.unpack(endian + "HHiIII", stream.read(20))
        assert (major, minor, zone, accuracy) == (2, 4, 0, 0) and link in {0, 1}, "capture-link-profile"
        counts["link_type"] = "Ethernet/IPv4/UDP" if link == 1 else "BSD-loopback/IPv4/UDP"
        assert 65550 <= snaplen <= 262144
        while header := stream.read(16):
            assert len(header) == 16, "capture-truncated-record"
            seconds, micros, size, original = struct.unpack(endian + "IIII", header)
            assert micros < 1000000 and 14 <= size == original <= snaplen, "capture-truncated-packet"
            packet = stream.read(size)
            assert len(packet) == size, "capture-truncated-body"
            assert seconds > 0, "capture-invalid-timestamp"
            if link == 1:
                assert packet[12:14] == b"\x08\x00", "capture-foreign-ethernet"
                ip = packet[14:]
            else:
                assert packet[:4] in {b"\x02\x00\x00\x00", b"\x00\x00\x00\x02"}, "capture-foreign-loopback"
                ip = packet[4:]
            assert len(ip) >= 28 and ip[0] >> 4 == 4 and ip[9] == 17, "capture-foreign-protocol"
            ihl = (ip[0] & 15) * 4
            length = int.from_bytes(ip[2:4], "big")
            assert 20 <= ihl <= 60 and ihl + 8 <= length <= len(ip), "capture-ip-length"
            assert int.from_bytes(ip[6:8], "big") & 0x3fff == 0, "capture-fragmented-ip"
            udp = ip[ihl:length]
            source, target, udp_length = struct.unpack("!HHH", udp[:6])
            assert len(udp) == udp_length >= 8, "capture-udp-length"
            assert (source == server_port) != (target == server_port), "capture-foreign-port"
            direction = "to_server" if target == server_port else "from_server"
            addresses = (peer_address, server_address) if direction == "to_server" else (server_address, peer_address)
            assert (ip[12:16], ip[16:20]) == addresses, "capture-foreign-address"
            counts[direction + "_packets"] += 1
            counts[direction + "_udp_bytes"] += len(udp) - 8
            counts["packets"] += 1
            counts["captured_bytes"] += size
            counts["udp_payload_bytes"] += len(udp) - 8
            assert counts["packets"] <= packet_limit, "capture-packet-bound"
    assert counts["to_server_packets"] > 0 and counts["from_server_packets"] > 0, "capture-no-duplex-traffic"
    return counts


class NativeCapture:
    """The caller owns this process and must finish it on success and failure."""
    def __init__(self, directory, server_port, env, *, interface=None, server_ip="127.0.0.1", peer_ip="127.0.0.1", media_workload=False, mac_linux_guest=False):
        assert type(media_workload) is bool
        self.media_workload = media_workload
        self.mac_linux_guest = mac_linux_guest
        self.packet_limit = 200000 if media_workload else 8192
        self.directory = directory
        self.port = server_port
        self.env = env
        self.process = None
        self.stream = None
        self.report = None
        self.interface = interface
        self.server_ip, self.peer_ip = server_ip, peer_ip

    def start(self):
        assert type(self.port) is int and 1 <= self.port <= 65535
        self.directory.mkdir(mode=0o700)
        self.path = self.directory / "native.pcap"
        self.log = self.directory / "capture.log"
        assert platform.system() in {"Linux", "Darwin"}
        if self.interface is None:
            assert self.server_ip == self.peer_ip == "127.0.0.1"
            self.interface = "lo" if platform.system() == "Linux" else "lo0"
        elif self.mac_linux_guest:
            assert platform.system() == "Linux" and re.fullmatch(r"(?:eth|en)[a-z0-9]{1,12}", self.interface)
            assert self.server_ip != self.peer_ip and all(ipaddress.ip_address(ip).is_private and not ipaddress.ip_address(ip).is_loopback for ip in (self.server_ip,self.peer_ip))
        else:
            assert platform.system() == "Linux" and re.fullmatch(r"g6th[0-9a-f]{10}", self.interface)
            assert (self.server_ip, self.peer_ip) == ("10.203.0.1", "10.203.0.2")
        binary = Path("/usr/bin/tcpdump" if platform.system() == "Linux" else "/usr/sbin/tcpdump")
        metadata = binary.stat()
        assert metadata.st_uid == 0 and metadata.st_mode & 0o022 == 0
        version = subprocess.check_output([str(binary), "--version"], env=self.env, timeout=5, text=True).splitlines()[0]
        assert re.fullmatch(r"tcpdump version [0-9]+\.[0-9]+\.[0-9]+(?: -- Apple version [0-9]+)?", version)
        self.binary = binary
        self.binary_sha256 = digest(binary)
        self.version = version
        self.stream = self.log.open("xb")
        self.started = time.monotonic()
        self.process = subprocess.Popen([str(binary), "-p", "-n", "-i", self.interface, "-U", "-s", "262144",
            "-c", str(self.packet_limit), "-w", str(self.path), "udp", "and", "host", self.server_ip, "and", "host", self.peer_ip, "and", "port", str(self.port)],
            env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=self.stream, start_new_session=True)
        until = time.monotonic() + 10
        while ("listening on " + self.interface + ",").encode() not in self.log.read_bytes():
            assert self.process.poll() is None and time.monotonic() < until, "capture-not-ready"
            time.sleep(.05)
        assert self.path.is_file() and self.path.stat().st_mode & 0o077 == 0
        assert digest(binary) == self.binary_sha256

    def start_observed(self):
        """Retain a failed capture attempt without suppressing other checks.

        The original capture owner is joined immediately on startup failure.
        finish() then returns the same negative observation throughout cleanup.
        The caller still requires observed=True for complete-cell acceptance.
        """
        try:
            self.start()
        except (AssertionError, OSError, ValueError, subprocess.SubprocessError) as error:
            report = self.finish()
            report.update(environment_failure=True, startup_failure=type(error).__name__)
            return report
        return None

    def finish(self):
        if self.report is not None:
            return self.report
        report = dict(format="connect/native-private-packet-capture/1", observed=False, owner_created=self.process is not None,
            original_joined=False, forced=False, scope="Actual owned native fixture UDP capture only; ciphertext is not decoded media and does not qualify a full transport cell.")
        self.report = report
        try:
            assert self.process is not None
            running = self.process.poll() is None
            if running:
                try:
                    os.killpg(self.process.pid, signal.SIGINT)
                except ProcessLookupError:
                    pass
            try:
                code = self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                report["forced"] = True
                os.killpg(self.process.pid, signal.SIGKILL)
                code = self.process.wait(timeout=5)
            report["exit_code"] = code
            try:
                os.killpg(self.process.pid, 0)
            except ProcessLookupError:
                report["original_joined"] = True
            assert running and code == 0 and report["original_joined"] and not report["forced"], "capture-owner-not-joined"
            assert digest(self.binary) == self.binary_sha256, "capture-executable-changed"
            raw = self.log.read_text()
            captured = re.search(r"(?m)^(\d+) packets captured$", raw)
            dropped = re.search(r"(?m)^(\d+) packets dropped by kernel$", raw)
            assert captured and dropped and int(dropped[1]) == 0, "capture-loss-or-incomplete-status"
            counts = inspect_pcap(self.path, self.port, self.server_ip, self.peer_ip, media_workload=self.media_workload, mac_linux_guest=self.mac_linux_guest)
            assert counts["packets"] < self.packet_limit, "capture-packet-limit-reached"
            assert counts["packets"] == int(captured[1]), "capture-count-disagreement"
            link_type = counts.pop("link_type")
            report.update(observed=True, packet_limit=self.packet_limit, media_workload=self.media_workload, version=self.version, executable_sha256=self.binary_sha256,
                pcap_sha256=digest(self.path), pcap_bytes=self.path.stat().st_size,
                captured_seconds=time.monotonic() - self.started, kernel_dropped_packets=0,
                interface=self.interface if self.server_ip == "127.0.0.1" else "owned-veth", link_type=link_type,
                filter="udp and exact owned fixture server/peer addresses and native server port",
                counts=counts, raw_capture_private=True, decoded_media_verified=False)
        except (AssertionError, OSError, ValueError, struct.error, subprocess.TimeoutExpired) as error:
            report["failure"] = str(error) if isinstance(error, AssertionError) else type(error).__name__
        finally:
            if self.stream is not None:
                self.stream.close()
        return report
