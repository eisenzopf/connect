"""Independent bounded decoder observations for an owned native fixture.

The caller must bind encoded inputs to its actual verified public native client.
This decoder reports observations; it does not assert transport, playout or task
qualification, and never substitutes an ACK or source packet count for decoding.
"""
from pathlib import Path
import hashlib
import json
import math
import re
import struct
import subprocess
from opus_fixture import ogg_image

MAX_AUDIO = 1000
MAX_VIDEO = 600
FRAME_BYTES = 640 * 360 * 3


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def validate_events(events, start, end):
    assert 0 <= start < end and end - start <= 20000
    assert all(isinstance(e['arrival_ms'], (int,float)) and math.isfinite(e['arrival_ms']) and e['arrival_ms'] >= 0 for e in events)
    assert all(a['arrival_ms'] <= b['arrival_ms'] for a,b in zip(events,events[1:]))


def audio(packet_file, events, start, end, expected_hz, directory, ffmpeg='/opt/homebrew/bin/ffmpeg'):
    directory=Path(directory);directory.mkdir(exist_ok=False)
    validate_events(events,start,end);assert expected_hz in (250,500)
    blob=Path(packet_file).read_bytes();assert len(blob)<=MAX_AUDIO*1170
    encoded=[];offset=0
    while offset<len(blob):
        assert offset+2<=len(blob)
        size=struct.unpack_from('<H',blob,offset)[0];offset+=2
        assert 0<size<=1168 and offset+size<=len(blob)
        packet=blob[offset:offset+size];offset+=size
        config=packet[0]>>3
        frame_ms=(10,20,40,60)[config%4] if config<12 else (10,20)[config%2] if config<16 else (2.5,5,10,20)[config%4]
        code=packet[0]&3
        assert code!=3 or len(packet)>=2
        count=1 if code==0 else 2 if code in (1,2) else packet[1]&63
        assert count>0 and frame_ms*count==20 and packet[0]&4==0
        encoded.append(packet);assert len(encoded)<=MAX_AUDIO
    assert offset==len(blob) and len(encoded)==len(events) and encoded
    wrapped=directory/'received.opus';wrapped.write_bytes(ogg_image(encoded))
    raw=directory/'decoded.pcm'
    command=[ffmpeg,'-hide_banner','-loglevel','error','-nostdin','-i',str(wrapped),'-f','s16le','-ar','48000','-ac','1','-threads','1',str(raw)]
    with (directory/'decode.log').open('wb') as log:subprocess.run(command,stderr=log,check=True,timeout=20)
    assert raw.stat().st_size==len(encoded)*960*2,'Decoded Opus/event mapping must be exact'
    basis={hz:([math.cos(2*math.pi*hz*n/48000) for n in range(960)],[math.sin(2*math.pi*hz*n/48000) for n in range(960)]) for hz in (250,500)}
    observed=[]
    with raw.open('rb') as stream:
        for index,event in enumerate(events):
            samples=struct.unpack('<960h',stream.read(1920))
            if not start<=event['arrival_ms']<end:continue
            def power(hz):
                cos,sin=basis[hz];return sum(a*b for a,b in zip(samples,cos))**2+sum(a*b for a,b in zip(samples,sin))**2
            identified=power(expected_hz)>4*power(750-expected_hz) and max(abs(v) for v in samples)>1310
            observed.append(dict(index=index,arrival_ms=event['arrival_ms'],identified=identified))
    result=dict(decoded=len(observed),identified=sum(e['identified'] for e in observed),window_start_ms=start,window_end_ms=end,observations=observed,input_sha256=digest(packet_file),decoded_sha256=digest(raw),command=command,helper_sha256=digest(__file__))
    raw.unlink();(directory/'summary.json').write_text(json.dumps(result,indent=2)+'\n');return result


def video(ivf_file, events, start, end, expected_color, directory, ffmpeg='/opt/homebrew/bin/ffmpeg'):
    directory=Path(directory);directory.mkdir(exist_ok=False)
    validate_events(events,start,end);assert expected_color in ('red','blue')
    blob=Path(ivf_file).read_bytes();assert len(blob)<=MAX_VIDEO*(256*1024+12)+32
    assert blob[:12]==b'DKIF\x00\x00\x20\x00VP80' and struct.unpack_from('<HH',blob,12)==(640,360)
    offset=32;input_pts=[]
    while offset<len(blob):
        assert offset+12<=len(blob)
        size,pts=struct.unpack_from('<IQ',blob,offset);offset+=12
        assert 0<size<=256*1024 and offset+size<=len(blob)
        input_pts.append(pts);offset+=size;assert len(input_pts)<=MAX_VIDEO
    assert offset==len(blob) and len(input_pts)==len(events) and input_pts
    assert input_pts==[e['pts'] for e in events] and len(set(input_pts))==len(input_pts)
    by_pts={e['pts']:e for e in events}
    raw=directory/'decoded.rgb';log_file=directory/'decode.log'
    # Passthrough prevents FFmpeg's output scheduler from manufacturing frames
    # for missing RTP timestamps. showinfo binds each decoded image to its exact
    # original input PTS; prior frames remain available as decoder pre-roll.
    command=[ffmpeg,'-hide_banner','-loglevel','info','-nostdin','-copyts','-i',str(ivf_file),'-vf','showinfo','-fps_mode','passthrough','-threads','1','-f','rawvideo','-pix_fmt','rgb24',str(raw)]
    with log_file.open('wb') as log:subprocess.run(command,stderr=log,check=True,timeout=30)
    pts=[int(m[1]) for m in re.finditer(r'\bn:\s*\d+\s+pts:\s*(-?\d+)\s+pts_time:',log_file.read_text())]
    assert len(pts)<=len(input_pts) and len(pts)==len(set(pts)) and all(p in by_pts for p in pts)
    assert raw.stat().st_size==len(pts)*FRAME_BYTES,'Decoded VP8/event mapping must be exact'
    assert pts,'No independently decoded video frames'
    observed=[];motion=set()
    with raw.open('rb') as stream:
        for pts_value in pts:
            frame=stream.read(FRAME_BYTES);event=by_pts[pts_value]
            if not start<=event['arrival_ms']<end:continue
            pixel=frame[:3];identified=pixel[0]>150 and pixel[2]<80 if expected_color=='red' else pixel[2]>150 and pixel[0]<80
            row=frame[180*640*3:181*640*3]
            xs=[x for x in range(640) if row[x*3+1]>120 and row[x*3+1]>row[x*3]*1.5 and row[x*3+1]>row[x*3+2]*1.5]
            center=round(sum(xs)/len(xs)) if len(xs)>8 else None
            if identified and center is not None:motion.add(center)
            observed.append(dict(pts=pts_value,arrival_ms=event['arrival_ms'],identified=identified,motion_center=center))
    result=dict(decoded=len(observed),identified=sum(e['identified'] for e in observed),distinct_motion=len(motion),window_start_ms=start,window_end_ms=end,observations=observed,input_sha256=digest(ivf_file),decoded_sha256=digest(raw),command=command,helper_sha256=digest(__file__))
    raw.unlink();(directory/'summary.json').write_text(json.dumps(result,indent=2)+'\n');return result


if __name__ == '__main__':
    import argparse
    import os
    import shutil
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--received',required=True,type=Path)
    args=parser.parse_args();root=args.received.resolve()
    ffmpeg=os.environ.get('CONNECT_GATE6_FFMPEG') or shutil.which('ffmpeg')
    assert ffmpeg,'An installed FFmpeg is required for independent native decoding'
    metadata=json.loads((root/'received.json').read_text())
    start,end=metadata['window_start_ms'],metadata['window_end_ms']
    assert 4000<=end-start<=10000
    audio_result=audio(root/'received.frames',metadata['audio_events'],start,end,500,root/'audio-observed',ffmpeg)
    video_result=video(root/'received.ivf',metadata['video_events'],start,end,'red',root/'video-observed',ffmpeg)
    passed=audio_result['identified']>=150 and video_result['identified']>=90 and video_result['distinct_motion']>=40 and metadata['data_messages']==3 and metadata['canonical_public_native_client'] is True and metadata['native_transport']=='quic'
    result=dict(passed=passed,audio={k:audio_result[k] for k in ('decoded','identified','input_sha256','decoded_sha256')},video={k:video_result[k] for k in ('decoded','identified','distinct_motion','input_sha256','decoded_sha256')},measured_ms=end-start,video_sequence_gaps=metadata['video_sequence_gaps'],reliable_data_messages=metadata['data_messages'],ffmpeg_version=subprocess.check_output([ffmpeg,'-version'],text=True).splitlines()[0],scope='Independent decoding of actual public native QUIC received packets with original arrival-window mapping and decoder pre-roll; software source fixtures. No live native encoder, realtime native presenter, sync or operating-profile qualification.')
    (root/'native-observed.json').write_text(json.dumps(result,indent=2)+'\n')
    assert passed,json.dumps(result)
    print(json.dumps(result),flush=True)
