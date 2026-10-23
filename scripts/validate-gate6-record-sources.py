#!/usr/bin/env python3
"""Independent schema, lineage, provider reference and JWS checks of actual exports."""
import base64
import json
from pathlib import Path
import subprocess
import sys
import tempfile
root=Path(__file__).resolve().parents[1]
directory=Path(sys.argv[1])
required={'actual-autoend-lineage.json','actual-autoend-recovery.json','actual-provider-capture.json','actual-inline-media-view.json','canonical-signed-payload.json','canonical-signed-fetch.jws'}
assert required <= {p.name for p in directory.iterdir()}
subprocess.run([sys.executable,str(root/'scripts/validate-gate6-record-retrieval.py'),str(directory)],check=True)
for filename,reason,terminal in [('actual-autoend-lineage.json','normal','adapter-ended'),('actual-autoend-recovery.json','failed','adapter-failed')]:
 value=json.loads((directory/filename).read_text());text=json.dumps(value)
 assert 'private-terminal-error-canary' not in text and 'changed-after-autoend-canary' not in text
 outcomes=[a['body'] for a in value['analysis'] if a['type']=='session-outcome']
 assert len(outcomes)==1 and outcomes[0]['reason']==reason and outcomes[0]['recorded_audio'] is False
 phases={a['body']['phase'] for a in value['analysis'] if a['type']=='connection-lineage'}
 assert {'adapter-connected','locally-retired',terminal} <= phases
provider=json.loads((directory/'actual-provider-capture.json').read_text())
refs=[a['body'] for a in provider['analysis'] if a['type']=='provider-reference']
assert refs and any(r['provider_session_id']=='call-mock-1' for r in refs)
assert all(r['verification']=='provider-reported' and r['remote_deletion']=='not-attempted' for r in refs)
media=json.dumps(json.loads((directory/'actual-inline-media-view.json').read_text()))
assert base64.urlsafe_b64encode(b'actual-public-inline-audio').decode().rstrip('=') in media
for secret in [b'actual-private-inline-audio',b'private-transcript-source']:
 assert base64.urlsafe_b64encode(secret).decode().rstrip('=') not in media
signed=json.loads((directory/'canonical-signed-fetch.jws').read_text())
def decode(value):return base64.urlsafe_b64decode(value+'='*(-len(value)%4))
payload=decode(signed['payload']);value=json.loads(payload)
assert value==json.loads((directory/'canonical-signed-payload.json').read_text())
assert 'public-record-body' in payload.decode() and 'private-record-body-canary' not in payload.decode() and 'private-record-party-canary' not in payload.decode()
assert len(signed['signatures'])==1
signature=signed['signatures'][0];protected=json.loads(decode(signature['protected']))
assert protected['alg']=='RS256' and protected['uuid']==value['uuid']
# Verify with the pinned public fixture key, never x5u certificate discovery.
with tempfile.TemporaryDirectory(prefix='signature-',dir=directory) as temporary:
 temp=Path(temporary);(temp/'input').write_bytes((signature['protected']+'.'+signed['payload']).encode());(temp/'signature').write_bytes(decode(signature['signature']))
 result=subprocess.run(['openssl','dgst','-sha256','-verify',str(root/'.deps/rvoip/crates/extensions/rvoip-vcon/tests/fixtures/jws-rs256-public-key.txt'),'-signature',str(temp/'signature'),str(temp/'input')],capture_output=True,text=True)
 assert result.returncode==0,result.stderr
print(json.dumps({'passed':True,'actual_exports':sorted(required),'signature':'RS256 independently verified with pinned public key using OpenSSL','scope':'Local reusable record integration; production installation and Gate 6 acceptance remain separate.'}))
