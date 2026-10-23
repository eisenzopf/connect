#!/usr/bin/env python3
"""Validate Gate 6 schemas, traceability and semantic boundary vectors.

Contract proof only: this does not start servers or qualify providers/media.
"""
import argparse
import copy
import hashlib
import importlib.metadata
import json
import pathlib
import re
import subprocess
import sys

try:
 import jsonschema
except ImportError:
 raise SystemExit('Install scripts/requirements-gate6.txt in the chosen test environment')

ROOT=pathlib.Path(__file__).resolve().parents[1]

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--output',type=pathlib.Path)
 args=parser.parse_args()
 subprocess.run([sys.executable,str(ROOT/'scripts/audit-gate6-uctp.py'),'--check'],check=True)
 ledger=json.loads((ROOT/'contracts/uctp-gate-6-requirements.json').read_text())
 schema=json.loads((ROOT/'contracts/schemas/uctp-envelope-2.schema.json').read_text())
 vectors=json.loads((ROOT/'contracts/uctp-vectors/gate6-envelope.json').read_text())['vectors']
 states=json.loads((ROOT/'contracts/uctp-vectors/operation-transitions.json').read_text())
 source=(ROOT/'docs/connect-gate-6-implementation.html').read_text()
 plan=json.loads(re.search(r'<script id="plan-data" type="application/json">(.*?)</script>',source,re.S)[1])
 ids={t['id'] for t in plan['tasks']}
 jsonschema.Draft202012Validator.check_schema(schema)
 validator=jsonschema.Draft202012Validator(schema,format_checker=jsonschema.FormatChecker())
 for row in ledger['requirements']:
  assert row['source_excerpt'] and row['applicability_rationale']
  assert set(row['implementation_items'])<=ids
  assert row['verification']['case_id']
 for section in ledger['sections']:
  assert section['implementation_items'] and set(section['implementation_items'])<=ids
 for vector in vectors:
  valid=validator.is_valid(vector['envelope'])
  assert valid==vector['valid'],vector['id']
 positive={v['envelope']['type'] for v in vectors if v['valid']}
 assert len(positive)==66 and set(e['type'] for e in ledger['envelopes'])<=positive
 # Security and concurrency boundaries go beyond generated shape roundtrips.
 handoff=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-session.handoff')
 cases=[]
 def reject(name,change):
  value=copy.deepcopy(handoff);change(value)
  assert not validator.is_valid(value),name
  cases.append(name)
 reject('unbounded-correlation-id',lambda v:v.update(id='x'*129))
 reject('non-ascii-identifier',lambda v:v.update(cid='conversation/other'))
 reject('unsafe-revision-precision',lambda v:v.update(expected_revision=2**53))
 reject('negative-revision',lambda v:v.update(expected_revision=-1))
 reject('invalid-outgoing-authority-mode',lambda v:v['payload'].update(outgoing='administrator'))
 reject('missing-retained-connection',lambda v:v['payload'].pop('retained_connid'))
 reject('conversation-role-transfer-needs-revision',lambda v:v['payload'].pop('expected_conversation_revision'))
 reject('unbounded-unknown-envelope-type',lambda v:v.update(type='arbitrary.admin.execute'))
 move=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-connection.replace')
 move['payload']['candidate_connid']='conn_other'
 assert not validator.is_valid(move),'replacement must choose endpoint XOR candidate'
 cases.append('ambiguous-replacement-target')
 private=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-session.consult')
 private['payload']['private_participants']=['part_one','part_one']
 assert not validator.is_valid(private),'duplicate private audience'
 cases.append('duplicate-consultation-audience')
 returning=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-session.consult.return')
 for key in ('consultation_record_id','retained_connid'):
  invalid_return=copy.deepcopy(returning);invalid_return['payload'].pop(key)
  assert not validator.is_valid(invalid_return),'return requires '+key
  cases.append('return-needs-'+key)
 invalid_return=copy.deepcopy(returning);invalid_return.pop('connid')
 assert not validator.is_valid(invalid_return),'return requires explicit original primary'
 cases.append('return-needs-explicit-primary')

 change=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-session.update')
 change['payload']['kind']='arbitrary-topology-change'
 assert not validator.is_valid(change),'session updates must use known semantics'
 cases.append('typed-session-update')
 change['payload'].update(kind='role-change',details={})
 assert not validator.is_valid(change),'role change must identify its target and role'
 cases.append('role-change-required-target')
 update=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-operation.updated')
 update['payload'].update(state='unknown',result={'media_connected':True})
 assert not validator.is_valid(update),'uncertain operation cannot advertise completed result'
 cases.append('unknown-is-not-completed')
 for name in states['terminal']:
  assert states['edges'][name]==[],name+' must not regress'
 assert 'preparing' not in states['edges']['unknown'],'unknown external effect must not redispatch'
 assert 'cancelled' not in states['edges']['committing'],'cannot promise precommit cancel after commit begins'
 assert 'interrupted->preparing' in states['conditional_edges'],'safe restart needs explicit evidence'
 # Fresh retry preserves mutation identity but changes transport correlation.
 retry=copy.deepcopy(handoff);retry['id']='env_fresh_retry'
 assert validator.is_valid(retry) and retry['operation_id']==handoff['operation_id'] and retry['id']!=handoff['id']
 cases.append('fresh-envelope-stable-operation')
 extension=copy.deepcopy(handoff);extension['future_optional_hint']={'value':1};extension['payload']['future_optional_hint']='opaque'
 assert validator.is_valid(extension),'UCTP optional extension seam must survive'
 cases.append('optional-extensions-do-not-change-authority')
 contracts=['connect-uctp.md','connect-audiences.md','connect-transitions.md','connect-operations.md','connect-events.md','connect-media-v2.md','connect-records.md']
 fingerprints={f'contracts/{name}':hashlib.sha256((ROOT/'contracts'/name).read_bytes()).hexdigest() for name in contracts}
 result=dict(status='passed',scope='Source audit, schema vectors and contract invariants only; no runtime/media/provider qualification.',source_sections=len(ledger['sections']),source_clauses=len(ledger['requirements']),baseline_envelopes=49,target_envelopes=len(positive),schema_vectors=len(vectors),semantic_boundary_checks=cases,operation_states=len(states['states']),contract_sha256=fingerprints,tool_versions=dict(python=sys.version.split()[0],jsonschema=importlib.metadata.version('jsonschema')))
 if args.output:
  args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps({k:v for k,v in result.items() if k not in ('contract_sha256',)}))

if __name__=='__main__':main()
