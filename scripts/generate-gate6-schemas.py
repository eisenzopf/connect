#!/usr/bin/env python3
"""Generate the frozen Gate 6 schema/vector kit from audited payloads and v2 rules."""
import copy
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
SAFE_INTEGER = 9_007_199_254_740_991
ID = {'type':'string','minLength':1,'maxLength':128,'pattern':r'^[A-Za-z0-9._~-]+$'}
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
 definitions['AuthHello']['properties']['application_profiles']=array({'type':'string','minLength':1},1,16)
 definitions['AuthHello']['required'].append('application_profiles')
 definitions['AuthSession']['properties']['application_profile']={'const':'connect/2'}
 definitions['AuthSession']['required'].append('application_profile')
 definitions['MessageSend']['properties']['to']={'oneOf':[{'const':'all'},array(ref('Identifier'),1)]}
 definitions['MessageSend']['properties']['content_type']={'type':'string','minLength':1,'maxLength':255}
 definitions['MessageSend']['properties']['body']={'type':'string','maxLength':87384,'description':'Runtime enforces 64 KiB decoded UTF-8/binary bytes, independently of JSON character length.'}
 definitions['SessionInvite']['properties']['to']=array(ref('Identifier'),1)
 definitions['StreamInfo']['properties']['stream_local_id']={'type':'integer','minimum':1,'maximum':65535}
 definitions['StreamSubscription']['oneOf']=[{'required':['strm_id'],'not':{'anyOf':[{'required':['from_participant']},{'required':['kinds']}]}},{'required':['from_participant'],'not':{'required':['strm_id']}}]
 definitions['ReadyStreamBinding']=obj({k:v for k,v in definitions['StreamInfo']['properties'].items() if k!='opened_at'})
 definitions['ConnectionReady']=obj({'streams':array(ref('ReadyStreamBinding'),1)})
 definitions['SessionUpdate']['properties']['kind']={'enum':['participant-leave','role-change','medium-upgrade','codec-renegotiate']}
 definitions['SessionUpdate']['properties']['details']={'type':'object'}
 definitions['SessionUpdate']['required'].append('details')
 definitions['SessionUpdate']['allOf']=[]
 for kind,fields in [('participant-leave',{'participant_id':ref('Identifier')}),('role-change',{'participant_id':ref('Identifier'),'role':{'type':'string','minLength':1,'maxLength':64}}),('medium-upgrade',{'media':array({'enum':['audio','video','screen','data']},1)}),('codec-renegotiate',{'codec_preferences':array({'type':'string','minLength':1},1)})]:
  definitions['SessionUpdate']['allOf'].append({'if':{'properties':{'kind':{'const':kind}}},'then':{'properties':{'details':obj(fields)}}})
 definitions['ConnectionUpdate']['properties']['action']={'enum':['hold','resume','mute','unmute','codec-renegotiate']}
 for name in ['StreamInfo','ReadyStreamBinding','StreamOffer','StreamAnswer']:
  definitions[name]['properties']['kind']={'enum':['audio','video','screen','data']}
  definitions[name]['properties']['direction']={'enum':['sendrecv','sendonly','recvonly','inactive']}
 definitions['OperationState']={'enum':STATES}
 definitions['OperationUpdated']=obj({'operation_id':ref('Identifier'),'state':ref('OperationState'),'revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'result':{},'problem':{'type':'object'}},['operation_id','state','revision'])
 definitions['OperationUpdated']['allOf']=[{'if':{'properties':{'state':{'not':{'const':'completed'}}}},'then':{'not':{'required':['result']}}}]
 definitions['MemberAdd']=obj({'identity_id':ref('Identifier'),'kind':{'enum':['human','ai','system','external']},'role':{'type':'string','minLength':1,'maxLength':64}})
 definitions['MemberUpdate']=obj({'participant_id':ref('Identifier'),'role':{'type':'string','minLength':1,'maxLength':64}})
 definitions['MemberRemove']=obj({'participant_id':ref('Identifier'),'reason':{'type':'string','maxLength':512}},['participant_id'])
 definitions['MemberChanged']=obj({'participant_id':ref('Identifier'),'kind':{'enum':['added','updated','removed']},'membership_epoch':{'type':'integer','minimum':1,'maximum':SAFE_INTEGER}})
 definitions['SessionConsult']=obj({'retained_connid':ref('Identifier'),'private_participants':array(ref('Identifier'),2),'hold_treatment':{'enum':['silence','configured-media']}},['retained_connid','private_participants'])
 definitions['SessionHandoff']=obj({'source_participant':ref('Identifier'),'target_participant':ref('Identifier'),'retained_connid':ref('Identifier'),'candidate_connid':ref('Identifier'),'outgoing':{'enum':['leave','observe']},'role_scope':{'enum':['conversation','session']},'expected_conversation_revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER}},['source_participant','target_participant','retained_connid','outgoing'])
 definitions['SessionHandoff']['allOf']=[{'if':{'required':['role_scope'],'properties':{'role_scope':{'const':'session'}}},'then':{},'else':{'required':['expected_conversation_revision']}}]
 definitions['ConnectionReplace']=obj({'endpoint_id':ref('Identifier'),'candidate_connid':ref('Identifier'),'retained_connid':ref('Identifier')},['retained_connid'])
 definitions['ConnectionReplace']['oneOf']=[{'required':['endpoint_id'],'not':{'required':['candidate_connid']}},{'required':['candidate_connid'],'not':{'required':['endpoint_id']}}]
 definitions['OperationGet']=obj({'operation_id':ref('Identifier')})
 definitions['OperationCancel']=obj({'operation_id':ref('Identifier'),'reason':{'type':'string','maxLength':512}},['operation_id'])
 definitions['OperationReconcile']=obj({'operation_id':ref('Identifier'),'outcome':{'enum':['completed','cancelled','failed']},'evidence_id':ref('Identifier')})
 definitions['ConversationGet']=obj({})
 definitions['ConversationSubscribe']=obj({'after':ref('Identifier'),'live':{'type':'boolean'},'limit':{'type':'integer','minimum':1,'maximum':500},'lease_seconds':{'type':'integer','minimum':1,'maximum':30}},[])
 definitions['ConversationSnapshot']=obj({'revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'cursor':ref('Identifier'),'items':array({'type':'object'},0,500),'has_more':{'type':'boolean'}})
 definitions['ConversationEvent']=obj({'event_id':ref('Identifier'),'cursor':ref('Identifier'),'kind':{'type':'string','minLength':1},'revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'data':{'type':'object'}})
 definitions['SessionObservation']=obj({'kind':{'enum':['transcript','speech','action-request']},'participant_id':ref('Identifier'),'source':{'type':'string','minLength':1},'correlation_id':ref('Identifier'),'data':{'type':'object'}})
 kinds={e['type']:e['payload_struct'] or 'ConnectionReady' for e in ledger['envelopes']}
 kinds.update({'conversation.get':'ConversationGet','conversation.participant.add':'MemberAdd','conversation.participant.update':'MemberUpdate','conversation.participant.remove':'MemberRemove','conversation.participant.changed':'MemberChanged','conversation.subscribe':'ConversationSubscribe','conversation.snapshot':'ConversationSnapshot','conversation.event':'ConversationEvent','session.consult':'SessionConsult','session.handoff':'SessionHandoff','connection.replace':'ConnectionReplace','operation.get':'OperationGet','operation.cancel':'OperationCancel','operation.reconcile':'OperationReconcile','operation.updated':'OperationUpdated','session.observation':'SessionObservation'})
 mutations={'conversation.create','conversation.close','conversation.participant.add','conversation.participant.update','conversation.participant.remove','session.invite','session.accept','session.reject','session.cancel','session.end','session.update','session.consult','session.handoff','connection.offer','connection.answer','connection.update','connection.end','connection.replace','stream.subscribe','stream.unsubscribe','message.send','message.read','dtmf.send','operation.cancel','operation.reconcile'}
 revisioned={'conversation.participant.add','conversation.participant.update','conversation.participant.remove','session.update','session.consult','session.handoff','connection.replace','connection.update','stream.subscribe','stream.unsubscribe'}
 definitions['Signature']=obj({'keyid':{'type':'string','minLength':1},'alg':{'enum':['EdDSA','ES256','PS256','RS256']},'sig':{'type':'string','minLength':1}})
 fields={'v':{'const':2},'type':{'enum':list(kinds)},'id':ref('Identifier'),'ts':{'type':'string','format':'date-time'},'payload':{'type':'object'},'cid':ref('Identifier'),'sid':ref('Identifier'),'connid':ref('Identifier'),'in_reply_to':ref('Identifier'),'operation_id':ref('Identifier'),'expected_revision':{'type':'integer','minimum':0,'maximum':SAFE_INTEGER},'signature':ref('Signature')}
 schema={'$schema':'https://json-schema.org/draft/2020-12/schema','$id':'https://rudeless.ai/connect/contracts/uctp-envelope-2.schema.json','title':'Gate 6 UCTP envelope / connect/2 target','description':'Schema conformance does not establish authorization, active media, provider effects or runtime qualification.',**obj(fields,['v','type','id','ts','payload']),'$defs':definitions,'allOf':[]}
 for name,kind in kinds.items():
  required=['operation_id'] if name in mutations else []
  if name in revisioned:required.append('expected_revision')
  if name.startswith(('session.','connection.','stream.','dtmf.')):required.extend(['cid','sid'])
  elif name.startswith(('conversation.','message.')) and name not in ('conversation.create','conversation.list'):required.append('cid')
  if name.startswith(('connection.','stream.','dtmf.')):required.append('connid')
  schema['allOf'].append({'if':{'properties':{'type':{'const':name}}},'then':{'properties':{'payload':ref(kind)},'required':required}})
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
 schema,kinds,mutations,revisioned=build()
 directory=ROOT/'contracts/schemas';directory.mkdir(exist_ok=True)
 (directory/'uctp-envelope-2.schema.json').write_text(json.dumps(schema,indent=2)+'\n')
 vectors=[]
 for i,(name,kind) in enumerate(kinds.items()):
  value={'v':2,'type':name,'id':f'env_vector_{i}','ts':'2026-10-15T12:00:00Z','cid':'conv_example','sid':'sess_example','connid':'conn_example','payload':sample(schema['$defs'][kind],schema['$defs'])}
  if name in mutations:value['operation_id']=f'op_vector_{i}'
  if name in revisioned:value['expected_revision']=1
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
 directory=ROOT/'contracts/uctp-vectors';directory.mkdir(exist_ok=True)
 (directory/'gate6-envelope.json').write_text(json.dumps(dict(format=1,scope='Schema vectors; no runtime/authorization/media claim.',vectors=vectors),indent=2)+'\n')
 (directory/'operation-transitions.json').write_text(json.dumps(dict(format=1,states=STATES,edges=EDGES,terminal=['completed','cancelled','failed'],conditional_edges={'interrupted->preparing':'Only after attributed reconciliation proves re-dispatch safe.','unknown->completed':'Requires attributed authorized reconciliation evidence.','unknown->cancelled':'Requires attributed authorized reconciliation evidence.','unknown->failed':'Requires attributed authorized reconciliation evidence.'}),indent=2)+'\n')
 print(json.dumps(dict(envelope_types=len(kinds),schema_vectors=len(vectors),operation_states=len(STATES))))

if __name__=='__main__':main()
