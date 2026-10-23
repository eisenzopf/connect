#!/usr/bin/env python3
"""Validate captured incoming-contact HTTP messages against the reference.

Wire-contract proof only. Run the actual named provider_flow/voice_flow behavior
fixtures with CONNECT_GATE6_INCOMING_SAMPLES set before invoking this checker.
"""
import argparse,datetime,hashlib,json
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT=Path(__file__).resolve().parents[1]
REQUIRED={'voice-held-get','voice-resolve-replay','sms-held-list','sms-resolve-replay','sms-rejected-get','sms-withdrawn-replay'}
def reference(path):
 with path.open('rb') as f:digest=hashlib.file_digest(f,'sha256').hexdigest()
 try:name=str(path.relative_to(ROOT))
 except ValueError:name=str(path)
 return {'path':name,'sha256':digest}
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--samples',type=Path,required=True);p.add_argument('--output',type=Path);args=p.parse_args()
 source=ROOT/'contracts/openapi-incoming-contacts.json';doc=json.loads(source.read_text())
 for schema in doc['components']['schemas'].values():Draft202012Validator.check_schema(schema)
 def validator(name):return Draft202012Validator({'$ref':'#/components/schemas/'+name,'components':doc['components']})
 request=validator('IncomingContactCommand');response=validator('IncomingContactResponse');seen={};negatives=0
 for path in sorted(args.samples.glob('*.json')):
  if path.stem not in REQUIRED:continue
  value=json.loads(path.read_text());request.validate(value['request']);response.validate(value['response']);status=value['status'];result=value['response']['result']
  assert (status==200 and 'Ok' in result) or (status==403 and result.get('Err',{}).get('code')=='forbidden'),path
  assert not any(marker in json.dumps(value['response']) for marker in ['private held SMS','resource-held-public-sms','/control','12345678-1234-1234-1234-123456789015']),path
  bad={**value['request'],'body':'spoofed-private-content'};assert not request.is_valid(bad);negatives+=1
  seen[path.stem]=reference(path)
 assert set(seen)==REQUIRED,('Missing exact actual listener samples',sorted(REQUIRED-set(seen)))
 held=json.loads((args.samples/'sms-held-list.json').read_text())['response']['result']['Ok']['page']['contacts'];assert len(held)==1 and held[0]['state']=='held' and len(held[0]['candidates'])==2
 for kind in ['sms','voice']:
  c=json.loads((args.samples/f'{kind}-resolve-replay.json').read_text())['response']['result']['Ok']['contact'];assert c['state']=='resolved' and c['kind']==kind and c['revision']==1 and c['decision']['candidate_id'] in {x['id'] for x in c['candidates']}
  assert bool(c['decision']['message_id'])==(kind=='sms') and bool(c['decision']['session_id'])==(kind=='voice')
 rejected=json.loads((args.samples/'sms-rejected-get.json').read_text())['response']['result']['Ok']['contact'];assert rejected['state']=='rejected' and rejected['decision']['candidate_id'] is None
 report={'status':'passed','created_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'scope':'Reference schema and exact captured main-listener request/response validation only. Behavior/runtime/authorization/recovery qualification belongs to the separately executed provider_flow and voice_flow fixtures; this checker does not start listeners or close G6-045.','reference':reference(source),'actual_listener_samples':seen,'schema_count':len(doc['components']['schemas']),'closed_shape_negative_vectors':negatives}
 if args.output:args.output.write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({'status':report['status'],'actual_listener_samples':len(seen),'schema_count':report['schema_count'],'negative_vectors':negatives,'runtime_qualification':False}))
if __name__=='__main__':main()
