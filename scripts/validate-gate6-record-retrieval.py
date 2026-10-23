#!/usr/bin/env python3
"""Independent schema and actual canonical-fetch projection export check."""
import json
from pathlib import Path
import subprocess
import sys
root=Path(__file__).resolve().parents[1]
directory=Path(sys.argv[1])
path=directory/'canonical-fetch-view.json'
assert path.is_file()
subprocess.run([sys.executable,str(root/'scripts/validate-gate6-record-redaction.py'),str(directory)],check=True)
value=json.loads(path.read_text());text=json.dumps(value)
assert 'public-record-body' in text
assert 'private-record-body-canary' not in text and 'private-record-party-canary' not in text
assert len(value['parties'])==1 and value['dialog'][0]['parties']==[0]
print(json.dumps({'passed':True,'actual_canonical_fetch_export':str(path),'scope':'Actual caller view/schema/privacy; no production authority or public HTTPS endpoint claim.'}))
