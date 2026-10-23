#!/usr/bin/env python3
"""Independently validate actual redacted canonical exports, including privacy."""
import json
from pathlib import Path
import subprocess
import sys
root=Path(__file__).resolve().parents[1]
directory=Path(sys.argv[1])
required={'metadata-redacted.json','retention-redacted.json'}
assert required <= {path.name for path in directory.glob('*.json')}
subprocess.run([sys.executable,str(root/'scripts/validate-gate6-record-capture.py'),str(directory)],check=True)
metadata=json.loads((directory/'metadata-redacted.json').read_text());text=json.dumps(metadata)
for marker in ['private-consultant-canary','private-dialog-canary','private-transcript-canary','private-attachment-canary','derived-private-canary']:
 assert marker not in text
assert 'public-dialog' in text and 'public-summary' in text
assert metadata['parties'][2]=={}
retained=json.loads((directory/'retention-redacted.json').read_text());text=json.dumps(retained)
assert 'private-transcript-canary' not in text and 'public-summary' in text
assert retained['analysis'][0]['body']=={'unavailable':'redacted'}
print(json.dumps({'passed':True,'required_actual_redacted_exports':sorted(required),'scope':'Actual local record redaction/retention output and schema; no remote erasure or host authorization claim.'}))
