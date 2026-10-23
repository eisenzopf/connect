#!/usr/bin/env python3
"""Verify G6-038's private durable Connection preparation, not media admission."""
import argparse,datetime,importlib.util,json,os,re,subprocess,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('runtime',ROOT/'scripts/check-gate6-session-runtime.py');RUNTIME=importlib.util.module_from_spec(spec);spec.loader.exec_module(RUNTIME)
BASE=RUNTIME.BASE
STORE_CASES={
 'six_durable_slots_for_four_members_replay_one_offer_and_stable_core_mapping_without_readiness',
 'camera_spoof_endpoint_and_stale_revision_refusals_do_not_create_connections',
 'failed_offer_storage_rolls_back_connection_receipt_and_journal_then_original_id_recovers',
 'leaving_one_participant_fences_only_their_preparing_slots_and_preserves_original_receipts',
 'native_connection_preparation_requires_exact_current_device_key_and_store_proof',
 'native_connection_key_expiry_behind_sql_leaves_no_slot_offer_receipt_or_journal',
 'retiring_preparations_keep_their_capacity_charge_until_cleanup_is_verified',
}
PROTOCOL_CASES={'private_preparation_id_preserves_wire_operation_and_cannot_complete_its_exchange','native_offer_profile_limits_and_required_stream_fields_are_checked','normalized_offer_preserves_optional_extensions_and_codec_preference_order'}
REQUIRED=RUNTIME.REQUIRED|{'connection_control::tests::'+case for case in STORE_CASES|PROTOCOL_CASES}
def source_inputs():return RUNTIME.source_inputs()
def validate_facts(directory):
 from jsonschema import Draft202012Validator
 document=json.loads((ROOT/'contracts/openapi-events.json').read_text());schemas=document['components']['schemas']
 data=json.loads((directory/'connection-preparation-facts.json').read_text())
 for name,value in [('SnapshotPage',data['snapshot']),('EventPage',data['events'])]:
  schema={'$ref':'#/components/schemas/'+name,'components':{'schemas':schemas}};Draft202012Validator.check_schema(schema);Draft202012Validator(schema).validate(value)
 connections=[item['data'] for item in data['snapshot']['items'] if item['kind']=='connection']
 assert len(connections)==6 and {c['state'] for c in connections}=={'preparing'}
 events=[event for event in data['events']['events'] if event['data']['kind']=='connection_prepared']
 assert len(events)==6 and len({event['connection_id'] for event in events})==6
 assert len({c['participant_id'] for c in connections})==4
 assert all(event['session_id']==event['data']['data']['session_id'] and event['connection_id']==event['data']['data']['connection_id'] for event in events)
 assert len(data['prepared_results'])==6
 print(json.dumps(dict(passed=True,actual_store_connections=6,actual_store_preparation_events=6,participants=4,physical_connections_qualified=0)))
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--validate-facts',type=Path);parser.add_argument('--reuse-candidate',type=Path);args=parser.parse_args()
 if args.validate_facts:validate_facts(args.validate_facts);return
 reuse={};candidate=None
 if args.reuse_candidate:
  candidate=json.loads(args.reuse_candidate.read_text());assert candidate['sources_unchanged']
  current=source_inputs();changed={n for n in set(current)|set(candidate['source_inputs_sha256']) if current.get(n)!=candidate['source_inputs_sha256'].get(n)}
  assert changed<={'scripts/check-gate6-connection-preparation.py'},changed
  counts={'core-memberships':9,'protocol':15,'store':328,'service':18,'client':9,'runtime':34}
  for step in candidate['steps']:
   name=step['name']
   if name not in counts or not step['passed']:continue
   assert step['exit_code']==0 and len(step['tests'])==counts[name]
   assert BASE.sha(ROOT/step['log'])==step['log_sha256']
   for binary,digest in step['test_binaries_sha256'].items():
    assert BASE.sha(ROOT/binary)==digest
    info=(ROOT/(binary+'.d')).read_text();assert all(n not in info for n in changed)
   reuse[name]=(step,dict(candidate=str(args.reuse_candidate),sha256=BASE.sha(args.reuse_candidate),changed_unconsumed_verifier_inputs=sorted(changed),exact_command_sources_binary_and_log=True))
 directory=ROOT/'.local/gate-6/connection-preparation'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory.mkdir(parents=True);samples=directory/'samples';samples.mkdir();facts=directory/'facts';facts.mkdir()
 if candidate:
  for n,digest in candidate['actual_samples_sha256'].items():
   src=ROOT/n;assert BASE.sha(src)==digest
   target=facts if src.name=='connection-preparation-facts.json' else samples
   shutil.copy2(src,target/src.name)
 before=source_inputs();steps=[]
 env=dict(os.environ,CONNECT_GATE6_CONNECTION_PREPARATION_SAMPLES=str(facts),CONNECT_G6_CANONICAL_HTTP_SAMPLES=str(samples),CONNECT_G6_SESSION_DECISION_SAMPLES=str(samples),CONNECT_GATE6_HTTP_SAMPLES=str(samples))
 commands=[
 ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),('format',['bash','scripts/format.sh','--check'],0),
 ('core-memberships',['cargo','test','--locked','--offline','-j2','--target-dir','target','--manifest-path','.deps/rvoip/Cargo.toml','-p','rvoip-core','--features','rvoip-core/opus,rvoip-core/vcon-signing','--lib','relationships::tests','--','--test-threads=1'],9),
 ('protocol',['cargo','test','--locked','--offline','-j2','-p','connect-protocol','--lib','--','--test-threads=1'],15),
 ('store',['cargo','test','--locked','--offline','-j2','-p','connect-store','--lib','--','--test-threads=1'],328),
 ('service',['cargo','test','--locked','--offline','-j2','-p','connect-service','--lib','--','--test-threads=1'],18),
 ('client',['cargo','test','--locked','--offline','-j2','-p','connect-client','--lib','canonical::tests','--','--test-threads=1'],9),
 ('runtime',['cargo','test','--locked','--offline','-j2','-p','connect-server','--test','gate6_canonical_runtime','--test','gate6_canonical_transitions','--test','gate6_application_sdk','--','--test-threads=1'],34),
 ('event-reference',['python3','scripts/generate-gate6-event-reference.py','--check'],0),('http-reference',['python3','scripts/generate-canonical-http-openapi.py','--check'],0),
 ('facts',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-connection-preparation.py','--validate-facts',str(facts)],0),
 ('openapi',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-canonical-http.py','--validate-openapi',str(samples)],0),
 ('progress',['python3','scripts/check-gate6-progress.py'],0),('diff',['git','diff','--check'],0),]
 for name,command,minimum in commands:
  log=directory/(name+'.log')
  if name in reuse:
   assert command==reuse[name][0]['command'];shutil.copyfile(ROOT/reuse[name][0]['log'],log);result=subprocess.CompletedProcess(command,0)
  else:
   with log.open('w') as output:result=subprocess.run(command,cwd=ROOT,env=env,stdout=output,stderr=subprocess.STDOUT)
  raw=log.read_text(errors='replace');tests=BASE.STATUSES.passed_tests(raw);helpers=re.findall(r'^test (\S+) \.\.\. ignored.*$',raw,re.M)
  passed=result.returncode==0 and len(tests)>=minimum
  if minimum:passed=passed and 'test result: ok.' in raw and 'test result: FAILED' not in raw
  if name=='store':passed=passed and set(helpers)=={'authority_fences::tests::process_worker','installation_ownership::tests::process_worker','transitions::tests::crash_worker'}
  binaries={str((ROOT/p).relative_to(ROOT)):BASE.sha(ROOT/p) for p in re.findall(r'^\s+Running .+ \(([^)]+)\)$',raw,re.M)}
  steps.append(dict(name=name,command=command,passed=passed,exit_code=result.returncode,tests=tests,ignored_subprocess_helpers=helpers,log=str(log.relative_to(ROOT)),log_sha256=BASE.sha(log),test_binaries_sha256=binaries))
  if name in reuse:steps[-1]['reused_successful_unchanged_run']=reuse[name][1]
  print(json.dumps(dict(step=name,passed=passed,tests=len(tests))),flush=True)
  if not passed:break
 observed={test for step in steps for test in step['tests']};missing=sorted(REQUIRED-observed);unchanged=before==source_inputs()
 report=dict(task='G6-038',implementation_status='in-progress',scope='Local macOS arm64 private durable Connection preparation plus current native/HTTP control regression. Six preparing database rows and four participant kinds; no six physical Connection or media qualification.',passed=len(steps)==len(commands) and all(s['passed'] for s in steps) and unchanged and not missing,executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),schema_version=36,source_lock_patches=104,source_inputs_sha256=before,sources_unchanged=unchanged,steps=steps,required_tests=sorted(REQUIRED),missing_required_tests=missing,runtime_tests=sum(len(s['tests']) for s in steps),actual_samples_sha256={str(p.relative_to(ROOT)):BASE.sha(p) for p in sorted([*samples.glob('*.json'),*facts.glob('*.json')])},historical_checkpoint_sha256={name:BASE.sha(ROOT/'docs/evidence/gate-6'/name) for name in ('upstream.json','session-control.json','session-runtime.json')},not_qualified=['Public connection.offer negotiation/answer/confirmation and actual Core/native allocation are not installed by this checkpoint. The private preparation receipt never completes the wire exchange.','Four heterogeneous media participants, six physical Connections, current per-packet media policy, stream/teardown isolation and runtime reconciliation.','Linux/browser/remote/provider, full upstream qualification, independent conformance, load/performance, acceptance and publication.'],known_unresolved=['Historical gate6_canonical_operations: 14 retired generic facade failures remain with G6-059/G6-065.'])
 for name,key in [('facts','store_fact_validation'),('openapi','http_validation')]:
  if any(s['name']==name and s['passed'] for s in steps):report[key]=json.loads((directory/(name+'.log')).read_text())
 output=ROOT/'docs/evidence/gate-6/connection-preparation.json';output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ('passed','runtime_tests','sources_unchanged','missing_required_tests')}),flush=True);raise SystemExit(0 if report['passed'] else 1)
if __name__=='__main__':main()
