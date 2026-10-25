#!/usr/bin/env python3
"""Check the frozen G6-059 contract/source inventory; not SDK implementation parity."""
import argparse
import ast
import copy
import datetime
import hashlib
import importlib.metadata
import json
from pathlib import Path
import re
import sys
import subprocess
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
             'scripts/check-gate6-sdk-contract.py', 'sdk/typescript/package-lock.json', 'docs/decisions/016-gate-6-sdk.md', 'docs/decisions/015-gate-6-operating-profile.md',
             'crates/connect-service/src/canonical_sessions.rs', 'crates/connect-protocol/src/membership.rs',
             'crates/connect-protocol/src/events.rs', 'crates/connect-service/src/canonical_membership.rs',
             'crates/connect-service/src/canonical.rs', 'crates/connect-protocol/src/conversations.rs',
             'crates/connect-store/src/transitions/native_consultations.rs']
    names += sorted(str(p.relative_to(ROOT)) for p in (ROOT/'contracts/sdk-api').rglob('*') if p.is_file())
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    before = {n: digest(ROOT / n) for n in names}
    contract = json.loads((ROOT / names[0]).read_text())
    assert contract['status'] == 'frozen'
    assert contract['schema'] == 'connect-gate6-sdk-contract/1'
    assert {h['name'] for h in contract['handles']} == {'Client','Conversation','Participant','Session','Connection','Stream','Operation'}
    assert all(not h['snapshot_grants_authority'] and not h['from_id_grants_authority'] for h in contract['handles'])
    profile = (ROOT / 'docs/decisions/015-gate-6-operating-profile.md').read_text()
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
    # Parse real TypeScript declarations so every advertised method path exists.
    ast_script = r"""
import {parsers} from './sdk/typescript/node_modules/prettier/plugins/typescript.mjs';
import fs from 'node:fs';
const source=fs.readFileSync('contracts/sdk-api/typescript/contract.ts','utf8');
const ast=await parsers.typescript.parse(source,{});
const paths={};
function visit(members,prefix){ for(const member of members){
 const name=member.key?.name;if(!name)continue;
 const path=prefix+'.'+name;
 if(member.type==='TSMethodSignature')paths[path]=source.slice(...member.returnType.typeAnnotation.range).replace(/\s+/g,'');
 else if(member.typeAnnotation?.typeAnnotation.type==='TSTypeLiteral')visit(member.typeAnnotation.typeAnnotation.members,path);
}}
for(const wrapper of ast.body){const node=wrapper.declaration;if(node?.type==='TSInterfaceDeclaration')visit(node.body.body,node.id.name==='ParticipantClient'?'Client':node.id.name);}
process.stdout.write(JSON.stringify(paths));
"""
    paths = json.loads(subprocess.check_output(['node','--input-type=module','-e',ast_script],cwd=ROOT,text=True))
    for method in methods:
        path = method['target_typescript']
        expected = method['await_result']
        if method['id'] == 'session.consult': expected = 'ConsultationOperation'
        expected = expected if method['id'] == 'events' else 'Promise<' + expected + '>'
        assert paths.get(path) == expected, (method['id'], path, paths.get(path), expected)
    schema = json.loads((ROOT / names[2]).read_text())
    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    vectors = json.loads((ROOT / names[1]).read_text())['vectors']
    checked = []
    for row in vectors:
        e = row['envelope']; valid = validator.is_valid(e)
        assert valid == row['schema_valid'], (row['id'], list(validator.iter_errors(e)))
        rule = row.get('rule'); semantic = valid
        if semantic and e['type'] in {'session.invite','session.accept','session.reject','session.cancel','session.update','session.end'}:
            semantic = 'connid' not in e and all(k in e for k in ('cid','sid'))
            if e['type'] == 'session.invite':
                semantic = semantic and e['payload']['medium'] in {'voice','video','voice+video','screen-share','text-chat','mixed'}
        if semantic and e['type'] == 'message.send':
            semantic = 'sid' not in e and 'connid' not in e

        if rule == 'private-audience': semantic = semantic and isinstance(e['payload']['to'], list) and bool(e['payload']['to'])
        elif rule == 'decision-id': semantic = semantic and e.get('operation_id') != e['payload']['operation_id']
        elif rule == 'recovery-query-only': semantic = semantic and e['type'] == 'operation.get' and 'operation_id' not in e
        assert semantic == row['contract_valid'], row['id']
        checked.append({'id':row['id'],'schema_valid':valid,'contract_valid':semantic})
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
    # Audit the target Session shape in each language, not just existing SDKs.
    target_py = ast.parse((ROOT/'contracts/sdk-api/python/contract.py').read_text())
    classes = {node.name:node for node in target_py.body if isinstance(node, ast.ClassDef)}
    py_fields = [n.target.id for n in classes['SessionSnapshot'].body if isinstance(n, ast.AnnAssign)]
    target_ts = (ROOT/'contracts/sdk-api/typescript/contract.ts').read_text()
    target_rust = (ROOT/'contracts/sdk-api/rust/contract.rs').read_text()
    to_snake = lambda name: re.sub(r'(?<!^)(?=[A-Z])','_',name).lower()
    ts_target_fields = [to_snake(n) for n in re.findall(r'^\s+readonly (\w+)\??:', re.search(r'export interface SessionSnapshot \{(.*?)\n\}',target_ts,re.S)[1],re.M)]
    rust_target_fields = re.findall(r'pub (\w+):',re.search(r'pub struct SessionSnapshot \{(.*?)\n\}',target_rust,re.S)[1])
    assert fields == py_fields == ts_target_fields == rust_target_fields
    expected_kinds = {m['canonical_type'].replace('.','_') for m in methods if m['logical_operation_id_required']}
    ts_kinds = set(re.findall(r'readonly (\w+): OperationKind<',re.search(r'export interface OperationKinds \{(.*?)\n\}',target_ts,re.S)[1]))
    py_kinds = {n.name for n in classes['OperationKinds'].body if isinstance(n,ast.FunctionDef)}
    rust_kinds = set(re.findall(r'pub (\w+): OperationKind<',re.search(r'pub struct OperationKinds<A: Api> \{(.*?)\n\}',target_rust,re.S)[1]))
    assert expected_kinds == ts_kinds == py_kinds == rust_kinds and len(expected_kinds) == 22
    transitions = (ROOT/'crates/connect-protocol/src/transitions.rs').read_text()
    actual_states = re.findall(r'^    (\w+),$',re.search(r'pub enum OperationState \{(.*?)\n\}',transitions,re.S)[1],re.M)
    assert [to_snake(s) for s in actual_states] == contract['operation_states']
    media_source = re.search(r'pub fn media\(.*?\n\}',rust,re.S)[0]
    assert set(re.findall(r'"([^"\n]+)" =>',media_source)) == {'voice','video','voice+video','screen-share','text-chat','mixed'}
    sessions = (ROOT/'crates/connect-service/src/canonical_sessions.rs').read_text()
    assert '|| e.scope(Scope::Connection).is_some()' in sessions
    membership = (ROOT/'crates/connect-service/src/canonical_membership.rs').read_text()
    assert 'MembershipAction::Add' in membership and 'v.kind' in membership and 'e.expected_revision().ok_or_else(invalid)?' in membership
    completion = re.findall(r'pub (\w+):',re.search(r'pub struct TransitionCompletion \{(.*?)\n\}',transitions,re.S)[1])
    assert completion == ['retained_connection_ids','admitted_connection_ids','retired_connection_ids','session_revision','conversation_revision']
    native_consult = (ROOT/'crates/connect-store/src/transitions/native_consultations.rs').read_text()
    assert 'SET topology_revision=?2' in native_consult and 'session_revision,' in native_consult
    report = {'schema':'connect-gate6-sdk-contract/1','task':'G6-059',
              'scope':'Frozen shared public contract, current canonical schema/refusal vectors, per-language target Session fields and 22 operation-kind witnesses, current source review. Declaration compilation is recorded separately. No SDK runtime/parity/package ownership or Gate6 acceptance qualification.',
              'passed':True,'candidate_finalized':True,'handles':len(contract['handles']),'mapped_methods':len(methods),
              'approved_runtime_minima':approved_minima,'operation_kind_witnesses':len(expected_kinds),
              'mapped_method_paths_and_returns_verified':len(methods),
              'vectors':checked,'session_view_fields':fields,'target_session_fields_match_all_languages':True,
              'current_typescript_snapshot_matches':True,'current_python_event_snapshot_matches':True,
              'legacy_python_generic_session_result_still_differs':legacy_generic,
              'inputs_sha256':before,'inputs_unchanged':all(digest(ROOT/n)==v for n,v in before.items()),
              'python':sys.version.split()[0],'jsonschema':importlib.metadata.version('jsonschema'),
              'checked_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    assert report['inputs_unchanged']
    (output/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ['passed','candidate_finalized','handles','mapped_methods','inputs_unchanged','legacy_python_generic_session_result_still_differs']}))

if __name__ == '__main__': main()
