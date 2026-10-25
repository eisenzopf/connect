"""Prepare isolated native encoded sources; no transport qualification."""
import argparse
import json
import os
from pathlib import Path
import shutil
from opus_fixture import prepare as audio
from vp8_fixture import prepare as video
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory',type=Path)
args=parser.parse_args();args.directory.mkdir(exist_ok=False)
ffmpeg=os.environ.get('CONNECT_GATE6_FFMPEG') or shutil.which('ffmpeg')
assert ffmpeg,'An installed FFmpeg is required for native codec fixtures'
result=dict(audio=audio(args.directory/'audio',ffmpeg),video=video(args.directory/'video',ffmpeg))
(args.directory/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(dict(passed=True)),flush=True)
