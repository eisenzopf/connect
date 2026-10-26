#!/usr/bin/env python3
"""Check derived application schemas against the real bounded serde codec.

Synthetic data only. This is not service authorization or runtime/media evidence.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import time
from jsonschema import Draft202012Validator, FormatChecker

ROOT=Path(__file__).resolve().parents[1]


def sha(path):
    with path.open('rb') as source:return hashlib.file_digest(source,'sha256').hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();os.umask(0o077)
    directory=args.output.resolve();assert directory.is_relative_to((ROOT/'.local').resolve())
    directory.mkdir(parents=True,exist_ok=False,mode=0o700)
    checked=subprocess.run(['python3','scripts/generate-application-http-openapi.py','--check'],cwd=ROOT,capture_output=True)
    (directory/'generation.log').write_bytes(checked.stdout+checked.stderr);assert checked.returncode==0,'Schema generation check failed'
    document=json.loads((ROOT/'contracts/openapi-application-http.json').read_text())
    catalog=json.loads((ROOT/'contracts/application-http-dispatch.json').read_text())
    guard={p:sha(ROOT/p) for p in document['x-connect-generation-inputs']}
    for schema in document['components']['schemas'].values():Draft202012Validator.check_schema(schema)
    def valid(name,value):
        return Draft202012Validator({'$ref':'#/components/schemas/'+name,'components':document['components']},format_checker=FormatChecker()).is_valid(value)
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--offline','--locked','--no-deps','--format-version','1'],cwd=ROOT))
    source=Path(metadata['target_directory'])/'release/examples/application_http_schema'
    assert source.stat().st_size>1_000_000 and os.access(source,os.X_OK)
    codec=directory/'codec';shutil.copy2(source,codec)
    assert source.stat().st_ino!=codec.stat().st_ino and sha(source)==sha(codec)
    rows=[]
    def decode(name,raw,operation=True):
        start=time.monotonic();result=subprocess.run([str(codec),'--decode-operation' if operation else '--decode-response'],input=raw,capture_output=True,timeout=5)
        assert result.returncode==0,(name,'codec process failed')
        value=json.loads(result.stdout);rows.append(dict(case=name,decoded=value['decoded'],seconds=time.monotonic()-start))
        return value
    for index,row in enumerate(catalog['cases']+catalog['task_cases']):
        sample=row['sample'];assert valid('WireOperation',sample)
        supported=row['response_kind'] is not None
        assert valid('ApplicationOperation',sample)==supported
        assert valid('RetiredTopLevelOperation',sample)!=supported
        result=decode('dispatch-'+str(index),json.dumps(sample).encode())
        assert result['decoded'] and result['response_kind']==row['response_kind']
    negative=[
        ('actor-selector',dict(op='health',actor='forged')),
        ('unknown-op',dict(op='unknown')),
        ('missing-field',dict(op='resolve')),
        ('wrong-type',dict(op='inbox',cursor=None,limit='one',wait_ms=0)),
        ('unknown-task',dict(op='task',request=dict(task_profile='connect-task/1',command=dict(action='unknown')))),
    ]
    for name,value in negative:
        assert not valid('WireOperation',value)
        assert not decode(name,json.dumps(value).encode())['decoded']
    for name,raw in [
        ('duplicate-discriminator',b'{"op":"health","op":"health"}'),
        ('duplicate-metadata',b'{"op":"create","request_id":"duplicate-metadata","request":{"policy":"persistent","initial_participants":[],"metadata":{"duplicate":1,"duplicate":2}}}'),
    ]:
        assert valid('WireOperation',json.loads(raw))
        assert not decode(name,raw)['decoded']
        rows[-1]['scope']='Byte-level duplicate-key refusal; parsed JSON Schema cannot express this constraint'
    healthy=dict(profile='connect-control/1',result=dict(Ok=dict(kind='health',data=dict(installation_id='synthetic',schema=1,profile='connect-control/1'))))
    assert valid('ApplicationResponse',healthy)
    result=decode('healthy-response',json.dumps(healthy).encode(),False)
    assert result['decoded'] and valid('ApplicationResponse',result['encoded'])
    for name,change in [('unknown-result-key','extra'),('both-result-branches','Err')]:
        value=copy.deepcopy(healthy);value['result'][change]={'code':'synthetic','message':'synthetic'}
        assert not valid('ApplicationResponse',value)
        assert not decode(name,json.dumps(value).encode(),False)['decoded']
    contact=dict(profile='connect-control/1',result=dict(Ok=dict(kind='contact',data=dict(peer_identity_id='synthetic',ownership='unknown',pairing_id=None,verified_at=None,sms_consent=False))))
    assert valid('ApplicationResponse',contact);assert decode('pairing-without-consent',json.dumps(contact).encode(),False)['decoded']
    contact['result']['Ok']['data']['sms_consent']=True
    assert not valid('ApplicationResponse',contact);assert not decode('synthetic-sms-consent',json.dumps(contact).encode(),False)['decoded']
    wrong=copy.deepcopy(healthy);wrong['profile']='other/1'
    assert not valid('ApplicationResponse',wrong);assert decode('wrong-transport-profile',json.dumps(wrong).encode(),False)['decoded']
    rows[-1]['scope']='Transport profile is intentionally narrower than the raw serde String DTO'
    assert all(sha(ROOT/p)==h for p,h in guard.items())
    summary=dict(scope='Synthetic real serde/schema/dispatch agreement only; no service authorization/media/profile qualification',
        codec_sha256=sha(codec),source_unchanged=True,generation_inputs=guard,schemas=len(document['components']['schemas']),
        dispatch_cases=len(catalog['cases']),task_cases=len(catalog['task_cases']),checks=rows,openapi_sha256=sha(ROOT/'contracts/openapi-application-http.json'),
        catalog_sha256=sha(ROOT/'contracts/application-http-dispatch.json'))
    (directory/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps(dict(dispatch=summary['dispatch_cases'],checks=len(rows),schemas=summary['schemas'],source_unchanged=True)))


if __name__=='__main__':main()
