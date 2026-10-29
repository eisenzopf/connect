#!/usr/bin/env python3
"""Generate bounded, identified synthetic codec files under ignored .local only."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

ROOT=Path(__file__).resolve().parents[2]


def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def prepare(output,ffmpeg):
    output=Path(output).absolute();ffmpeg=Path(ffmpeg).resolve(strict=True)
    assert output.resolve().is_relative_to((ROOT/'.local').resolve()) and ffmpeg.is_file()
    output.mkdir(mode=0o700,parents=True,exist_ok=False)
    started=time.monotonic();version=subprocess.check_output([str(ffmpeg),'-version'],text=True,timeout=10).splitlines()[0]
    scenes=[('node-source',440,'red'),('node-target',660,'blue'),('python-source',880,'yellow'),('python-target',1100,'cyan')]
    results=[]
    with (output/'codec.log').open('wb') as log:
        for name,frequency,color in scenes:
            base=[str(ffmpeg),'-nostdin','-hide_banner','-v','error','-n']
            audio=output/(name+'.mulaw');video=output/(name+'.ivf')
            subprocess.run([*base,'-f','lavfi','-i',f'sine=frequency={frequency}:sample_rate=8000:duration=0.6','-ac','1','-ar','8000','-c:a','pcm_mulaw','-f','mulaw',str(audio)],stdout=log,stderr=log,check=True,timeout=10)
            subprocess.run([*base,'-f','lavfi','-i',f'color=c={color}:size=640x360:rate=10,drawgrid=width=32:height=32:thickness=1:color=white','-frames:v','6','-an','-c:v','libvpx','-g','1','-deadline','realtime','-cpu-used','8','-threads','1','-b:v','1000k','-f','ivf',str(video)],stdout=log,stderr=log,check=True,timeout=10)
            assert audio.stat().st_size==4800 and 32<video.stat().st_size<=4*1024*1024
            results.append({'name':name,'frequency_hz':frequency,'video_color':color,'audio_sha256':digest(audio),'video_sha256':digest(video),'video_bytes':video.stat().st_size})
    result={'profile':'connect-language-codec-fixtures/1','passed':True,'ffmpeg_version':version,'ffmpeg_sha256':digest(ffmpeg),'ffmpeg':str(ffmpeg),'seconds':time.monotonic()-started,'scenes':results,'codec_children_joined':8,'scope':'Synthetic pre-encoded PCMU/VP8 files only. No transport, language API, decoded-media or platform qualification.'}
    (output/'summary.json').write_text(json.dumps(result,indent=2)+'\n');return result


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--ffmpeg',type=Path,required=True);args=parser.parse_args();print(json.dumps(prepare(args.output,args.ffmpeg)))
