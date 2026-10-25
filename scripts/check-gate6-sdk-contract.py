#!/usr/bin/env python3
"""Check the G6-059 candidate inventory/vectors; not SDK implementation parity."""
import argparse
import copy
import datetime
import hashlib
import importlib.metadata
import json
from pathlib import Path
import re
import sys
import jsonschema

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    assert output.is_relative_to((ROOT / '.local').resolve())
    output.mkdir(parents=True, exist_ok=False)
    names = ['contracts/sdk-gate-6-api.json', 'contracts/uctp-vectors/gate6-sdk-api.json',
             'contracts/schemas/uctp-envelope-2.schema.json', 'contracts/uctp-vectors/gate6-envelope.json',
             'crates/connect-protocol/src/session_control.rs', 'crates/connect-protocol/src/transitions.rs',
             'sdk/typescript/src/events.ts', 'sdk/python/connect_sdk/event_validation.py',
             'sdk/python/connect_sdk/protocol.py', 'docs/SDK_COMPATIBILITY.md',
             'scripts/check-gate6-sdk-contract.py', 'docs/decisions/015-gate-6-operating-profile.md']
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    before = {n: digest(ROOT / n) for n in names}
    contract = json.loads((ROOT / names[0]).read_text())
    assert contract['status'] == 'candidate-under-review'
    assert {h['name'] for h in contract['handles']} == {'Client','Conversation','Participant','Session','Connection','Stream','Operation'}
    assert all(not h['snapshot_grants_authority'] and not h['from_id_grants_authority'] for h in contract['handles'])
    profile = (ROOT / names[-1]).read_text()
    approved_minima = {
        'rust': re.search(r'Rust/MSRV.*?(\d+\.\d+\.\d+)', profile, re.S)[1],
        'node': re.search(r'Node minimum is (\d+)', profile)[1],
        'python': re.search(r'Python minimum is (\d+\.\d+)', profile)[1],
    }
    assert all(contract['runtime_targets'][language]['minimum'] == minimum
               for language, minimum in approved_minima.items()), 'Runtime targets differ from approved decision 015'
    methods = contract['methods']; assert len({m['id'] for m in methods}) == len(methods)
    available = {v['envelope']['type'] for v in json.loads((ROOT / names[3]).read_text())['vectors'] if v['valid']}
    assert all(m['canonical_type'] in available and m['implementation_qualified'] is False for m in methods)
    assert set(contract['required_application_outlines']) >= {'private-send','consultation','handoff','device-move','recovery'}
    schema = json.loads((ROOT / names[2]).read_text())
    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    vectors = json.loads((ROOT / names[1]).read_text())['vectors']
    checked = []
    for row in vectors:
        e = row['envelope']; valid = validator.is_valid(e)
        assert valid == row['schema_valid'], (row['id'], list(validator.iter_errors(e)))
        rule = row.get('rule'); semantic = valid
        if rule == 'private-audience': semantic = isinstance(e['payload']['to'], list) and bool(e['payload']['to'])
        elif rule == 'decision-id': semantic = e.get('operation_id') != e['payload']['operation_id']
        elif rule == 'recovery-query-only': semantic = e['type'] == 'operation.get' and 'operation_id' not in e
        assert semantic == row['contract_valid'], row['id']
        checked.append({'id':row['id'],'schema_valid':valid,'candidate_contract_valid':semantic})
    rust = (ROOT / names[4]).read_text(); fields = re.findall(r'pub (\w+):', re.search(r'pub struct SessionView \{(.*?)\n\}',rust,re.S)[1])
    ts = (ROOT / names[6]).read_text(); ts_fields = re.findall(r'^\s+(\w+)\??:',re.search(r'export interface SessionSnapshot \{(.*?)\n\}',ts,re.S)[1],re.M)
    assert fields == contract['session_snapshot']['fields'] == ts_fields
    sys.path.insert(0, str(ROOT / 'sdk/python'))
    from connect_sdk.event_validation import session_snapshot
    from connect_sdk.protocol import valid_data
    current = {'id':'sess_example','conversation_id':'conv_example','state':'active','media':['audio'],
               'intent':'consult','capabilities_offer':{},'codec_preferences':['opus'],'is_main':True,
               'revision':7,'topology_revision':1,'created_at':1,'updated_at':2,'ended_reason':None}
    legacy = {'id':'sess_example','conversation_id':'conv_example','initiator_id':'part_a','participant_id':'part_b',
              'state':'active','media':['audio'],'created_at':1,'updated_at':2,'ended_reason':None}
    assert session_snapshot(current) and not session_snapshot(legacy)
    legacy_generic = valid_data('session',legacy) and not valid_data('session',current)
    report = {'schema':'connect-gate6-sdk-contract-preparation/1','task':'G6-059',
              'scope':'Candidate contract inventory, canonical schema/refusal vectors and current snapshot source audit only. Final language outlines/type checking/review remain pending. No SDK runtime/parity/package ownership or whole-task qualification.',
              'passed':True,'candidate_finalized':False,'handles':len(contract['handles']),'mapped_methods':len(methods),
              'approved_runtime_minima':approved_minima,
              'vectors':checked,'session_view_fields':fields,'current_typescript_snapshot_matches':True,
              'current_python_event_snapshot_matches':True,'legacy_python_generic_session_result_still_differs':legacy_generic,
              'inputs_sha256':before,'inputs_unchanged':all(digest(ROOT/n)==v for n,v in before.items()),
              'python':sys.version.split()[0],'jsonschema':importlib.metadata.version('jsonschema'),
              'checked_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    assert report['inputs_unchanged']
    (output/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ['passed','candidate_finalized','handles','mapped_methods','inputs_unchanged','legacy_python_generic_session_result_still_differs']}))

if __name__ == '__main__': main()
