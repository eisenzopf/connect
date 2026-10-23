#!/usr/bin/env python3
"""Validate actual tenant role provisioning exchanges, without printing values."""
import argparse,json,pathlib
from jsonschema import Draft202012Validator
ROOT=pathlib.Path(__file__).resolve().parents[1]
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--samples',type=pathlib.Path,required=True);parser.add_argument('--output',type=pathlib.Path,required=True);args=parser.parse_args()
 api=json.loads((ROOT/'contracts/openapi-provisioning.json').read_text());schemas=api['components'];paths=sorted(args.samples.glob('*.json'));assert len(paths)==10
 def validator(name):
  schema={'$ref':'#/components/schemas/'+name,'components':schemas};Draft202012Validator.check_schema(schema);return Draft202012Validator(schema)
 request=validator('RecordReadRoleRequest');response=validator('ProvisioningResponse');statuses=set()
 for path in paths:
  sample=json.loads(path.read_text());request.validate(sample['request']);response.validate(sample['response']);status=sample['status'];statuses.add(status)
  assert (status==200)==('Ok' in sample['response']['result'])
  if 'Err' in sample['response']['result']:assert status=={'forbidden':403,'conflict':409}[sample['response']['result']['Err']['code']]
  if sample['case']=='old-create-stays-revoked':assert sample['response']['result']['Ok']['active'] is False
 assert statuses=={200,403,409}
 report={'passed':True,'actual_exchanges':len(paths),'statuses':sorted(statuses),'scope':'Actual named local tenant role provisioning HTTP exchanges; no headers or credentials in samples.'};args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
