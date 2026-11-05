"""Private bounded capture of a cell's actual QUIC or WSS/WebRTC sockets."""
import os
import platform
import re
import signal
import socket
import struct
import subprocess
import time

from .transport_matrix import digest


class BrowserCapture:
    def __init__(self, directory, *, interface, server_ip, native, wss, media_start, transport, env):
        self.directory, self.interface, self.server_ip = directory, interface, server_ip
        self.env, self.transport = env, transport
        self.native, self.wss, self.media_start = native, wss, media_start
        self.filter = f"ip and host {server_ip} and " + (f"udp port {native}" if transport == "chrome-webtransport" else
            f"(tcp port {wss} or udp portrange {media_start}-{media_start + 63})")
        self.process = self.stream = self.report = None

    def start(self):
        self.directory.mkdir(mode=0o700)
        self.path, self.log = self.directory / "browser.pcap", self.directory / "capture.log"
        self.binary = "/usr/bin/tcpdump" if platform.system() == "Linux" else "/usr/sbin/tcpdump"
        self.binary_hash = digest(self.binary)
        self.stream = self.log.open("xb")
        self.process = subprocess.Popen([self.binary, "-p", "-n", "-i", self.interface, "-U", "-s", "262144",
            "-c", "200000", "-w", str(self.path), self.filter], env=self.env, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=self.stream, start_new_session=True)
        cutoff = time.monotonic() + 5
        while "listening on " not in self.log.read_text():
            if self.process.poll() is not None or time.monotonic() >= cutoff:
                raise RuntimeError("owned-browser-packet-capture-unavailable")
            time.sleep(.025)

    def finish(self):
        if self.report is not None:
            return self.report
        self.report = dict(format="connect/browser-private-packet-capture/1", transport=self.transport,
            observed=False, original_joined=False, raw_capture_private=True, filter=self.filter)
        try:
            if self.process is None:
                raise ValueError("Capture was not started")
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGINT)
            code = self.process.wait(timeout=10)
            self.report.update(exit_code=code, original_joined=True)
            raw = self.log.read_text()
            captured = re.search(r"(?m)^(\d+) packets captured$", raw)
            dropped = re.search(r"(?m)^(\d+) packets dropped by kernel$", raw)
            if code != 0 or not captured or not dropped or int(dropped[1]) != 0:
                raise ValueError("Capture failed or lost packets")
            if self.path.stat().st_mode & 0o077 or not 24 <= self.path.stat().st_size <= 256 * 1024**2:
                raise ValueError("Capture bounds or privacy differ")
            counts = self.inspect()
            if counts["packets"] != int(captured[1]) or not 0 < counts["packets"] < 200000 or not min(counts["to_server"], counts["from_server"]):
                raise ValueError("Original duplex capture missing or incomplete")
            self.report.update(observed=digest(self.binary) == self.binary_hash,
                executable_sha256=self.binary_hash, pcap_sha256=digest(self.path),
                pcap_bytes=self.path.stat().st_size, kernel_dropped_packets=0, counts=counts)
        except (OSError, ValueError, struct.error, subprocess.SubprocessError) as error:
            self.report.update(failure=type(error).__name__, environment_failure=self.process is None or self.process.returncode != 0)
        finally:
            if self.process is not None and self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
                self.report.update(forced=True, original_joined=True)
            if self.stream:
                self.stream.close()
        return self.report

    def inspect(self):
        counts = dict(packets=0, to_server=0, from_server=0, udp=0, tcp=0)
        server = socket.inet_aton(self.server_ip)
        with self.path.open("rb") as source:
            magic = source.read(4)
            if magic not in {b"\xd4\xc3\xb2\xa1", b"\xa1\xb2\xc3\xd4"}:
                raise ValueError("Unknown pcap format")
            endian = "<" if magic[0] == 0xd4 else ">"
            major, minor, _, _, snaplen, link = struct.unpack(endian + "HHiIII", source.read(20))
            if (major, minor) != (2, 4) or link not in {0, 1}:
                raise ValueError("Unexpected capture link type")
            while header := source.read(16):
                _, _, length, original = struct.unpack(endian + "IIII", header)
                if length != original or not 34 <= length <= snaplen:
                    raise ValueError("Truncated capture")
                body = source.read(length)
                ip = body[4:] if link == 0 else body[14:]
                if len(body) != length or ip[0] >> 4 != 4 or ip[9] not in (6, 17):
                    raise ValueError("Non-profile capture packet")
                if server not in (ip[12:16], ip[16:20]):
                    raise ValueError("Capture includes another endpoint")
                counts["packets"] += 1
                source_port, target_port = struct.unpack_from("!HH", ip, (ip[0] & 15) * 4)
                def server_port(port):
                    return port == self.native if self.transport == "chrome-webtransport" else (
                        port == self.wss if ip[9] == 6 else self.media_start <= port <= self.media_start + 63)
                counts["from_server"] += ip[12:16] == server and server_port(source_port)
                counts["to_server"] += ip[16:20] == server and server_port(target_port)
                counts["udp" if ip[9] == 17 else "tcp"] += 1
                if counts["packets"] > 200000:
                    raise ValueError("Packet count bound")
        return counts
