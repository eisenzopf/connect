#!/usr/bin/env python3
"""Bounded synthetic SIP/RTP peer in a fresh owned Linux container."""
import hashlib
import json
import ipaddress
from pathlib import Path
import select
import platform
import socket
import struct
import sys
import time
from urllib.parse import unquote



def designated_gateway(spec):
    # The existing Lima profile stays explicit. The Linux CI owner supplies the
    # verified fresh bridge gateway as both signaling hop and advertised host.
    if spec.get("owned_internal_bridge") is True:
        gateway = ipaddress.IPv4Address(spec["gateway"])
        assert str(gateway) == spec["host_media_ip"] and gateway.is_private
        assert not (gateway.is_loopback or gateway.is_unspecified or gateway.is_multicast or gateway.is_link_local)
    else:
        assert spec["gateway"] == "192.168.5.2"
    return spec["gateway"]

def main():
    directory = Path(sys.argv[1])
    spec = json.loads((directory / "guest-input.json").read_text())
    designated_gateway(spec)
    assert len(spec["nonce"]) == 64
    wire = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    media = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    report = {"passed": False, "os": sys.platform, "architecture": platform.machine(), "sip_invites": 0, "sip_acks": 0,
              "sip_byes": 0, "sent_rtp": 0, "received_rtp": 0, "foreign_datagrams": 0}
    calls = set()
    received = bytearray()
    try:
        for channel, port, name in [(wire, spec["sip_front"], "sip"), (media, spec["rtp_front"], "rtp")]:
            channel.bind(("0.0.0.0", 0))
            channel.settimeout(.2)
            until = time.monotonic() + 10
            while True:
                channel.sendto(f'CONNECT-G6-PHONE {spec["nonce"]} {name}'.encode(), (spec["gateway"], port))
                try:
                    data, source = channel.recvfrom(256)
                    if data == b"mapped" and source == (spec["gateway"], port):
                        break
                except TimeoutError:
                    pass
                assert time.monotonic() < until, "owned UDP mapping deadline"
            channel.setblocking(False)
        (directory / "guest-ready").write_text("mapped\n")
        wave = (directory / "guest-send.pcmu").read_bytes()
        assert len(wave) == 200 * 160
        media_target = None
        began = None
        next_frame = 0
        until = time.monotonic() + 100
        while True:
            assert time.monotonic() < until, "synthetic telephone deadline"
            assert not (directory / "host-stopped").exists(), "owned host ended before signed BYE"
            if began is None and report["sip_acks"] and (directory / "media-go").exists():
                began = time.monotonic()
            now = time.monotonic()
            if began is not None and next_frame < 200 and now >= began + next_frame * .02:
                packet = struct.pack("!BBHII", 0x80, 0, next_frame + 1, next_frame * 160, 0x19374621)
                packet += wave[next_frame * 160:(next_frame + 1) * 160]
                assert media_target is not None
                media.sendto(packet, (spec["gateway"], spec["rtp_front"]))
                next_frame += 1
                report["sent_rtp"] += 1
            for channel in select.select([wire, media], [], [], .005)[0]:
                data, source = channel.recvfrom(16384)
                expected = (spec["gateway"], spec["sip_front"] if channel is wire else spec["rtp_front"])
                if source != expected:
                    report["foreign_datagrams"] += 1
                    continue
                if channel is media:
                    if began is None:
                        continue
                    if len(data) >= 4 and data[0] >> 6 == 2 and 192 <= data[1] <= 223:
                        report["rtcp_on_rtp_socket"] = report.get("rtcp_on_rtp_socket", 0) + 1
                        offset = 0
                        types = []
                        while offset < len(data):
                            assert offset + 4 <= len(data) and data[offset] >> 6 == 2
                            types.append(data[offset + 1])
                            offset += 4 * (struct.unpack("!H", data[offset + 2:offset + 4])[0] + 1)
                        assert offset == len(data), "bounded compound RTCP"
                        report["last_compound_rtcp"] = {"packet_types": types, "bytes": len(data)}
                        assert report.get("rtcp_mux_offered"), "RTCP arrived on RTP without offered multiplexing"
                        continue
                    report["last_rtp_header"] = {"bytes": len(data), "first": data[0] if data else None, "second": data[1] if len(data) > 1 else None}
                    assert len(data) >= 12 and data[0] >> 6 == 2 and data[1] & 127 == 0, "expected PCMU RTP"
                    header = 12 + 4 * (data[0] & 15)
                    if data[0] & 16:
                        assert header + 4 <= len(data)
                        header += 4 + 4 * struct.unpack("!H", data[header + 2:header + 4])[0]
                    end = len(data) - (data[-1] if data[0] & 32 else 0)
                    assert header <= end
                    received.extend(data[header:end])
                    assert len(received) <= 160000, "bounded receive capture"
                    report["received_rtp"] += 1
                    continue
                head, body = data.decode("utf8").split("\r\n\r\n", 1)
                lines = head.split("\r\n")
                assert len(lines) <= 64 and lines[0].endswith(" SIP/2.0")
                method, uri, _ = lines[0].split(" ")
                fields = []
                for line in lines[1:]:
                    name, value = line.split(":", 1)
                    fields.append((name.lower(), value.strip()))
                def field(name):
                    values = [value for key, value in fields if key == name]
                    assert len(values) == 1, "unique SIP fixture header"
                    return values[0]
                call_id = field("call-id")
                calls.add(call_id)
                assert len(calls) == 1, "replay caused another SIP dialog"
                assert int(field("content-length")) == len(body.encode())
                answer = ""
                if method == "INVITE":
                    assert uri.startswith("sip:"), "SIP fixture destination"
                    user, authority = uri[4:].split("@", 1)
                    assert unquote(user) == "+14155550123", "explicit synthetic destination only"
                    assert authority.split(";", 1)[0] == f'{spec["host_media_ip"]}:{spec["sip_front"]}', "fixed owned SIP authority"
                    report["sip_invites"] += 1
                    address = [line.removeprefix("c=IN IP4 ") for line in body.splitlines() if line.startswith("c=IN IP4 ")]
                    audio = [line.split() for line in body.splitlines() if line.startswith("m=audio ")]
                    assert address == [spec["host_media_ip"]] and len(audio) == 1 and "0" in audio[0][3:]
                    port = int(audio[0][1])
                    assert spec["media_port_start"] <= port <= spec["media_port_start"] + 15
                    media_target = (address[0], port)
                    report["rtcp_mux_offered"] = "a=rtcp-mux" in body.splitlines()
                    rtcp = "a=rtcp-mux\r\n" if report["rtcp_mux_offered"] else f'a=rtcp:{spec["rtp_front"] + 1} IN IP4 {spec["host_media_ip"]}\r\n'
                    answer = (f'v=0\r\no=remote-phone 1 1 IN IP4 {spec["host_media_ip"]}\r\ns=synthetic-phone\r\n'
                              f'c=IN IP4 {spec["host_media_ip"]}\r\nt=0 0\r\nm=audio {spec["rtp_front"]} RTP/AVP 0\r\n'
                              f'a=rtpmap:0 PCMU/8000\r\n{rtcp}a=sendrecv\r\n')
                    report["server_advertised_media_ip_matched"] = True
                elif method == "ACK":
                    report["sip_acks"] += 1
                    continue
                elif method == "BYE":
                    report["sip_byes"] += 1
                else:
                    assert method == "OPTIONS", "unexpected SIP method"
                to = field("to")
                if ";tag=" not in to:
                    to += ";tag=remote-linux-phone"
                response = ["SIP/2.0 200 OK"] + ["Via: " + value for name, value in fields if name == "via"]
                response += ["From: " + field("from"), "To: " + to, "Call-ID: " + call_id, "CSeq: " + field("cseq")]
                if answer:
                    response += [f'Contact: <sip:peer@{spec["host_media_ip"]}:{spec["sip_front"]}>', "Content-Type: application/sdp"]
                response += ["Content-Length: " + str(len(answer.encode())), "", answer]
                wire.sendto("\r\n".join(response).encode(), source)
                if method == "BYE":
                    report["unique_call_ids"] = len(calls)
                    assert report["sip_acks"] and next_frame >= 190 and len(received) >= 8000
                    report["passed"] = True
                    return
    finally:
        wire.close()
        media.close()
        report["owned_sockets_closed"] = True
        report["received_bytes"] = len(received)
        report["received_sha256"] = hashlib.sha256(received).hexdigest()
        (directory / "guest-received.pcmu").write_bytes(received)
        (directory / "guest-summary.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
