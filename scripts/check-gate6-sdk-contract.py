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
             'crates/connect-store/src/transitions/native_consultations.rs',
             'crates/connect-store/src/session_control.rs', 'crates/connect-store/src/session_decisions.rs', 'crates/connect-store/src/transitions/actions.rs',
             'crates/connect-client/src/sdk/connections.rs', 'crates/connect-client/src/sdk/connection_updates.rs',
             'crates/connect-client/src/sdk/connection_controls.rs', 'crates/connect-client/src/sdk/selections.rs',
             'crates/connect-client/examples/native_receipts.rs']
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
const sessionSignatures={};
function visit(members,prefix){ for(const member of members){
 const name=member.key?.name;if(!name)continue;
 const path=prefix+'.'+name;
 if(prefix==='DecisionReceipt'&&name==='target')paths.__decision_target=source.slice(...member.typeAnnotation.typeAnnotation.range).replace(/\s+/g,'');
 if(member.type==='TSMethodSignature'){paths[path]=source.slice(...member.returnType.typeAnnotation.range).replace(/\s+/g,'');if(prefix==='Session')sessionSignatures[name]=source.slice(...member.range).replace(/\s+/g,'');}
 else if(member.typeAnnotation?.typeAnnotation.type==='TSTypeLiteral')visit(member.typeAnnotation.typeAnnotation.members,path);
}}
for(const wrapper of ast.body){const node=wrapper.declaration;if(node?.type==='TSInterfaceDeclaration')visit(node.body.body,node.id.name==='ParticipantClient'?'Client':node.id.name);}
paths.__session_signatures=sessionSignatures;
process.stdout.write(JSON.stringify(paths));
"""
    paths = json.loads(subprocess.check_output(['node','--input-type=module','-e',ast_script],cwd=ROOT,text=True))
    for method in methods:
        path = method['target_typescript']
        expected = method['await_result']
        if method['id'] == 'session.consult': expected = 'ConsultationOperation'
        expected = expected if method['id'] == 'events' else 'Promise<' + expected + '>'
        assert paths.get(path) == expected, (method['id'], path, paths.get(path), expected)
    assert paths['Session.inviteParticipants'] == 'Promise<Operation<Session>>'
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
    # Revision 3 records the implemented native receipts. Nullable captures are
    # historical observations, never invented Stream handles or local custody.
    native_types = {
        'NegotiatedStream': 'connections.rs',
        'CodecObservation': 'connection_updates.rs',
        'ConnectionUpdateReceipt': 'connection_updates.rs',
        'StreamSelectionReceipt': 'selections.rs',
    }
    native_fields = {}
    for name, source in native_types.items():
        current = (ROOT / 'crates/connect-client/src/sdk' / source).read_text()
        fields_for = lambda text: re.findall(r'pub (\w+):', re.search(
            r'pub struct ' + name + r'(?:<\w+>)? \{(.*?)\n\}', text, re.S)[1])
        current_fields = fields_for(current)
        ts_fields = [to_snake(n) for n in re.findall(r'^\s+readonly (\w+)\??:', re.search(
            r'export interface ' + name + r' \{(.*?)\n\}', target_ts, re.S)[1], re.M)]
        py_fields = [n.target.id for n in classes[name].body if isinstance(n, ast.AnnAssign)]
        assert current_fields == fields_for(target_rust) == ts_fields == py_fields, name
        native_fields[name] = current_fields
    assert 'pub bindings: Option<Vec<CodecObservation>>' in target_rust
    assert 'readonly bindings: readonly CodecObservation[] | null' in target_ts
    assert ast.unparse(next(n.annotation for n in classes['StreamSelectionReceipt'].body
                           if isinstance(n, ast.AnnAssign) and n.target.id == 'bindings')) == 'Sequence[CodecObservation] | None'
    selection = (ROOT/'crates/connect-client/src/sdk/selections.rs').read_text()
    update = (ROOT/'crates/connect-client/src/sdk/connection_updates.rs').read_text()
    ending = (ROOT/'crates/connect-client/src/sdk/connection_controls.rs').read_text()
    assert 'selectors: NonEmpty<StreamSelector>' in selection and 'Operation<StreamSelectionReceipt>' in selection
    assert 'Operation<ConnectionUpdateReceipt>' in update
    assert 'reason_code: u16' in ending and 'expected_revision: Option<StreamRevision>' in ending
    rust_connection = re.search(r'pub trait Connection<A: Api>.*?\n\}', target_rust, re.S)[0]
    assert 'selectors: NonEmpty<StreamSelector>' in rust_connection
    assert 'reason_code: u16' in rust_connection and 'expected_stream_revision: Option<StreamRevision>' in rust_connection
    assert 'Operation<ConnectionUpdateReceipt<A::Connection>>' in rust_connection
    assert 'Operation<StreamSelectionReceipt<A::Connection>>' in rust_connection
    assert 'StreamSelection<' not in target_rust and 'class StreamSelection:' not in (ROOT/'contracts/sdk-api/python/contract.py').read_text()
    assert contract['revision'] >= 3
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
    controls = ['accept','reject','cancel_invitation','update','end','invite_participants']
    for name in controls:
        method = next(n for n in classes['Session'].body if isinstance(n,ast.AsyncFunctionDef) and n.name == name)
        parameter = next(n for n in method.args.args if n.arg == 'expected_topology_revision')
        assert isinstance(parameter.annotation,ast.Name) and parameter.annotation.id == 'TopologyRevision'
        ts_name = {'cancel_invitation':'cancelInvitation','invite_participants':'inviteParticipants'}.get(name,name)
        signature = paths['__session_signatures'][ts_name]
        assert 'expectedTopologyRevision:Revision<"topology">' in signature and 'Revision<"session">' not in signature
    rust_session = re.search(r'pub trait Session<A: Api>.*?\n\}',target_rust,re.S)[0]
    assert rust_session.count('expected_topology_revision: TopologyRevision') == 6
    assert 'expected_revision: SessionRevision' not in rust_session
    source_control = (ROOT/'crates/connect-store/src/session_control.rs').read_text()
    source_decision = (ROOT/'crates/connect-store/src/session_decisions.rs').read_text()
    assert '.is_some_and(|v| v != old.topology_revision)' in source_control
    assert 'SET topology_revision=topology_revision+1' in source_control
    assert 'SELECT state,topology_revision FROM sessions' in source_decision
    assert 'r.expected_revision.is_some_and(|v| v != revision)' in source_decision
    assert contract['revision'] >= 2 and 'Session accept/reject/cancel/update/end' in contract['revision_families']['topology']
    target_annotation = next(n.annotation for n in classes['DecisionReceipt'].body if isinstance(n,ast.AnnAssign) and n.target.id == 'target')
    assert isinstance(target_annotation,ast.Name) and target_annotation.id == 'OperationId'
    assert paths['__decision_target'] == 'OperationId'
    assert 'pub target: OperationId,' in re.search(r'pub struct DecisionReceipt \{(.*?)\n\}',target_rust,re.S)[1]
    actions = (ROOT/'crates/connect-store/src/transitions/actions.rs').read_text()
    result = re.search(r'let result = serde_json::json!\((.*?)\);',actions,re.S)[1]
    assert all('"'+key+'"' in result for key in ['target_operation_id','state_at_decision','disposition','evidence_id'])
    assert 'target_command' not in result and 'target_scope' not in result
    report = {'schema':'connect-gate6-sdk-contract/1','task':'G6-059',
              'scope':'Frozen shared public contract, current canonical schema/refusal vectors, per-language target Session fields and 22 operation-kind witnesses, current source review. Declaration compilation is recorded separately. No SDK runtime/parity/package ownership or Gate6 acceptance qualification.',
              'passed':True,'candidate_finalized':True,'handles':len(contract['handles']),'mapped_methods':len(methods),
              'approved_runtime_minima':approved_minima,'operation_kind_witnesses':len(expected_kinds),
              'mapped_method_paths_and_returns_verified':len(methods),
              'native_receipt_fields_match_implemented_rust_and_all_declarations':native_fields,
              'native_receipt_nullability_and_lifecycle_inputs_audited':True,
              'session_control_precondition':'topology_revision; audited against both Store paths and all three declarations',
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
