"""Closed snapshot/event shapes; proposals and source labels are never authority."""
import json,re
from ._validation import integer
from .agent_observations import valid_agent,agent_supported

def text(v,maximum=512):return isinstance(v,str) and 0<len(v)<=maximum and not re.search(r'[\x00-\x1f\x7f]',v)
def token(v):return isinstance(v,str) and len(v)<=2048 and re.fullmatch(r'ev1_[A-Za-z0-9_-]+',v) is not None
def num(v):return integer(v) and v<=9007199254740991
def epoch(v):return num(v) and v>0
def nil(v,test):return v is None or test(v)
def closed(v,keys):return isinstance(v,dict) and set(v)==set(keys.split())
def array(v,maximum,test):return isinstance(v,list) and len(v)<=maximum and all(test(x) for x in v)
def source(v):return closed(v,'kind correlation_id') and text(v['kind'],128) and text(v['correlation_id'])
def audience(v):return closed(v,'participant_id membership_epoch') and text(v['participant_id']) and epoch(v['membership_epoch'])
STATES=('accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown')
BASE=dict(session='id conversation_id initiator_id participant_id state media created_at updated_at ended_reason',artifact='id conversation_id owner_id name media_type size received sha256 state created_at expires_at',call='id conversation_id route_id destination state provider_id ended_reason direction end_requested',message='id conversation_id sender body sender_participant_id audience content_type body_encoding label attachments in_reply_to_msg accepted_at')
def base(kind,v,validate,depth):
    if not closed(v,BASE[kind]):return False
    if kind=='message':return validate('history',dict(messages=[v],next=None),depth+1) and array(v['audience'],100,audience) and array(v['attachments'],100,lambda a:closed(a,'id content_type size_bytes') and text(a['id']) and text(a['content_type'],128) and num(a['size_bytes']))
    return validate(kind,v,depth+1)
def operation(v,validate,depth):
    return closed(v,'operation_id record_id command_type conversation_id session_id connection_id expected_revision state revision expires_at created_at updated_at cancel_requested result effects causal_event_ids events_omitted') and validate('durable_operation',v,depth+1) and array(v['effects'],64,lambda e:closed(e,'id provider correlation_key idempotency state attempt compensates_effect_id submission_generation')) and array(v['causal_event_ids'],100,text)
def provider_call(v):
    return closed(v,'id operation_id conversation_id session_id participant_id membership_epoch endpoint_id state direction remote_identity_verified revision observed_at closed_at ended_reason problem local_media') and all(text(v[k]) for k in ('id','operation_id','conversation_id','session_id','participant_id','endpoint_id')) and epoch(v['membership_epoch']) and epoch(v['revision']) and v['state'] in ('starting','active','ended','failed','unknown') and nil(v['observed_at'],num) and nil(v['closed_at'],num) and nil(v['ended_reason'],lambda x:isinstance(x,str) and len(x)<=256 and not re.search(r'[\x00-\x1f\x7f]',x)) and nil(v['problem'],lambda x:text(x,128)) and v['direction'] in ('inbound','outbound') and v['remote_identity_verified'] is False and v['local_media'] is False
def session_snapshot(v):
    return closed(v,'id conversation_id state media intent capabilities_offer codec_preferences is_main revision topology_revision created_at updated_at ended_reason') and all(text(v[k]) for k in ('id','conversation_id','state')) and array(v['media'],16,text) and nil(v['intent'],lambda x:text(x,128)) and nil(v['capabilities_offer'],lambda x:isinstance(x,dict)) and array(v['codec_preferences'],16,text) and type(v['is_main']) is bool and all(num(v[k]) for k in ('revision','topology_revision','created_at','updated_at')) and nil(v['ended_reason'],closure_reason)
def item(v,validate,depth):
    if not closed(v,'kind data') or not isinstance(v['data'],dict):return False
    k,d=v['kind'],v['data']
    if k=='provider_call':return provider_call(d)
    if k=='conversation':return closed(d,'conversation_id revision closed policy') and text(d['conversation_id']) and num(d['revision']) and type(d['closed']) is bool and d['policy'] in ('persistent','ephemeral')
    if k=='participant':return closed(d,'participant_id conversation_id identity_id kind role membership_epoch active removal_reason') and all(text(d[x]) for x in ('participant_id','conversation_id','identity_id')) and d['kind'] in ('human','ai','system','external') and text(d['role'],64) and epoch(d['membership_epoch']) and type(d['active']) is bool and nil(d['removal_reason'],lambda x:isinstance(x,str) and len(x)<=512)
    if k=='session':return session_snapshot(d)
    if k=='session_participant':return closed(d,'session_id participant_id membership_epoch role state') and text(d['session_id']) and text(d['participant_id']) and epoch(d['membership_epoch']) and text(d['role'],64) and d['state'] in ('invited','joining','present','left','removed')
    if k=='connection':return closed(d,'connection_id session_id participant_id membership_epoch endpoint_id device_id generation revision state replaces') and all(text(d[x]) for x in ('connection_id','session_id','participant_id','endpoint_id','device_id')) and epoch(d['membership_epoch']) and epoch(d['generation']) and num(d['revision']) and d['state'] in ('preparing','ready','active','held','retiring','retired','interrupted','unknown','failed') and nil(d['replaces'],text)
    if k=='stream':return closed(d,'stream_id session_id participant_id connection_id medium state revision') and all(text(d[x]) for x in ('stream_id','session_id','participant_id','medium','state')) and nil(d['connection_id'],text) and num(d['revision'])
    if k=='operation':return operation(d,validate,depth+1)
    return False
def snapshot(v,validate,depth):
    return closed(v,'conversation_id snapshot_id revision cursor expires_at items next has_more') and text(v['conversation_id']) and text(v['snapshot_id']) and num(v['revision']) and num(v['expires_at']) and token(v['cursor']) and nil(v['next'],token) and type(v['has_more']) is bool and v['has_more']==(v['next'] is not None) and array(v['items'],500,lambda x:item(x,validate,depth+1) and ('conversation_id' not in x['data'] or x['data']['conversation_id']==v['conversation_id']))
def receipt(d):
    return closed(d,'conversation_id message_id participant_id membership_epoch channel delivery_id state provenance observed_at') and all(text(d[x]) for x in ('conversation_id','message_id','participant_id','delivery_id')) and epoch(d['membership_epoch']) and num(d['observed_at']) and d['channel'] in ('native','provider') and d['state'] in ('queued','accepted','delivered','read','failed','unknown') and d['provenance'] in ('journal','client_reported','provider_observation','provider_state','local_attempt') and (d['state']!='read' or (d['channel']=='native' and d['provenance']=='client_reported'))
def closure_reason(v):
    if not isinstance(v,str) or re.search(r'[\x00-\x1f\x7f-\x9f]',v):return False
    try:return len(v.encode('utf-8'))<=512
    except UnicodeEncodeError:return False
def journal_data(v,validate,depth):
    if not closed(v,'kind data') or not isinstance(v['data'],dict):return False
    k,d=v['kind'],v['data']
    if k=='provider_call':return provider_call(d)
    if k=='connection_prepared':return closed(d,'connection_id conversation_id session_id participant_id membership_epoch endpoint_id generation revision state') and all(text(d[x]) for x in ('connection_id','conversation_id','session_id','participant_id','endpoint_id')) and epoch(d['membership_epoch']) and type(d['generation']) is int and d['generation']==1 and type(d['revision']) is int and d['revision']==0 and d['state']=='preparing'
    if k in ('connection_ready','connection_retired'):return item(dict(kind='connection',data=d),validate,depth+1) and d['state']==('ready' if k=='connection_ready' else 'retired') and d['generation']>=(1 if k=='connection_ready' else 2) and d['revision']>0
    if k in ('message','session','artifact','call'):return base(k,d,validate,depth+1)
    if k=='native_data_write':return closed(d,'message_id source_connection_id target_connection_id participant_id membership_epoch generation disposition provenance observed_at') and all(text(d[x]) for x in ('message_id','source_connection_id','target_connection_id','participant_id')) and epoch(d['membership_epoch']) and epoch(d['generation']) and num(d['observed_at']) and d['disposition'] in ('not_started','local_written','submission_unknown') and d['provenance'] in ('local_writer','runtime_recovery') and (d['provenance']!='runtime_recovery' or d['disposition']=='submission_unknown')
    if k=='receipt':return receipt(d)
    if k=='membership':return closed(d,'participant_id identity_id role active membership_epoch removal_reason') and nil(d['participant_id'],text) and nil(d['identity_id'],text) and nil(d['role'],lambda x:text(x,64)) and nil(d['active'],lambda x:type(x) is bool) and nil(d['membership_epoch'],epoch) and nil(d['removal_reason'],lambda x:isinstance(x,str) and len(x)<=512)
    if k=='operation':return closed(d,'operation_id record_id state') and text(d['operation_id']) and text(d['record_id']) and d['state'] in STATES
    if k=='session_participant_joined':return closed(d,'fact_id session membership_epoch participant via_connection') and text(d['fact_id']) and session_snapshot(d['session']) and d['session']['state']=='active' and epoch(d['membership_epoch']) and text(d['via_connection']) and closed(d['participant'],'participant_id identity_id kind role display_name joined_at') and text(d['participant']['participant_id']) and text(d['participant']['identity_id']) and d['participant']['kind'] in ('human','ai','system','external') and text(d['participant']['role'],64) and text(d['participant']['display_name'],256) and epoch(d['participant']['joined_at'])
    if k=='session_participant_left':return closed(d,'fact_id session membership_epoch participant_id left_at reason') and text(d['fact_id']) and session_snapshot(d['session']) and epoch(d['membership_epoch']) and text(d['participant_id']) and epoch(d['left_at']) and d['reason'] in ('explicit-leave','all-connections-ended','kicked','transferred-out')
    if k=='session_started':return closed(d,'session started_at participants_present active_connections negotiated_capabilities') and session_snapshot(d['session']) and d['session']['state']=='active' and epoch(d['started_at']) and array(d['participants_present'],256,text) and bool(d['participants_present']) and len(set(d['participants_present']))==len(d['participants_present']) and array(d['active_connections'],256,lambda x:closed(x,'connid participant_id transport') and text(x['connid']) and x['participant_id'] in d['participants_present'] and x['transport'] in ('quic','webtransport','websocket+webrtc','sip-interop','webrtc-interop')) and bool(d['active_connections']) and len({x['connid'] for x in d['active_connections']})==len(d['active_connections']) and isinstance(d['negotiated_capabilities'],dict)
    if k=='session_control':return session_snapshot(d)
    if k=='session_ended':return closed(d,'session by_participant by_membership_epoch reason_code reason ended_at') and session_snapshot(d['session']) and d['session']['state']=='ended' and d['reason']==(d['session']['ended_reason'] if d['session']['ended_reason'] is not None else 'unspecified') and nil(d['by_participant'],text) and nil(d['by_membership_epoch'],epoch) and (d['by_participant'] is None)==(d['by_membership_epoch'] is None) and num(d['reason_code']) and d['reason_code']<=65535 and closure_reason(d['reason']) and epoch(d['ended_at'])
    if k=='conversation_closed':return closed(d,'conversation_id reason_code reason closed_at') and text(d['conversation_id']) and num(d['reason_code']) and num(d['closed_at']) and d['closed_at']>0 and closure_reason(d['reason'])
    if k=='changed':return closed(d,'resource_kind resource_id') and nil(d['resource_kind'],text) and nil(d['resource_id'],text)
    return False
def bound_connection_fact(v):
    return v['data']['kind'] not in ('connection_prepared','connection_ready','connection_retired') or all(v['data']['data'][field]==v[field] for field in ('session_id','connection_id','participant_id'))
def bound_provider_call(v):
    return v['data']['kind']!='provider_call' or (v['kind']=='provider_voice' and v['connection_id'] is None and all(v['data']['data'][field]==v[field] for field in ('session_id','participant_id','operation_id','revision')))
def bound_session_fact(v,cid):
    k,d=v['data']['kind'],v['data']['data']
    if k in ('session_participant_joined','session_participant_left'):return v['kind']==k and v['connection_id'] is None and d['session']['id']==v['session_id'] and d['session']['conversation_id']==cid and d['session']['revision']==v['revision'] and ((d['participant']['participant_id']==v['participant_id'] and d['participant']['joined_at']<=v['observed_at']) if k=='session_participant_joined' else (d['participant_id']==v['participant_id'] and d['left_at']<=v['observed_at']))
    if k=='session_started':return v['kind']=='session_started' and v['connection_id'] is None and v['operation_id'] is None and d['session']['id']==v['session_id'] and d['session']['conversation_id']==cid and d['session']['revision']==v['revision'] and d['started_at']<=v['observed_at']
    if k=='session_control':return v['kind']=='session_control' and d['id']==v['session_id'] and d['conversation_id']==cid and d['revision']==v['revision']
    if k=='session_ended':return v['kind'] in ('session_control','session_ended') and v['connection_id'] is None and d['session']['id']==v['session_id'] and d['session']['conversation_id']==cid and d['session']['revision']==v['revision'] and d['ended_at']<=v['observed_at']
    return True
def event(v,cid,validate,depth):
    return closed(v,'event_id cursor conversation_id session_id connection_id participant_id operation_id kind revision source observed_at audience data') and text(v['event_id']) and token(v['cursor']) and v['conversation_id']==cid and all(nil(v[x],text) for x in ('session_id','connection_id','participant_id','operation_id')) and text(v['kind'],128) and num(v['revision']) and source(v['source']) and num(v['observed_at']) and array(v['audience'],100,audience) and bool(v['audience']) and journal_data(v['data'],validate,depth+1) and (v['data']['kind']!='native_data_write' or (v['kind']=='native_data_write' and v['operation_id'] is not None and v['session_id'] is not None and v['data']['data']['target_connection_id']==v['connection_id'] and v['data']['data']['participant_id']==v['participant_id'] and v['data']['data']['observed_at']==v['observed_at'])) and (v['data']['kind']!='conversation_closed' or (v['kind']=='conversation_closed' and v['session_id'] is None and v['connection_id'] is None and v['data']['data']['closed_at']<=v['observed_at'])) and bound_session_fact(v,cid) and bound_connection_fact(v) and bound_provider_call(v) and ('conversation_id' not in v['data']['data'] or v['data']['data']['conversation_id']==cid)
def capabilities(v):return closed(v,'transcript speech actions') and type(v['transcript']) is bool and type(v['speech']) is bool and array(v['actions'],16,lambda x:text(x,128)) and len(set(v['actions']))==len(v['actions'])
def work(v):return closed(v,'task_id effect_id question_id') and all(text(x) for x in v.values())
def route(v):return closed(v,'work correlation_id participant_id connection_id generation source capabilities') and nil(v['work'],work) and all(text(v[x],128) for x in ('correlation_id','participant_id','connection_id','source')) and epoch(v['generation']) and capabilities(v['capabilities'])
def draft(v):
    if not isinstance(v,dict):return False
    if v.get('kind')=='agent':return closed(v,'kind observation') and valid_agent(v['observation'])
    if v.get('kind')=='transcript':return closed(v,'kind text completeness turn_id fragment_id') and isinstance(v['text'],str) and bool(v['text']) and v['completeness'] in ('interim','final','incomplete') and nil(v['turn_id'],lambda x:text(x,128)) and nil(v['fragment_id'],lambda x:text(x,128))
    if v.get('kind')=='speech':return closed(v,'kind state turn_id') and v['state'] in ('started','stopped') and nil(v['turn_id'],lambda x:text(x,128))
    if v.get('kind')=='action-request':return closed(v,'kind action provider_tool_call_id arguments turn_id') and text(v['action'],128) and text(v['provider_tool_call_id'],128) and isinstance(v['arguments'],dict) and nil(v['turn_id'],lambda x:text(x,128))
    return False
def observation(v,page):
    if not closed(v,'work observation_id conversation_id session_id participant_id membership_epoch connection_id generation source observed_at data') or not nil(v['work'],work) or not text(v['observation_id']) or v['conversation_id']!=page['conversation_id'] or v['session_id']!=page['session_id'] or not text(v['participant_id']) or not text(v['connection_id']) or not epoch(v['membership_epoch']) or not epoch(v['generation']) or not source(v['source']) or not num(v['observed_at']) or not draft(v['data']):return False
    if len(json.dumps(v['data'],ensure_ascii=False,separators=(',',':')).encode())>(70*1024 if v['data']['kind']=='agent' else 16*1024):return False
    d=v['data']
    return any(r['work']==v['work'] and r['connection_id']==v['connection_id'] and r['generation']==v['generation'] and r['participant_id']==v['participant_id'] and r['source']==v['source']['kind'] and r['correlation_id']==v['source']['correlation_id'] and (agent_supported(d['observation'],r['capabilities']) if d['kind']=='agent' else r['capabilities']['transcript'] if d['kind']=='transcript' else r['capabilities']['speech'] if d['kind']=='speech' else d['action'] in r['capabilities']['actions']) for r in page['routes'])
def _valid_events(kind,v,validate,depth):
    if depth>8:return False
    if kind=='conversation_snapshot':return snapshot(v,validate,depth+1)
    if kind=='conversation_events':return closed(v,'conversation_id events cursor has_more lease expires_at resync_required snapshot') and text(v['conversation_id']) and token(v['cursor']) and type(v['has_more']) is bool and type(v['resync_required']) is bool and nil(v['lease'],token) and nil(v['expires_at'],num) and ((v['lease'] is None)==(v['expires_at'] is None)) and array(v['events'],256,lambda x:event(x,v['conversation_id'],validate,depth+1)) and (v['snapshot'] is None or (snapshot(v['snapshot'],validate,depth+1) and v['snapshot']['conversation_id']==v['conversation_id'] and v['snapshot']['cursor']==v['cursor'] and not v['events'])) and (not v['resync_required'] or v['snapshot'] is not None)
    if kind=='session_observations':return closed(v,'conversation_id session_id observations expires_at routes cursor has_more resync_required') and text(v['conversation_id']) and text(v['session_id']) and token(v['cursor']) and num(v['expires_at']) and type(v['has_more']) is bool and type(v['resync_required']) is bool and array(v['routes'],128,route) and array(v['observations'],256,lambda x:observation(x,v))
    return False

def valid_events(kind,v,validate,depth):
    try:
        return len(json.dumps(v,ensure_ascii=False,separators=(',',':'),allow_nan=False).encode())<=512*1024 and _valid_events(kind,v,validate,depth)
    except (UnicodeError,ValueError,TypeError,RecursionError):
        return False
