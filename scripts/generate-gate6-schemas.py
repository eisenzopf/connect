#!/usr/bin/env python3
"""Generate the frozen Gate 6 schema/vector kit from audited payloads and v2 rules."""
import argparse
import copy
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
SAFE_INTEGER = 9_007_199_254_740_991
ID = {'type':'string','minLength':1,'maxLength':128,'pattern':r'^[A-Za-z0-9._~-]+$'}
TOKEN = {**ID, 'maxLength':2048}
STATES = ['accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown']
EDGES = {
 'accepted':['preparing','cancelled','failed','interrupted'],
 'preparing':['ready','committing','cancelled','failed','interrupted','unknown'],
 'ready':['committing','cancelled','failed','interrupted'],
 'committing':['completed','failed','interrupted','unknown'],
 'interrupted':['preparing','completed','cancelled','failed','unknown'],
 'unknown':['completed','cancelled','failed'],
 'completed':[],'cancelled':[],'failed':[],
}

def obj(fields,required=None):
 # UCTP ignores unknown optional fields; authority derives from known typed
 # fields and authenticated context, never from an arbitrary JSON extension.
 return dict(type='object',properties=fields,required=list(fields) if required is None else required,additionalProperties=True)

def ref(name):return {'$ref':'#/$defs/'+name}

def array(items,minimum=0,maximum=256):
 return dict(type='array',items=items,minItems=minimum,maxItems=maximum,uniqueItems=True)

def rust_type(t):
 if t=='String':return {'type':'string'}
 if t=='bool':return {'type':'boolean'}
 if t=='f32':return {'type':'number'}
 if t in ('u16','u32','u64'):return {'type':'integer','minimum':0,'maximum':{'u16':65535,'u32':4294967295,'u64':SAFE_INTEGER}[t]}
 if t=='DateTime<Utc>':return {'type':'string','format':'date-time'}
 if t=='serde_json::Value':return {}
 if t.startswith('Option<'):return {'anyOf':[rust_type(t[7:-1]),{'type':'null'}]}
 if t.startswith('Vec<'):return array(rust_type(t[4:-1]))
 return ref(t)

def build():
 ledger=json.loads((ROOT/'contracts/uctp-gate-6-requirements.json').read_text())
 definitions={'Identifier':ID,'BodyEncoding':{'enum':['utf8','base64']},'ConversationPolicy':{'enum':['ephemeral','persistent']},'DataReliability':obj({'mode':{'const':'reliable_ordered'}})}
 for path in sorted((ROOT/'.deps/rvoip/crates/uctp/rvoip-uctp/src/payloads').glob('*.rs')):
  source=path.read_text()
  for m in re.finditer(r'pub struct (\w+)\s*\{(.*?)\n\}',source,re.S):
   fields={};required=[]
   for f in re.finditer(r'(?P<before>(?:(?:\s*#\[[^\n]+\]|\s*///[^\n]*)\n)*)\s*pub (\w+): ([^\n]+),',m[2]):
    fields[f[2]]=rust_type(f[3])
    if 'Option<' not in f[3] and 'default' not in f['before'] and f[2] not in ('from','by','by_participant'):
     required.append(f[2])
   definitions[m[1]]=obj(fields,required)
 # Baseline Rust payloads used strings for IDs. The v2 wire contract applies
 # the same bounded identifier rule inside payloads as in envelope routing.
 identifier_fields={'id','tenant_id','identity_id','participant_id','device_id','connid','strm_id','msg_id','from','by','by_participant','to_participant','via_connection','active_participant','from_participant','since_msg_id','in_reply_to_msg','cursor'}
 for definition in definitions.values():
  for key,value in definition.get('properties',{}).items():
   if key in identifier_fields:
    if value.get('type')=='string':definition['properties'][key]=ref('Identifier')
    elif value.get('anyOf',[{}])[0].get('type')=='string':definition['properties'][key]={'anyOf':[ref('Identifier'),{'type':'null'}]}
   elif key in ('participants_present','strm_ids'):
    definition['properties'][key]=array(ref('Identifier'))
 # G6-027: fetched integrity describes the exact projected delivery bytes.
 definitions['RecordingVconFetch']['properties']['uuid']={'anyOf':[{'type':'string','minLength':1,'maxLength':36},{'type':'null'}]}
 definitions['RecordingVconFetch']['properties']['url']={'anyOf':[{'type':'string','minLength':1,'maxLength':2048},{'type':'null'}]}
 definitions['RecordingVconFetch']['properties']['content_hash']={'anyOf':[{'type':'string','minLength':1,'maxLength':512},{'type':'null'}]}
 definitions['RecordingVconFetch']['anyOf']=[{'required':['uuid'],'properties':{'uuid':{'type':'string','minLength':1}}},{'required':['url'],'properties':{'url':{'type':'string','minLength':1}}}]
 definitions['RecordingVconFetched']['properties']['delivery']={'type':'string','enum':['inline','url']}
 definitions['RecordingVconFetched']['properties']['content_hash']={'type':'string','minLength':1,'maxLength':512,'description':'Digest of these exact authorized projected bytes; never the full source digest for a filtered view.'}
 definitions['RecordingVconFetched']['required'].append('content_hash')
 definitions['RecordingVconFetched']['oneOf']=[
  {'required':['vcon'],'properties':{'delivery':{'const':'inline'},'vcon':{'type':'string','minLength':1}},'not':{'anyOf':[{'required':['url']},{'required':['expires_at']}]}},
  {'required':['url','expires_at'],'properties':{'delivery':{'const':'url'},'url':{'type':'string','minLength':1,'maxLength':2048}},'not':{'required':['vcon']}}]
 definitions['AuthHello']['properties']['application_profiles']=array({'type':'string','minLength':1},1,16)
 # Cursors are bounded opaque transport tokens, not resource identifiers.
 for model,field in [('MessageHistory','cursor'),('ConversationList','cursor')]:
  definitions[model]['properties'][field]=copy.deepcopy(TOKEN)
 definitions['Ack']['properties']['next_cursor']=copy.deepcopy(TOKEN)
 definitions['AuthHello']['required'].append('application_profiles')
 definitions['AuthSession']['properties']['application_profile']={'const':'connect/2'}
 definitions['AuthSession']['required'].append('application_profile')
 # A transport peer may authenticate before selecting any Conversation.
 definitions['AuthSession']['required'].remove('participant_id')
 definitions['MessageSend']['properties']['to']={'oneOf':[{'const':'all'},array(ref('Identifier'),1)]}
 definitions['MessageSend']['properties']['content_type']={'type':'string','minLength':1,'maxLength':255}
 definitions['MessageSend']['properties']['body']={'type':'string','maxLength':87384,'description':'Runtime enforces 64 KiB decoded UTF-8/binary bytes, independently of JSON character length.'}
 definitions['SessionTarget']={'anyOf':[ref('Identifier'),{'type':'string','minLength':5,'maxLength':2048,'pattern':r'^(sip|sips|tel):[!-~]+$'}]}
 definitions['SessionInvite']['properties']['to']=array(ref('SessionTarget'))
 definitions['StreamInfo']['properties']['stream_local_id']={'type':'integer','minimum':1,'maximum':65535}
 definitions['StreamSubscription']['oneOf']=[{'required':['strm_id'],'not':{'anyOf':[{'required':['from_participant']},{'required':['kinds']}]}},{'required':['from_participant'],'not':{'required':['strm_id']}}]
 definitions['ReadyStreamBinding']=obj({k:v for k,v in definitions['StreamInfo']['properties'].items() if k!='opened_at'})
 definitions['ConnectionReady']=obj({'streams':array(ref('ReadyStreamBinding'),1)})
 # Target-owned native isolation is distinct from an executable handoff or
 # decoded readiness. Its signed profile has no caller reference or ready bit.
 definitions['HandoffPreparation']={'type':'object','properties':{'profile':{'const':'connect-handoff-preparation-v1'}},'required':['profile'],'additionalProperties':False}
 definitions['ConnectionOffer']['properties']['handoff_preparation']=ref('HandoffPreparation')
 definitions['ConnectionOffer'].setdefault('allOf',[]).append({'if':{'required':['handoff_preparation']},'then':{'not':{'required':['private_consultation']},'properties':{'streams_offered':{'type':'array','minItems':1,'maxItems':1,'items':{'allOf':[ref('StreamOffer'),{'properties':{'kind':{'const':'audio'},'direction':{'const':'sendrecv'}}}]}}}}})
 definitions['SessionUpdate']['properties']['kind']={'enum':['participant-leave','role-change','medium-upgrade','codec-renegotiate']}
 definitions['SessionUpdate']['properties']['details']={'type':'object'}
 definitions['SessionUpdate']['required'].append('details')
 definitions['SessionUpdate']['allOf']=[]
 for kind,fields in [('participant-leave',{'participant_id':ref('Identifier')}),('role-change',{'participant_id':ref('Identifier'),'role':{'type':'string','minLength':1,'maxLength':64}}),('medium-upgrade',{'media':array({'enum':['audio','video','screen','data']},1)}),('codec-renegotiate',{'codec_preferences':array({'type':'string','minLength':1},1)})]:
  definitions['SessionUpdate']['allOf'].append({'if':{'properties':{'kind':{'const':kind}}},'then':{'properties':{'details':obj(fields)}}})
 definitions['ConnectionUpdate']['properties']['action']={'enum':['hold','resume','mute','unmute','codec-renegotiate']}
 definitions['ConnectionUpdate']['properties']['action']['enum'].append('quality')
 definitions['ConnectionUpdate']['properties']['action']['enum'].append('ice-restart')
 definitions['ConnectionUpdate']['properties']['quality_interval_ms']={'type':'integer','minimum':100,'maximum':60000}
 definitions['ConnectionUpdate']['allOf']=[{'if':{'required':['action'],'properties':{'action':{'const':'quality'}}},'then':{'required':['quality_interval_ms'],'not':{'anyOf':[{'required':[key]} for key in ['streams','codec_preferences','details']]}},'else':{'not':{'required':['quality_interval_ms']}}}]
 ice_setup={'type':'object','properties':{'kind':{'const':'websocket+webrtc'},'sdp':{'type':'string','minLength':5,'maxLength':524288}},'required':['kind','sdp'],'additionalProperties':False}
 ice_details={'type':'object','properties':{'expected_ice_generation':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER-1},'setup':ice_setup},'required':['expected_ice_generation','setup'],'additionalProperties':False}
 definitions['ConnectionUpdate']['allOf'].append({'if':{'required':['action'],'properties':{'action':{'const':'ice-restart'}}},'then':{'type':'object','properties':{'action':{'const':'ice-restart'},'details':ice_details},'required':['action','details'],'additionalProperties':False}})
 definitions['ConnectionQuality']['properties']['interval_ms']['minimum']=1
 # An absent measurement is unavailable, never a fabricated zero. Packets count
 # accepted/submitted RTP datagrams, not reliable data messages or remote playout.
 definitions['StreamQuality']['required']=['strm_id','bitrate_bps','packets_sent','packets_received']
 definitions['StreamQuality']['properties']['loss_pct'].update(minimum=0,maximum=100)
 definitions['StreamQuality']['properties']['mos'].update(minimum=1,maximum=5)
 definitions['StreamQuality']['properties']['bitrate_bps']['maximum']=SAFE_INTEGER

 for name in ['StreamInfo','ReadyStreamBinding','StreamOffer','StreamAnswer']:
  definitions[name]['properties']['kind']={'enum':['audio','video','screen','data']}
  definitions[name]['properties']['direction']={'enum':['sendrecv','sendonly','recvonly','inactive']}
 definitions['StreamActiveSpeaker']['required']=['active_participant','strm_id','changed_at']
 definitions['OperationState']={'enum':STATES}
 definitions['OperationUpdated']=obj({'operation_id':ref('Identifier'),'record_id':ref('Identifier'),'subscription_record_id':ref('Identifier'),'command_type':{'type':'string','minLength':1,'maxLength':128},'expected_revision':{'anyOf':[{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},{'type':'null'}]},'state':ref('OperationState'),'revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'result':{},'problem':{'type':'object'}},['operation_id','state','revision'])
 definitions['OperationUpdated']['allOf']=[{'if':{'properties':{'state':{'not':{'const':'completed'}}}},'then':{'not':{'required':['result']}}}]
 definitions['OperationUpdated']['allOf'].append({'if':{'required':['command_type'],'properties':{'command_type':{'enum':['session.consult','session.consult.return','session.handoff','connection.replace']}}},'then':{'required':['record_id','expected_revision'],'properties':{'expected_revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER-1}}}})
 # A current-authorized server reconciliation status binds the callback to
 # its original explicit subscription without exposing the private engine plan.
 definitions['OperationUpdated']['allOf'].append({'if':{'required':['command_type'],'properties':{'command_type':{'const':'stream.reconcile'}}},'then':{'required':['record_id','subscription_record_id','expected_revision'],'properties':{'expected_revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER-1}}}})
 definitions['OperationUpdated']['allOf'].append({'if':{'required':['subscription_record_id']},'then':{'required':['command_type'],'properties':{'command_type':{'const':'stream.reconcile'}}}})
 definitions['ReconciliationHint']=obj({'operation_id':ref('Identifier'),'record_id':ref('Identifier'),'command_type':{'const':'stream.reconcile'},'expected_revision':{'type':'integer','minimum':1,'maximum':SAFE_INTEGER-1},'state':{'const':'completed'},'revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'subscription_record_id':ref('Identifier'),'stream_revision':{'type':'integer','minimum':1,'maximum':SAFE_INTEGER},'journal_hint':{'const':True},'resources_changed':{'const':False}})
 definitions['ReconciliationHint']['additionalProperties']=False
 definitions['OperationUpdated']['allOf'].append({'if':{'required':['journal_hint'],'properties':{'journal_hint':{'const':True}}},'then':ref('ReconciliationHint')})
 definitions['MemberAdd']=obj({'identity_id':ref('Identifier'),'kind':{'enum':['human','ai','system','external']},'role':{'type':'string','minLength':1,'maxLength':64}})
 definitions['MemberUpdate']=obj({'participant_id':ref('Identifier'),'role':{'type':'string','minLength':1,'maxLength':64}})
 definitions['MemberRemove']=obj({'participant_id':ref('Identifier'),'reason':{'type':'string','maxLength':512}},['participant_id'])
 definitions['MemberChanged']=obj({'participant_id':ref('Identifier'),'kind':{'enum':['added','updated','removed']},'membership_epoch':{'type':'integer','minimum':1,'maximum':SAFE_INTEGER}})
 definitions['SessionConsult']=obj({'retained_connid':ref('Identifier'),'private_participants':array(ref('Identifier'),2),'hold_treatment':{'enum':['silence','configured-media']}},['retained_connid','private_participants'])
 definitions['SessionConsultReturn']=obj({'consultation_record_id':ref('Identifier'),'retained_connid':ref('Identifier')})
 definitions['SessionHandoff']=obj({'source_participant':ref('Identifier'),'target_participant':ref('Identifier'),'retained_connid':ref('Identifier'),'candidate_connid':ref('Identifier'),'outgoing':{'enum':['leave','observe']},'role_scope':{'enum':['conversation','session']},'expected_conversation_revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER}},['source_participant','target_participant','retained_connid','outgoing'])
 definitions['SessionHandoff']['allOf']=[{'if':{'required':['role_scope'],'properties':{'role_scope':{'const':'session'}}},'then':{},'else':{'required':['expected_conversation_revision']}}]
 definitions['ConnectionReplace']=obj({'endpoint_id':ref('Identifier'),'candidate_connid':ref('Identifier'),'retained_connid':ref('Identifier')},['retained_connid'])
 definitions['ConnectionReplace']['oneOf']=[{'required':['endpoint_id'],'not':{'required':['candidate_connid']}},{'required':['candidate_connid'],'not':{'required':['endpoint_id']}}]
 definitions['OperationGet']=obj({'operation_id':ref('Identifier')})
 definitions['OperationCancel']=obj({'operation_id':ref('Identifier'),'reason':{'type':'string','maxLength':512}},['operation_id'])
 definitions['OperationReconcile']=obj({'operation_id':ref('Identifier'),'outcome':{'enum':['completed','cancelled','failed']},'evidence_id':ref('Identifier')})
 definitions['ConversationGet']=obj({'page':copy.deepcopy(TOKEN),'limit':{'type':'integer','minimum':1,'maximum':127}},[])
 definitions['ConversationSubscribe']=obj({'after':copy.deepcopy(TOKEN),'live':{'type':'boolean'},'limit':{'type':'integer','minimum':1,'maximum':500},'lease_seconds':{'type':'integer','minimum':1,'maximum':30}},[])
 definitions['ConversationSnapshot']=obj({'revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'cursor':copy.deepcopy(TOKEN),'items':array({'type':'object'},0,500),'has_more':{'type':'boolean'}})
 definitions['ConversationSnapshot']['properties']['next_page']=copy.deepcopy(TOKEN)
 definitions['ConversationEvent']=obj({'event_id':ref('Identifier'),'cursor':copy.deepcopy(TOKEN),'kind':{'type':'string','minLength':1},'revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'data':{'type':'object'}})
 definitions['SessionObservation']=obj({'kind':{'enum':['transcript','speech','action-request']},'participant_id':ref('Identifier'),'source':{'type':'string','minLength':1},'correlation_id':ref('Identifier'),'data':{'type':'object'}})
 kinds={e['type']:e['payload_struct'] or 'ConnectionReady' for e in ledger['envelopes']}
 kinds.update({'conversation.get':'ConversationGet','conversation.participant.add':'MemberAdd','conversation.participant.update':'MemberUpdate','conversation.participant.remove':'MemberRemove','conversation.participant.changed':'MemberChanged','conversation.subscribe':'ConversationSubscribe','conversation.snapshot':'ConversationSnapshot','conversation.event':'ConversationEvent','session.consult':'SessionConsult','session.handoff':'SessionHandoff','connection.replace':'ConnectionReplace','operation.get':'OperationGet','operation.cancel':'OperationCancel','operation.reconcile':'OperationReconcile','operation.updated':'OperationUpdated','session.observation':'SessionObservation'})
 kinds['session.consult.return']='SessionConsultReturn'
 mutations={'conversation.create','conversation.close','conversation.participant.add','conversation.participant.update','conversation.participant.remove','session.invite','session.accept','session.reject','session.cancel','session.end','session.update','session.consult','session.consult.return','session.handoff','connection.offer','connection.answer','connection.update','connection.end','connection.replace','stream.subscribe','stream.unsubscribe','message.send','message.read','dtmf.send','operation.cancel','operation.reconcile'}
 revisioned={'conversation.participant.add','conversation.participant.update','conversation.participant.remove','session.update','session.consult','session.consult.return','session.handoff','connection.replace','connection.update','stream.subscribe','stream.unsubscribe'}
 definitions['Signature']=obj({'keyid':{'type':'string','minLength':1},'alg':{'enum':['EdDSA','ES256','PS256','RS256']},'sig':{'type':'string','minLength':1}})
 fields={'v':{'const':2},'type':{'enum':list(kinds)},'id':ref('Identifier'),'ts':{'type':'string','format':'date-time'},'payload':{'type':'object'},'cid':ref('Identifier'),'sid':ref('Identifier'),'connid':ref('Identifier'),'in_reply_to':ref('Identifier'),'operation_id':ref('Identifier'),'expected_revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'signature':ref('Signature')}
 schema={'$schema':'https://json-schema.org/draft/2020-12/schema','$id':'https://rudeless.ai/connect/contracts/uctp-envelope-2.schema.json','title':'Gate 6 UCTP envelope / connect/2 target','description':'Schema conformance does not establish authorization, active media, provider effects or runtime qualification.',**obj(fields,['v','type','id','ts','payload']),'$defs':definitions,'allOf':[]}
 for name,kind in kinds.items():
  command_required=['operation_id'] if name in mutations else []
  if name in revisioned:command_required.append('expected_revision')
  required=[]
  if name.startswith(('session.','connection.','stream.','dtmf.','recording.')):required.extend(['cid','sid'])
  elif name.startswith(('conversation.','message.')) and name not in ('conversation.create','conversation.list'):required.append('cid')
  if name.startswith(('connection.','stream.','dtmf.')) and name!='stream.active-speaker':required.append('connid')
  if name=='session.consult.return':required.append('connid')
  then={'properties':{'payload':ref(kind)},'required':required}
  if command_required:then['anyOf']=[{'required':command_required},{'required':['in_reply_to']}]
  if name=='auth.session':
   then['allOf']=[{'if':{'properties':{'payload':{'required':['participant_id']}}},'then':{'required':['cid']}}]
  schema['allOf'].append({'if':{'properties':{'type':{'const':name}}},'then':then})
 schema['allOf'].append({'if':{'required':['type','payload'],'properties':{'type':{'const':'operation.updated'},'payload':{'required':['journal_hint'],'properties':{'journal_hint':{'const':True}}}}},'then':{'required':['cid','sid','connid','operation_id'],'not':{'anyOf':[{'required':['in_reply_to']},{'required':['expected_revision']}]}}})
 # All referenced definitions must be known before vectors can be produced.
 def check_refs(node):
  if isinstance(node,dict):
   if '$ref' in node:assert node['$ref'].split('/')[-1] in definitions,node['$ref']
   for value in node.values():check_refs(value)
  elif isinstance(node,list):
   for value in node:check_refs(value)
 check_refs(schema)
 return schema,kinds,mutations,revisioned

def sample(node,definitions):
 if '$ref' in node:return sample(definitions[node['$ref'].split('/')[-1]],definitions)
 if 'const' in node:return node['const']
 if 'enum' in node:return node['enum'][0]
 if 'oneOf' in node and 'properties' not in node:return sample(node['oneOf'][0],definitions)
 if 'anyOf' in node:return sample(node['anyOf'][0],definitions)
 if node.get('format')=='date-time':return '2026-10-15T12:00:00Z'
 if node.get('type')=='string':return 'example'
 if node.get('type') in ('integer','number'):return node.get('minimum',0)
 if node.get('type')=='boolean':return False
 if node.get('type')=='array':
  items=[sample(node['items'],definitions) for _ in range(node.get('minItems',0))]
  for i,item in enumerate(items):
   if isinstance(item,str):items[i]=item+str(i)
  return items
 if node.get('type')=='object':return {k:sample(node['properties'][k],definitions) for k in node.get('required',[])}
 return {}

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--check',action='store_true',help='Refuse stale generated files without changing them')
 args=parser.parse_args()
 def emit(path, text):
  if args.check:
   if not path.is_file() or path.read_text()!=text:raise SystemExit(f'{path.relative_to(ROOT)} is stale')
  else:path.write_text(text)
 schema,kinds,mutations,revisioned=build()
 directory=ROOT/'contracts/schemas';directory.mkdir(exist_ok=True)
 emit(directory/'uctp-envelope-2.schema.json', json.dumps(schema,indent=2)+'\n')
 vectors=[]
 for i,(name,kind) in enumerate(kinds.items()):
  value={'v':2,'type':name,'id':f'env_vector_{i}','ts':'2026-10-15T12:00:00Z','cid':'conv_example','sid':'sess_example','connid':'conn_example','payload':sample(schema['$defs'][kind],schema['$defs'])}
  if name in mutations:value['operation_id']=f'op_vector_{i}'
  if name in revisioned:value['expected_revision']=1
  if name=='stream.active-speaker':value.pop('connid')
  if name=='recording.vcon-fetch':value['payload']['uuid']='00000000-0000-8000-8000-000000000001'
  if name=='recording.vcon-fetched':value['payload'].update(delivery='inline',vcon='{}',content_hash='sha512-fixture-projected-bytes')
  if name=='connection.replace':value['payload']['endpoint_id']='endpoint_example'
  if name=='session.update':value['payload']['details']={'participant_id':'part_example'}
  if name=='session.handoff':value['payload'].update(source_participant='part_source',target_participant='part_target',retained_connid='conn_remote',expected_conversation_revision=1)
  vectors.append(dict(id='schema-'+name,scope='schema-only',valid=True,envelope=value))
  invalid=copy.deepcopy(value);invalid['v']=1
  vectors.append(dict(id='schema-unsupported-version-'+name,scope='schema-only',valid=False,envelope=invalid))
  if name in mutations:
   invalid=copy.deepcopy(value);invalid.pop('operation_id')
   vectors.append(dict(id='schema-missing-operation-'+name,scope='schema-only',valid=False,envelope=invalid))
  if name in revisioned:
   invalid=copy.deepcopy(value);invalid.pop('expected_revision')
   vectors.append(dict(id='schema-missing-revision-'+name,scope='schema-only',valid=False,envelope=invalid))
 speaker=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-stream.active-speaker')
 speaker['payload']['active_participant']=None
 vectors.append(dict(id='active-speaker-session-silence',scope='schema-only',valid=True,envelope=copy.deepcopy(speaker)))
 invalid=copy.deepcopy(speaker);invalid['payload'].pop('active_participant')
 vectors.append(dict(id='active-speaker-needs-explicit-participant-or-null',scope='schema-only',valid=False,envelope=invalid))
 invalid=copy.deepcopy(speaker);invalid['payload']['active_participant']=37
 vectors.append(dict(id='active-speaker-refuses-numeric-participant',scope='schema-only',valid=False,envelope=invalid))
 invalid=copy.deepcopy(speaker);invalid.pop('sid')
 vectors.append(dict(id='active-speaker-needs-session',scope='schema-only',valid=False,envelope=invalid))
 solo=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-session.invite')
 solo['payload']['to']=[]
 vectors.append(dict(id='session-invite-implicit-solo-initiator',scope='schema-only',valid=True,envelope=solo))
 # Gateway references belong only to Session invitation selectors.
 for label,targets,valid in [('sip',['sip:bob@example.com'],True),('tel',['tel:+14155550123'],True),('mixed',['part_one','id_two','sip:bob@example.com'],True),('unknown-scheme',['https://example.com'],False),('whitespace',['sip:bob@exa mple.com'],False),('overlong',['sip:'+('x'*2045)],False)]:
  item=copy.deepcopy(solo);item['payload']['to']=targets
  vectors.append(dict(id='session-invite-gateway-'+label,scope='schema-only',valid=valid,envelope=item))
 item=copy.deepcopy(solo);item['cid']='sip:bob@example.com'
 vectors.append(dict(id='session-invite-gateway-is-not-conversation-id',scope='schema-only',valid=False,envelope=item))
 # Preserve the original embedded invitation refusal vector IDs.
 for label,targets in [('duplicates',['part-one','part-one']),('null',None),('over-capacity',[f'part-{i}' for i in range(257)])]:
  invalid=copy.deepcopy(solo)
  invalid['payload'].update(to=targets,medium='voice',intent='solo',capabilities_offer={'audio':['g.711-mu']})
  vectors.append(dict(id='session-invite-'+label,scope='schema-only',valid=False,envelope=invalid))
 auth=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-auth.session')
 for field in ['cid','sid','connid']:auth.pop(field,None)
 vectors.append(dict(id='auth-unscoped-identity-only',scope='schema-only',valid=True,envelope=copy.deepcopy(auth)))
 auth['payload']['participant_id']='participant_example'
 vectors.append(dict(id='auth-participant-needs-conversation',scope='schema-only',valid=False,envelope=copy.deepcopy(auth)))
 auth['cid']='conversation_example'
 vectors.append(dict(id='auth-scoped-participant',scope='schema-only',valid=True,envelope=auth))
 # Correlation is syntax, not authorization: runtimes route replies exclusively
 # through authenticated exact-scope waiters, never ordinary command execution.
 replay=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-message.send')
 replay.pop('operation_id');replay['in_reply_to']='history_request'
 vectors.append(dict(id='history-correlated-message-observation',scope='schema-only',valid=True,envelope=copy.deepcopy(replay)))
 replay['in_reply_to']=''
 vectors.append(dict(id='empty-correlation-is-not-a-reply',scope='schema-only',valid=False,envelope=replay))
 query=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-message.history')
 query['payload']['cursor']='a'*1500
 vectors.append(dict(id='history-opaque-cursor-distinct-from-resource-id',scope='schema-only',valid=True,envelope=copy.deepcopy(query)))
 query['payload']['cursor']='a'*2049
 vectors.append(dict(id='history-cursor-bound',scope='schema-only',valid=False,envelope=query))
 ack=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-ack')
 ack['payload']['next_cursor']='a'*1500
 vectors.append(dict(id='ack-opaque-next-cursor',scope='schema-only',valid=True,envelope=ack))
 for token,valid,name in [('a'*2048,True,'maximum'),('',False,'empty'),('a/b',False,'slash'),('é',False,'non-ascii')]:
  query=copy.deepcopy(next(v['envelope'] for v in vectors if v['id']=='schema-message.history'))
  query['payload']['cursor']=token
  vectors.append(dict(id='history-cursor-'+name,scope='schema-only',valid=valid,envelope=query))
 for length,valid in [(128,True),(129,False)]:
  query=copy.deepcopy(next(v['envelope'] for v in vectors if v['id']=='schema-message.history'))
  query['cid']='a'*length
  vectors.append(dict(id='resource-identifier-'+str(length),scope='schema-only',valid=valid,envelope=query))
 quality=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-connection.quality')
 quality['payload']={'interval_ms':5000,'streams':[{'strm_id':'audio','bitrate_bps':256000,'packets_sent':25,'packets_received':25}]}
 vectors.append(dict(id='quality-unavailable-metrics-omitted',scope='schema-only',valid=True,envelope=copy.deepcopy(quality)))
 for loss,mos in [(0.0,1.0),(100.0,5.0),(0.4,4.1)]:
  measured=copy.deepcopy(quality);measured['payload']['streams'][0].update(loss_pct=loss,mos=mos,jitter_ms=12,rtt_ms=80)
  vectors.append(dict(id='quality-measured-'+str(loss)+'-'+str(mos),scope='schema-only',valid=True,envelope=measured))

 for field,value in [('interval_ms',0),('loss_pct',-1),('loss_pct',101),('loss_pct',-0.1),('loss_pct',100.1),('mos',0),('mos',6),('mos',0.9),('mos',5.1),('packets_received',-1),('rtt_ms',-1)]:
  invalid=copy.deepcopy(quality)
  if field=='interval_ms':invalid['payload'][field]=value
  else:invalid['payload']['streams'][0][field]=value
  vectors.append(dict(id='quality-invalid-'+field+'-'+str(value),scope='schema-only',valid=False,envelope=invalid))
 tune=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-connection.update')
 tune['payload']={'action':'quality','quality_interval_ms':100}
 vectors.append(dict(id='quality-interval-update',scope='schema-only',valid=True,envelope=copy.deepcopy(tune)))
 for field,value in [('quality_interval_ms',99),('quality_interval_ms',60001),('streams',['audio']),('details',{})]:
  invalid=copy.deepcopy(tune);invalid['payload'][field]=value
  vectors.append(dict(id='quality-invalid-update-'+field+'-'+str(value),scope='schema-only',valid=False,envelope=invalid))
 invalid=copy.deepcopy(tune);del invalid['payload']['quality_interval_ms']
 vectors.append(dict(id='quality-update-needs-interval',scope='schema-only',valid=False,envelope=invalid))
 fetched=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-recording.vcon-fetched')
 invalid=copy.deepcopy(fetched);invalid['payload'].pop('content_hash');vectors.append(dict(id='record-fetched-needs-projected-digest',scope='schema-only',valid=False,envelope=invalid))
 invalid=copy.deepcopy(fetched);invalid['payload']['url']='https://records.invalid/ref';vectors.append(dict(id='record-fetched-inline-url-exclusive',scope='schema-only',valid=False,envelope=invalid))
 url=copy.deepcopy(fetched);url['payload']={'delivery':'url','url':'https://records.invalid/ref','expires_at':'2026-10-15T12:01:00Z','content_hash':'sha512-fixture-projected-bytes'}
 vectors.append(dict(id='record-fetched-authorized-url-shape',scope='schema-only',valid=True,envelope=copy.deepcopy(url)))
 url['payload'].pop('expires_at');vectors.append(dict(id='record-fetched-url-needs-expiry',scope='schema-only',valid=False,envelope=url))
 query=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-recording.vcon-fetch');query['payload']={};vectors.append(dict(id='record-fetch-needs-local-selector',scope='schema-only',valid=False,envelope=query))
 status=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-operation.updated')
 status['payload']['record_id']='opr_record_reference'
 vectors.append(dict(id='operation-status-stable-record-reference',scope='schema-only',valid=True,envelope=status))
 identity=copy.deepcopy(status);identity['payload']['command_type']='session.consult';identity['payload']['expected_revision']=0
 vectors.append(dict(id='operation-transition-original-identity',scope='schema-only',valid=True,envelope=copy.deepcopy(identity)))
 for label,field,value in [('missing-record','record_id',None),('missing-revision','expected_revision',None),('null-revision','expected_revision','null'),('unsafe-revision','expected_revision',SAFE_INTEGER)]:
  invalid=copy.deepcopy(identity)
  if value is None:invalid['payload'].pop(field)
  else:invalid['payload'][field]=None if value=='null' else value
  vectors.append(dict(id='operation-transition-identity-'+label,scope='schema-only',valid=False,envelope=invalid))

 reconciliation=copy.deepcopy(identity);reconciliation['payload'].update(command_type='stream.reconcile',subscription_record_id='opr_explicit_subscription',state='preparing')
 vectors.append(dict(id='operation-reconciliation-original-subscription',scope='schema-only',valid=True,envelope=copy.deepcopy(reconciliation)))
 for label in ['missing-parent','invalid-parent','missing-record','missing-revision','null-revision','foreign-command']:
  invalid=copy.deepcopy(reconciliation);payload=invalid['payload']
  if label=='missing-parent':payload.pop('subscription_record_id')
  elif label=='invalid-parent':payload['subscription_record_id']='invalid/record'
  elif label=='missing-record':payload.pop('record_id')
  elif label=='missing-revision':payload.pop('expected_revision')
  elif label=='null-revision':payload['expected_revision']=None
  else:payload['command_type']='stream.subscribe'
  vectors.append(dict(id='operation-reconciliation-'+label,scope='schema-only',valid=False,envelope=invalid))

 hint=copy.deepcopy(reconciliation);hint['operation_id']=hint['payload']['operation_id'];hint.pop('in_reply_to',None);hint.pop('expected_revision',None)
 hint['payload'].update(journal_hint=True,resources_changed=False,stream_revision=3,expected_revision=2,state='completed')
 vectors.append(dict(id='routing-journal-hint-shape',scope='schema-only',valid=True,envelope=copy.deepcopy(hint)))
 for label in ['missing-parent','missing-root-operation','reply-header','assertion-header','result','plan','resources','state','unsafe-counter']:
  invalid=copy.deepcopy(hint)
  if label=='missing-parent':invalid['payload'].pop('subscription_record_id')
  elif label=='missing-root-operation':invalid.pop('operation_id')
  elif label=='reply-header':invalid['in_reply_to']='invented-request'
  elif label=='assertion-header':invalid['expected_revision']=2
  elif label=='result':invalid['payload']['result']={'unproven':'capture'}
  elif label=='plan':invalid['payload']['plan']={'private':'descriptor'}
  elif label=='resources':invalid['payload']['resources_changed']=True
  elif label=='state':invalid['payload']['state']='preparing'
  else:invalid['payload']['stream_revision']=SAFE_INTEGER+1
  vectors.append(dict(id='routing-journal-hint-'+label,scope='schema-only',valid=False,envelope=invalid))

 prepared=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-connection.offer')
 prepared['payload']['handoff_preparation']={'profile':'connect-handoff-preparation-v1'}
 prepared['payload']['streams_offered']=[{'id':'prepared-voice','kind':'audio','direction':'sendrecv','codec_preferences':['g.711-mu']}]
 vectors.append(dict(id='handoff-target-owned-preparation',scope='schema-only',valid=True,envelope=copy.deepcopy(prepared)))
 for label in ['wrong-profile','asserted-ready','mixed-private','video','one-way','multiple-streams']:
  invalid=copy.deepcopy(prepared);p=invalid['payload']
  if label=='wrong-profile':p['handoff_preparation']['profile']='unrecognized'
  elif label=='asserted-ready':p['handoff_preparation']['ready']=True
  elif label=='mixed-private':p['private_consultation']={'profile':'connect-private-audio-v1','operation_record_id':'copied'}
  elif label=='video':p['streams_offered'][0]['kind']='video'
  elif label=='one-way':p['streams_offered'][0]['direction']='sendonly'
  else:p['streams_offered'].append({**p['streams_offered'][0],'id':'second'})
  vectors.append(dict(id='handoff-preparation-'+label,scope='schema-only',valid=False,envelope=invalid))
 ice=next(copy.deepcopy(v['envelope']) for v in vectors if v['id']=='schema-connection.update')
 ice['payload']={'action':'ice-restart','details':{'expected_ice_generation':0,'setup':{'kind':'websocket+webrtc','sdp':'v=0\r\na=ice-ufrag:synthetic\r\n'}}}
 vectors.append(dict(id='ice-restart-original-shape',scope='schema-only',valid=True,envelope=copy.deepcopy(ice)))
 largest=copy.deepcopy(ice);largest['payload']['details']['expected_ice_generation']=SAFE_INTEGER-1
 vectors.append(dict(id='ice-restart-largest-successor',scope='schema-only',valid=True,envelope=largest))
 for label in ['missing-details','missing-generation','negative','unsafe','fractional','kind','streams','codecs','quality','endpoint','ready']:
  invalid=copy.deepcopy(ice);p=invalid['payload']
  if label=='missing-details':p.pop('details')
  elif label=='missing-generation':p['details'].pop('expected_ice_generation')
  elif label=='negative':p['details']['expected_ice_generation']=-1
  elif label=='unsafe':p['details']['expected_ice_generation']=SAFE_INTEGER
  elif label=='fractional':p['details']['expected_ice_generation']=0.5
  elif label=='kind':p['details']['setup']['kind']='native-prepared/1'
  elif label=='streams':p['streams']=['stream-example']
  elif label=='codecs':p['codec_preferences']=['opus']
  elif label=='quality':p['quality_interval_ms']=1000
  elif label=='endpoint':p['details']['endpoint_id']='untrusted-endpoint'
  else:p['details']['ready']=True
  vectors.append(dict(id='ice-restart-'+label,scope='schema-only',valid=False,envelope=invalid))
 directory=ROOT/'contracts/uctp-vectors';directory.mkdir(exist_ok=True)
 emit(directory/'gate6-envelope.json', json.dumps(dict(format=1,scope='Schema vectors; no runtime/authorization/media claim.',vectors=vectors),indent=2)+'\n')
 emit(directory/'operation-transitions.json', json.dumps(dict(format=1,states=STATES,edges=EDGES,terminal=['completed','cancelled','failed'],conditional_edges={'interrupted->preparing':'Only after attributed reconciliation proves re-dispatch safe.','unknown->completed':'Requires attributed authorized reconciliation evidence.','unknown->cancelled':'Requires attributed authorized reconciliation evidence.','unknown->failed':'Requires attributed authorized reconciliation evidence.'}),indent=2)+'\n')
 print(json.dumps(dict(envelope_types=len(kinds),schema_vectors=len(vectors),operation_states=len(STATES))))

if __name__=='__main__':main()
