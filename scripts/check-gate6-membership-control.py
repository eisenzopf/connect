#!/usr/bin/env python3
"""Qualify the selected schema-34 membership continuation, never Gate 6 acceptance."""
import argparse,datetime,hashlib,importlib.util,json,os,pathlib,platform,re,subprocess,sys
import jsonschema
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('conversation_checkpoint',ROOT/'scripts/check-gate6-conversation-control.py')
previous=importlib.util.module_from_spec(spec);spec.loader.exec_module(previous)
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def sources():
 result=previous.sources()
 for name in ('scripts/check-gate6-membership-control.py','scripts/generate-gate6-membership-reference.py','docs/developer/membership-control.md','docs/developer/canonical-service.md'):
  result[name]=digest(ROOT/name)
 return dict(sorted(result.items()))
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=pathlib.Path,default=ROOT/'docs/evidence/gate-6/membership-control.json');args=parser.parse_args()
 stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory=ROOT/'.local/gate-6/membership-control'/stamp;directory.mkdir(parents=True)
 samples=directory/'http-samples';state=directory/'prism-state';state.mkdir()
 before=sources();env=dict(os.environ,PYTHONPATH=str(ROOT/'sdk/python'),CONNECT_GATE6_HTTP_SAMPLES=str(samples),PRISM_TEST_STATE_DIR=str(state))
 cargo=['cargo','test','--no-fail-fast','--locked','--offline','-j1']
 commands=[
  ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
  ('format',['bash','scripts/format.sh','--check'],0),
  ('sdk-build',['npm','run','build','--prefix','sdk/typescript'],0),
  ('sdk-format',['npm','run','format:check','--prefix','sdk/typescript'],0),
  ('sdk-node',['node','--test','sdk/tests/membership.test.mjs','sdk/tests/conversations.test.mjs','sdk/tests/control_receipts.test.mjs','sdk/tests/events.test.mjs','sdk/tests/clients.test.mjs'],0),
  ('sdk-python',['python3','-m','unittest','discover','-s','sdk/tests','-p','test_*.py'],0),
  ('protocol',cargo+['-p','connect-protocol'],33),
  ('store',cargo+['-p','connect-store'],443),
  ('service',cargo+['-p','connect-service'],36),
  ('mcp',cargo+['-p','connect-mcp'],7),
  ('server',cargo+['-p','connect-server','--test','gate6_canonical_operations','--test','gate6_membership_control','--test','gate6_messages','--test','gate6_provisioning','--test','first_run_schema_boundary','--test','setup_vault_auth','--test','maintenance','--test','gate6_operations'],20),
  ('native-mcp-regression',cargo+['-p','connect-server','--test','gate4_agent_communications','--test','gate4_native_grants','--test','gate4_native_grant_lifecycle'],5),
  ('server-api-build',['cargo','check','--locked','--offline','-j1','-p','connect-server','--tests'],0),
  ('contracts',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-contracts.py','--output',str(directory/'contracts.json')],0),
  ('progress',['python3','scripts/check-gate6-progress.py'],0),
  ('diff',['git','diff','--check'],0),
 ]
 steps=[]
 for name,command,minimum in commands:
  log=directory/(name+'.log')
  with log.open('w') as output:result=subprocess.run(command,cwd=ROOT,env=env,stdout=output,stderr=subprocess.STDOUT)
  raw=log.read_text(errors='replace');tests=re.findall(r'^test (\S+) \.\.\. ok$',raw,re.M);binaries={}
  for relative in re.findall(r'Running [^\n]+\((target/debug/deps/[^)]+)\)',raw):
   path=ROOT/relative
   if path.is_file():binaries[relative]=digest(path)
  step=dict(name=name,command=command,exit_code=result.returncode,tests_passed=tests,test_binaries_sha256=binaries,log=str(log.relative_to(ROOT)),log_sha256=digest(log),passed=result.returncode==0 and len(tests)>=minimum)
  if name in ('sdk-node','sdk-python'):
   match=re.search(r'(?:ℹ tests |# tests |Ran )(\d+)',raw);step['sdk_tests_passed']=int(match[1]) if match else 0;step['passed']=step['passed'] and step['sdk_tests_passed']>=2
  steps.append(step);print(json.dumps(dict(step=name,passed=step['passed'],rust_tests=len(tests),sdk_tests=step.get('sdk_tests_passed',0))),flush=True)
 apis={name:json.loads((ROOT/f'contracts/openapi-{name}.json').read_text()) for name in ('membership','conversations','operations','messages','provisioning')}
 checked=[];errors=[]
 for path in sorted(samples.glob('*.json')):
  sample=json.loads(path.read_text());name=sample.get('api','provisioning')
  if name not in apis:continue
  api=apis[name];route=api['paths'][sample['route']]['post']
  try:
   for value,schema in ((sample['request'],route['requestBody']['content']['application/json']['schema']),(sample['response'],route['responses'][str(sample['status'])]['content']['application/json']['schema'])):
    jsonschema.Draft202012Validator(dict(schema,components=api['components'])).validate(value)
   checked.append(dict(api=name,path=str(path.relative_to(ROOT)),sha256=digest(path),status=sample['status']))
  except (KeyError,jsonschema.ValidationError) as error:errors.append(dict(sample=path.name,error=str(error)))
 required_samples={f'membership-{name}.json' for name in ('invite-declined','declined','invite-cancelled','cancelled','invite-joined','joined','share','revoke-share','role','remove','add~again','leave','snapshot','invitations')}
 missing_samples=sorted(required_samples-{p.name for p in samples.glob('*.json')})
 required_tests={
  'all_membership_actions_share_receipts_across_mcp_native_http_and_public_sdks',
  'membership_http_and_mcp_require_named_scopes_and_closed_authority_fields',
  'actual_canonical_membership_unread_reply_rotation_and_http_native_replay_share_one_ledger',
  'membership_add_remove_reason_share_core_receipt_and_status',
  'membership_self_leave_receipt_does_not_revive_after_readmission',
  'membership_original_manager_epoch_fences_old_add_receipt',
  'membership_invitation_cursor_is_opaque_owner_bound_and_has_no_later_insertions',
  'membership_invitation_target_and_nonmember_administrator_receipts_are_explicit',
  'membership_business_rejection_is_retained_but_receipt_storage_failure_rolls_back_every_effect',
  'membership_concurrent_retry_is_one_receipt_and_restart_keeps_removal_reason_and_epoch',
  'gate6_backup_preserves_ids_audiences_unknown_effects_and_fences_live_claims',
  'gate6_incompatible_state_refusal_preserves_files_and_requires_explicit_new_path',
 }
 required_tests.update({'authority_fences::tests::membership_removal_and_other_member_revocation_wait_for_snapshot_delivery','authority_fences::tests::mutations::membership_and_provisioning_reuse_sufficient_write_proofs_without_reentering'})
 required_tests.update('peer_authority::tests::'+name for name in ('membership_peer_proof_binds_every_field_credential_and_installation_without_mutation','membership_peer_held_writer_finishes_before_queued_key_retirement_without_reentry','membership_peer_source_expiring_behind_sql_wait_rolls_back_effect_receipt_and_journal'))
 # Require the actual prior membership behavior suite, not merely a package test count.
 required_tests.update(re.findall(r'^fn (\w+)\(', (ROOT/'crates/connect-store/tests/gate6_membership.rs').read_text(), re.M))
 required_tests.discard('fixture')
 # Helpers are not tests: retain only #[test] functions from the prior policy file.
 prior=(ROOT/'crates/connect-store/tests/gate6_membership.rs').read_text()
 helper_names=set(re.findall(r'^fn (\w+)\(',prior,re.M))-set(re.findall(r'#\[test\]\s*fn (\w+)\(',prior))
 required_tests-=helper_names
 observed={test for step in steps for test in step['tests_passed']};missing_tests=sorted(required_tests-observed)
 reference=(ROOT/'contracts/openapi-membership.json').read_bytes();subprocess.run(['python3','scripts/generate-gate6-membership-reference.py'],cwd=ROOT,check=True);reference_reproducible=(ROOT/'contracts/openapi-membership.json').read_bytes()==reference
 after=sources();historical=ROOT/'docs/evidence/gate-6/conversation-control.json'
 report=dict(format=1,task='G6-037',implementation_status='in-progress',schema=34,qualification='Selected local macOS arm64 schema34 shared membership engine, original actor receipts, canonical Add/Update/Remove over explicit signed QUIC composition, all ten named MCP/native/HTTP/TypeScript/Python actions, current queries and failure/recovery. This does not qualify production signed native installation or full Gate6 acceptance.',platform=platform.platform(),architecture=platform.machine(),generated_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),source_inputs_sha256=before,sources_unchanged=before==after,steps=steps,rust_tests_passed=sum(len(s['tests_passed']) for s in steps),sdk_tests_passed=sum(s.get('sdk_tests_passed',0) for s in steps),required_tests=sorted(required_tests),missing_required_tests=missing_tests,http_openapi_samples=checked,missing_http_samples=missing_samples,contract_errors=errors,reference_reproducible=reference_reproducible,historical_checkpoint=dict(path=str(historical.relative_to(ROOT)),sha256=digest(historical),status='Historical schema33 sources; not a current-source qualification.'),ignored_test_disposition={'transitions::tests::crash_worker':'Invoked explicitly by required passing parent crash tests; standalone ignored worker is not counted as a pass.','installation_ownership::tests::process_worker':'Invoked by passing parent process/kernel ownership tests; not a standalone pass.','authority_fences::tests::process_worker':'Invoked by passing parent revocation/process lock tests; not a standalone pass.'},not_qualified=['Production canonical signed native SDK/listener setup, remaining canonical commands and subscriptions.','Complete per-underlying-write HTTP response custody and physical media teardown after logical membership retirement.','Remote/browser/live-provider profiles, independent conformance, publication and Gate6 acceptance.','Unchanged historical evidence remains historical; this runner reruns selected affected packages and SDKs, not every prior upstream/provider/media lane.'],acceptance_qualified=False,unqualified_candidates=[dict(path='.local/gate-6/membership-control-candidate-2.json',reason='Intermediate source changed while adding explicit malformed enum-type rejections in the TypeScript membership decoder. Array-to-string coercion is not a typed enum; final source reruns all selected checks.'),dict(path='.local/gate-6/membership-control-candidate-1.json',reason='Preserved complete intermediate mixed-source report, including original manifest and failures. It is not a final qualification.'),dict(path='.local/gate-6/membership-control-qualification-1.log',reason='Intermediate broad run found two callers reentering snapshot authority beneath existing guards. Added explicit held-authority query composition and updated composed callers. Service fixtures also needed the new optional receipt field in destructuring and the shared Outcome::Membership wrapper in operation status; source changed and this run cannot qualify the final candidate.'),dict(path='.local/gate-6/membership-control-facades-2.log',reason='New test helper gained observer argument before two compact call sites were updated; compile failed and no behavior pass is inferred.'),dict(path='.local/gate-6/membership-control-quic-1.log',reason='New fixture assumed a parse-time malformed envelope had trusted in_reply_to correlation. Signed member effects succeeded; no full pass inferred.'),dict(path='.local/gate-6/membership-control-quic-2.log',reason='Diagnostic repeat of the same malformed-envelope correlation fixture; fixed to assert signed wire parsing refusal before trusted correlation.')])
 report['status']='passed' if all(s['passed'] for s in steps) and not (missing_tests or missing_samples or errors) and before==after and reference_reproducible else 'failed'
 args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(status=report['status'],rust_tests=report['rust_tests_passed'],sdk_tests=report['sdk_tests_passed'],samples=len(checked),missing_tests=missing_tests,missing_samples=missing_samples,contract_errors=errors,sources_unchanged=before==after,output=str(args.output))),flush=True)
 return 0 if report['status']=='passed' else 1
if __name__=='__main__':raise SystemExit(main())
