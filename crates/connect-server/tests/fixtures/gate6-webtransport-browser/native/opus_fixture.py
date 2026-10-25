"""Bounded Opus packets and independent PCM witness for native interop fixtures."""
from pathlib import Path
import hashlib
import json
import math
import struct
import subprocess

RATE, SAMPLES, COUNT = 48000, 960, 200


def packets(blob):
    offset, pending, result = 0, bytearray(), []
    while offset < len(blob):
        assert blob[offset:offset + 4] == b'OggS' and blob[offset + 4] == 0
        segments = blob[offset + 26]
        lacing = blob[offset + 27:offset + 27 + segments]
        assert len(lacing) == segments
        cursor = offset + 27 + segments
        for size in lacing:
            pending.extend(blob[cursor:cursor + size]); cursor += size
            if size < 255:
                result.append(bytes(pending)); pending.clear()
        assert cursor <= len(blob)
        offset = cursor
    assert not pending and offset == len(blob)
    assert result[0].startswith(b'OpusHead') and result[1].startswith(b'OpusTags')
    return result[2:]


def crc(blob):
    value = 0
    for b in blob:
        value ^= b << 24
        for _ in range(8):
            value = ((value << 1) ^ (0x04c11db7 if value & 0x80000000 else 0)) & 0xffffffff
    return value


def ogg_image(encoded):
    # One bounded packet/page, monotonically increasing 48kHz granules, no
    # encoder-specific pre-skip: these are already RTP-facing Opus packets.
    headers = [b'OpusHead' + bytes((1, 1)) + struct.pack('<HIhB', 0, RATE, 0, 0), b'OpusTags' + struct.pack('<I', 7) + b'Connect' + struct.pack('<I', 0)]
    output = bytearray()
    for number, packet in enumerate(headers + encoded):
        assert 0 < len(packet) < 1180 - 12
        lacing = bytes([255] * (len(packet) // 255) + [len(packet) % 255])
        flags = 2 if number == 0 else 4 if number == len(encoded) + 1 else 0
        granule = max(0, number - 1) * SAMPLES
        page = bytearray(b'OggS' + bytes((0, flags)) + struct.pack('<QIII', granule, 150, number, 0) + bytes((len(lacing),)) + lacing + packet)
        struct.pack_into('<I', page, 22, crc(page));output.extend(page)
    return output


def prepare(directory, ffmpeg='/opt/homebrew/bin/ffmpeg'):
    directory = Path(directory); directory.mkdir(exist_ok=False)
    source = directory / 'native-250hz.opus'
    pcm = struct.pack('<' + 'h' * (SAMPLES * COUNT), *(round(8192 * math.sin(2 * math.pi * 250 * n / RATE)) for n in range(SAMPLES * COUNT)))
    command = [ffmpeg, '-hide_banner', '-loglevel', 'error', '-nostdin', '-f', 's16le', '-ar', str(RATE), '-ac', '1', '-i', 'pipe:0', '-c:a', 'libopus', '-b:a', '32k', '-vbr', 'off', '-application', 'voip', '-frame_duration', '20', '-threads', '1', '-f', 'opus', str(source)]
    with (directory / 'encode.log').open('wb') as log:subprocess.run(command,input=pcm,stderr=log,check=True,timeout=20)
    encoded = packets(source.read_bytes())[:COUNT]; assert len(encoded) == COUNT
    for packet in encoded:
        config = packet[0] >> 3
        duration = (10,20,40,60)[config % 4] if config < 12 else (10,20)[config % 2] if config < 16 else (2.5,5,10,20)[config % 4]
        count = 1 if packet[0] & 3 == 0 else 2 if packet[0] & 3 in (1, 2) else packet[1] & 63
        assert count > 0 and duration * count == 20 and packet[0] & 4 == 0 and len(packet) == 80
    # A packet file carries no Ogg headers and can be read directly by the
    # public native-client fixture for one original RTP publication per frame.
    packet_file = directory / 'native-250hz.frames'
    packet_file.write_bytes(b''.join(struct.pack('<H',len(p)) + p for p in encoded))
    rewrapped = directory / 'native-rtp-packets.opus';rewrapped.write_bytes(ogg_image(encoded))
    decoded = directory / 'independent-s16le.raw'
    decode = [ffmpeg, '-hide_banner', '-loglevel', 'error', '-nostdin', '-i', str(rewrapped), '-f', 's16le', '-ar', str(RATE), '-ac', '1', '-threads', '1', str(decoded)]
    with (directory / 'decode.log').open('wb') as log:subprocess.run(decode,stderr=log,check=True,timeout=20)
    raw=decoded.read_bytes(); assert len(raw) == COUNT * SAMPLES * 2
    identified = 0
    for offset in range(0,len(raw),SAMPLES * 2):
        frame=struct.unpack('<' + 'h' * SAMPLES,raw[offset:offset + SAMPLES*2])
        def power(hz):
            re=sum(v*math.cos(2*math.pi*hz*n/RATE) for n,v in enumerate(frame));im=sum(v*math.sin(2*math.pi*hz*n/RATE) for n,v in enumerate(frame));return re*re+im*im
        identified += power(250) > power(500)*4 and max(abs(v) for v in frame)>1310
    assert identified >= 195
    h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    result=dict(passed=True,scope='Native32kbit mono48kHz20ms Opus packet source and independent rewrapped PCM witness only; no public client/transport/browser interoperability/playout or G6-050 completion claim.',packets=COUNT,identified_frames=identified,packet_bytes=80,frame_duration_ms=20,bitrate=32000,packet_file_sha256=h(packet_file),source_sha256=h(source),rewrapped_sha256=h(rewrapped),decoded_sha256=h(decoded),helper_sha256=h(Path(__file__)),command=command,decoder_command=decode)
    (directory/'summary.json').write_text(json.dumps(result,indent=2)+'\n');return result

if __name__ == '__main__':
    import sys
    print(json.dumps(prepare(sys.argv[1])),flush=True)
