#!/usr/bin/env python3
"""Source-bound codec examples: byte/framing/failure components, no live SDK IO."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import time
ROOT=Path(__file__).resolve().parents[2]


def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    output=args.output.absolute();assert output.resolve().is_relative_to((ROOT/'.local').resolve());output.mkdir(mode=0o700,parents=True,exist_ok=False)
    paths=['examples/sdk-media/packet-codec.mjs','examples/sdk-media/packet_codec.py','examples/sdk-media/package.json','sdk/tests/packet_codec.test.mjs','sdk/tests/test_packet_codec.py','sdk/tests/check_packet_codec.py']
    paths += [str(p.relative_to(ROOT)) for p in (ROOT/'sdk/typescript/dist').glob('*') if p.is_file()]
    paths += [str(p.relative_to(ROOT)) for p in (ROOT/'sdk/python/connect_sdk').glob('*') if p.is_file()]
    before={p:digest(ROOT/p) for p in paths};(output/'source-inputs.json').write_text(json.dumps(before,indent=2)+'\n');started=time.monotonic()
    results=[]
    for name,command in [('node',['node','--test','--test-reporter=tap','sdk/tests/packet_codec.test.mjs']),('python',[sys.executable,'sdk/tests/test_packet_codec.py'])]:
        with (output/(name+'.log')).open('wb') as log:
            child=subprocess.run(command,cwd=ROOT,env=dict(os.environ,PYTHONPATH=str(ROOT/'sdk/python'),PYTHONDONTWRITEBYTECODE='1'),stdout=log,stderr=subprocess.STDOUT,timeout=30)
        text=(output/(name+'.log')).read_text()
        inventory=(len(re.findall(r'^ok [0-9]+ - ',text,re.M))==4 and all(re.search(r'^# '+value+r'$',text,re.M) for value in ['pass 4','fail 0','skipped 0'])) if name=='node' else bool(re.search(r'^Ran 4 tests in ',text,re.M) and text.rstrip().endswith('OK'))
        results.append({'language':name,'passed':child.returncode==0 and inventory,'functions_passed':4 if child.returncode==0 and inventory else None,'exit_code':child.returncode,'log_sha256':digest(output/(name+'.log'))})
    unchanged=all(digest(ROOT/p)==v for p,v in before.items());result={'passed':unchanged and all(r['passed'] for r in results),'source_inputs_unchanged':unchanged,'source_count':len(before),'source_inputs_sha256':digest(output/'source-inputs.json'),'results':results,'functions_passed':8 if all(r['passed'] for r in results) else None,'seconds':time.monotonic()-started,'scope':'Finite app-owned PCMU/VP8 codec example byte/framing/gap/expiry/source/size components. No helper, real listener, independent decoder, installed media or full G6-064 qualification.'}
    (output/'summary.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));raise SystemExit(0 if result['passed'] else 1)


if __name__=='__main__':main()
