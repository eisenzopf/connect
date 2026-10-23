#!/usr/bin/env python3
"""Qualify schema-34 canonical read reports and typed receipt observations, never Gate 6 acceptance."""
import argparse,datetime,hashlib,importlib.util,json,os,pathlib,platform,re,subprocess,sys
import jsonschema
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('subscription_checkpoint',ROOT/'scripts/check-gate6-canonical-subscriptions.py')
previous=importlib.util.module_from_spec(spec);spec.loader.exec_module(previous)
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def sources():
 result=previous.sources()
 for name in ('scripts/check-gate6-canonical-receipts.py','docs/developer/canonical-receipts.md','scripts/check-gate6-canonical-subscriptions.py','docs/developer/canonical-subscriptions.md','scripts/check-gate6-membership-control.py','scripts/generate-gate6-membership-reference.py','docs/developer/membership-control.md','docs/developer/canonical-service.md'):
  result[name]=digest(ROOT/name)
 return dict(sorted(result.items()))
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=pathlib.Path,default=ROOT/'docs/evidence/gate-6/canonical-receipts.json');parser.add_argument('--resume-candidate',type=pathlib.Path,help='Reuse fully passing recorded steps only when every compiled input is byte-identical and only this runner changed.');args=parser.parse_args()
 candidate=json.loads(args.resume_candidate.read_text()) if args.resume_candidate else None
 before=sources();reuse=None
 if candidate:
  assert candidate['sources_unchanged'] and all(step['passed'] and step['exit_code']==0 for step in candidate['steps'])
  assert not any(candidate[key] for key in ('missing_required_tests','missing_http_samples','contract_errors','inventory_errors'))
  assert candidate['source_revision']==subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
  assert candidate['platform']==platform.platform() and candidate['architecture']==platform.machine()
  assert set(candidate['source_inputs_sha256'])==set(before)
  changed=[name for name,value in before.items() if candidate['source_inputs_sha256'][name]!=value]
  assert changed==['scripts/check-gate6-canonical-receipts.py'],changed
  directory=(ROOT/candidate['steps'][0]['log']).parent
  reuse=dict(candidate=str(args.resume_candidate.relative_to(ROOT)),sha256=digest(args.resume_candidate),changed_source_inputs=changed,qualification='Every compiled/application/contract input is byte-identical to the candidate. All recorded command, log and compiled artifact hashes are revalidated. Only the verification runner changed; reference, sample, inventory and source checks execute again.')
 else:
  stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory=ROOT/'.local/gate-6/canonical-receipts'/stamp;directory.mkdir(parents=True)
 samples=directory/'http-samples';state=directory/'prism-state';state.mkdir(exist_ok=True)
 env=dict(os.environ,PYTHONPATH=str(ROOT/'sdk/python'),CONNECT_GATE6_HTTP_SAMPLES=str(samples),PRISM_TEST_STATE_DIR=str(state))
 cargo=['cargo','test','--no-fail-fast','--locked','--offline','-j1']
 upstream=cargo+['--target-dir','target','--manifest-path','.deps/rvoip/Cargo.toml']
 commands=[
  ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
  ('format',['bash','scripts/format.sh','--check'],0),
  ('upstream-format',['rustfmt','--edition','2021','--config','skip_children=true','--check']+[str(ROOT/'.deps/rvoip'/p) for p in ('crates/uctp/rvoip-uctp/src/v2/auth_dispatch.rs','crates/uctp/rvoip-uctp/src/v2/auth_dispatch/subscriptions.rs','crates/uctp/rvoip-uctp/src/v2/auth_peer/exchange.rs','crates/uctp/rvoip-uctp/tests/v2_auth_peer.rs')],0),
  ('upstream-signing',upstream+['-p','rvoip-auth-core','--lib','sig9421'],31),
  ('upstream-peer',upstream+['-p','rvoip-uctp','--lib','--test','v2_auth_dispatch','--test','v2_auth_peer','--test','v2_canonical_transports','--test','v2_step_up','--test','v2_auth_recovery','--test','v2_peer_admission'],242),
  ('sdk-build',['npm','run','build','--prefix','sdk/typescript'],0),
  ('sdk-format',['npm','run','format:check','--prefix','sdk/typescript'],0),
  ('sdk-node',['node','--test','sdk/tests/membership.test.mjs','sdk/tests/conversations.test.mjs','sdk/tests/control_receipts.test.mjs','sdk/tests/events.test.mjs','sdk/tests/clients.test.mjs'],0),
  ('sdk-python',['python3','-m','unittest','discover','-s','sdk/tests','-p','test_*.py'],0),
  ('protocol',cargo+['-p','connect-protocol'],33),
  ('store',cargo+['-p','connect-store'],456),
  ('service',cargo+['-p','connect-service'],39),
  ('mcp',cargo+['-p','connect-mcp'],7),
  ('server',cargo+['-p','connect-server','--test','gate6_canonical_operations','--test','gate6_events','--test','gate6_membership_control','--test','gate6_messages','--test','gate6_provisioning','--test','first_run_schema_boundary','--test','setup_vault_auth','--test','maintenance','--test','gate6_operations'],34),
  ('native-mcp-regression',cargo+['-p','connect-server','--test','gate4_agent_communications','--test','gate4_native_grants','--test','gate4_native_grant_lifecycle'],5),
  ('server-api-build',['cargo','check','--locked','--offline','-j1','-p','connect-server','--tests'],0),
  ('contracts',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-contracts.py','--output',str(directory/'contracts.json')],0),
  ('progress',['python3','scripts/check-gate6-progress.py'],0),
  ('diff',['git','diff','--check'],0),
 ]
 steps=[]
 for index,(name,command,minimum) in enumerate(commands):
  if candidate:
   step=candidate['steps'][index]
   assert step['name']==name and step['command']==command
   log=ROOT/step['log'];assert digest(log)==step['log_sha256']
   raw=log.read_text(errors='replace')
   assert re.findall(r'^test (\S+) \.\.\. ok$',raw,re.M)==step['tests_passed']
   assert len(step['tests_passed'])>=minimum
   if name in ('sdk-node','sdk-python'):
    count=re.search(r'(?:ℹ tests |# tests |Ran )(\d+)',raw)
    assert count and int(count[1])==step['sdk_tests_passed'] and int(count[1])>=2
   for relative,sha in step['test_binaries_sha256'].items():assert digest(ROOT/relative)==sha,relative
   steps.append(step);print(json.dumps(dict(step=name,passed=True,reused=True,artifacts_rehashed=len(step['test_binaries_sha256']))),flush=True)
   continue
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
 apis={name:json.loads((ROOT/f'contracts/openapi-{name}.json').read_text()) for name in ('membership','conversations','operations','messages','provisioning','events','attachments')}
 checked=[];errors=[];skipped=[]
 for path in sorted(samples.glob('*.json')):
  sample=json.loads(path.read_text());name=sample.get('api','provisioning')
  if name not in apis:
   skipped.append(dict(api=name,path=str(path.relative_to(ROOT)),sha256=digest(path),reason='Outside this runner selected API set; not counted as an OpenAPI pass.'));continue
  api=apis[name];route=api['paths'][sample['route']]['post']
  try:
   for value,schema in ((sample['request'],route['requestBody']['content']['application/json']['schema']),(sample['response'],route['responses'][str(sample['status'])]['content']['application/json']['schema'])):
    jsonschema.Draft202012Validator(dict(schema,components=api['components'])).validate(value)
   checked.append(dict(api=name,path=str(path.relative_to(ROOT)),sha256=digest(path),status=sample['status']))
  except (KeyError,jsonschema.ValidationError) as error:errors.append(dict(sample=path.name,error=str(error)))
 required_samples={'subscription-http-cursor.json','canonical-read-http-replay.json'} | {f'membership-{name}.json' for name in ('invite-declined','declined','invite-cancelled','cancelled','invite-joined','joined','share','revoke-share','role','remove','add~again','leave','snapshot','invitations')}
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
 inventory=json.loads((ROOT/'contracts/gate6-authority-writers.json').read_text())
 required_tests.add('native_and_http_share_snapshot_catchup_private_observations_and_restart_boundaries')
 inventory_errors=[]
 for group_id in ('schema34-canonical-subscriptions','schema34-canonical-read-receipts'):
  selected=next(g for g in inventory['qualified_groups'] if g['id']==group_id)
  required_tests.update(selected['required_tests'])
  for relative in selected['paths']:
   if not (ROOT/relative).is_file():inventory_errors.append('Missing '+relative)
  for function in selected['functions']:
   if not any(re.search(r'fn\s+'+re.escape(function)+r'\b',(ROOT/p).read_text()) for p in selected['paths'] if p.endswith('.rs')):inventory_errors.append('Missing function '+function)
 observed={test for step in steps for test in step['tests_passed']};missing_tests=sorted(required_tests-observed)
 reference_checks=[]
 for name,generator in (('membership','scripts/generate-gate6-membership-reference.py'),('events','scripts/generate-gate6-event-reference.py')):
  path=ROOT/f'contracts/openapi-{name}.json';reference=path.read_bytes();command=['python3',generator,'--check'];result=subprocess.run(command,cwd=ROOT)
  reference_checks.append(dict(api=name,command=command,exit_code=result.returncode,sha256=digest(path),unchanged=path.read_bytes()==reference))
 reference_reproducible=all(check['exit_code']==0 and check['unchanged'] for check in reference_checks)
 after=sources();historical=ROOT/'docs/evidence/gate-6/canonical-subscriptions.json'
 report=dict(format=1,task='G6-037',implementation_status='in-progress',schema=34,source_lock_patches=len(json.loads((ROOT/'dependencies/source-lock.json').read_text())['patches']),qualification='Selected local macOS arm64 schema34 canonical message.read through shared original-recipient authority, preferences and operation receipts; typed attributed receipt hints through bounded physical subscription delivery; fixed live deadlines, cursor-safe 63-fact pages, native/HTTP replay and signed local QUIC recovery. Production canonical signed native setup and Gate6 acceptance remain open.',platform=platform.platform(),architecture=platform.machine(),generated_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),source_inputs_sha256=before,sources_unchanged=before==after,steps=steps,rust_tests_passed=sum(len(s['tests_passed']) for s in steps),sdk_tests_passed=sum(s.get('sdk_tests_passed',0) for s in steps),required_tests=sorted(required_tests),missing_required_tests=missing_tests,http_openapi_samples=checked,skipped_http_samples=skipped,verification_reuse=reuse,reference_checks=reference_checks,missing_http_samples=missing_samples,contract_errors=errors,inventory_errors=inventory_errors,reference_reproducible=reference_reproducible,historical_checkpoint=dict(path=str(historical.relative_to(ROOT)),sha256=digest(historical),status='Historical schema34 subscription sources; not a current-source qualification.'),ignored_test_disposition={'transitions::tests::crash_worker':'Invoked explicitly by required passing parent crash tests; standalone ignored worker is not counted as a pass.','installation_ownership::tests::process_worker':'Invoked by passing parent process/kernel ownership tests; not a standalone pass.','authority_fences::tests::process_worker':'Invoked by passing parent revocation/process lock tests; not a standalone pass.'},not_qualified=['Production canonical signed native SDK/listener setup and remaining canonical command families.','Complete per-underlying-write HTTP response custody and physical media teardown after logical membership retirement.','Remote/browser/live-provider profiles, independent conformance, publication and Gate6 acceptance.','Unchanged historical evidence remains historical; this selected checkpoint covers affected packages and SDKs, not every prior upstream/provider/media lane.'],unqualified_candidates=[
 dict(path='.local/gate-6/read-receipts-store-tests-1.log',reason='Fixture omitted the preexisting queued fact when counting a report that adds delivered and read. Corrected the expected count from two to three; held-reader and expiry cases passed, but no complete pass inferred.'),
 dict(path='.local/gate-6/read-receipts-quic-1.log',reason='New integration child module needed an explicit path relative to its test crate root. Compile failed before behavior; fixed the test module path.'),
 dict(path='.local/gate-6/inactive-compression-2.log',reason='Historical artifact inventory used entries rather than artifacts; preparation failed before any file modification. Corrected inventory reader; no behavior qualification.'),
 dict(path='.local/gate-6/inactive-compression-1.json',reason='Lossless native filesystem compression of one inactive binary preserved its SHA, mode and path; compiler-capacity maintenance, not product behavior evidence.'),
 dict(path='.local/gate-6/inactive-compression-2.json',reason='Lossless native filesystem compression of 62 inactive binaries preserved every SHA, mode and logical path and recovered 12497285120 physical bytes. The prior selected source/log/binary closure was checked again before application edits.'),
 ],acceptance_qualified=False)

 report['status']='passed' if all(s['passed'] for s in steps) and not (missing_tests or missing_samples or errors or inventory_errors) and before==after and reference_reproducible else 'failed'
 args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(status=report['status'],rust_tests=report['rust_tests_passed'],sdk_tests=report['sdk_tests_passed'],samples=len(checked),missing_tests=missing_tests,missing_samples=missing_samples,contract_errors=errors,inventory_errors=inventory_errors,sources_unchanged=before==after,output=str(args.output))),flush=True)
 return 0 if report['status']=='passed' else 1
if __name__=='__main__':raise SystemExit(main())
