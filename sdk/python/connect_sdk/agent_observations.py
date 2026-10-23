"""Provider metadata remains data; reported tool results are not Core effects."""
import json,unicodedata
MAX=9007199254740991
def num(v):return type(v) is int and 0<=v<=MAX
def closed(v,keys):return isinstance(v,dict) and set(v)==set(keys.split())
def reference(v):return isinstance(v,str) and bool(v.strip()) and len(v.encode('utf-8'))<=512 and not any(unicodedata.category(c)=='Cc' for c in v)
def nil(v,f):return v is None or f(v)
def gap(v):return closed(v,'first_sequence last_sequence observations') and num(v['first_sequence']) and v['first_sequence']>0 and num(v['last_sequence']) and v['first_sequence']<=v['last_sequence'] and num(v['observations']) and v['observations']==v['last_sequence']-v['first_sequence']+1
def valid_agent(v):
    if not closed(v,'local_sequence capture_complete missing_before provider_session_id provider_event_id captured_at content') or not num(v['local_sequence']) or v['local_sequence']==0 or v['capture_complete'] is not False or not num(v['captured_at']) or not nil(v['provider_session_id'],reference) or not nil(v['provider_event_id'],reference) or not (v['missing_before'] is None or (gap(v['missing_before']) and v['missing_before']['last_sequence']<v['local_sequence'])) or len(json.dumps(v,ensure_ascii=False,separators=(',',':'),allow_nan=False).encode())>65536:return False
    d=v['content']
    if not isinstance(d,dict):return False
    k=d.get('kind');role=lambda r:r in (None,'user','assistant','system','other');complete=lambda r:r in ('partial','final','unavailable')
    if k=='transcript':return closed(d,'kind text speaker_role completeness') and (d['text'] is None or isinstance(d['text'],str)) and role(d['speaker_role']) and complete(d['completeness'])
    if k=='speech':return closed(d,'kind state speaker_role turn completeness') and nil(d['state'],reference) and role(d['speaker_role']) and nil(d['turn'],num) and complete(d['completeness'])
    if k=='action':return closed(d,'kind name call_id phase arguments result completeness') and nil(d['name'],reference) and nil(d['call_id'],reference) and d['phase'] in ('requested','result-observed','unknown') and complete(d['completeness'])
    if k=='provider-status':return closed(d,'kind state') and reference(d['state'])
    if k=='unavailable':return closed(d,'kind issue') and d['issue'] in ('missing-fields','malformed-event','mismatched-session','unsupported-event','sequence-exhausted')
    if k=='gap':return closed(d,'kind first_sequence last_sequence observations') and gap({x:d[x] for x in ('first_sequence','last_sequence','observations')}) and d['last_sequence']==v['local_sequence']
    return False
def agent_supported(v,cap):
    if not valid_agent(v):return False
    c=v['content'];k=c['kind']
    return cap['transcript'] if k=='transcript' else cap['speech'] if k=='speech' else bool(cap['actions']) and (c['name'] is None or c['name'] in cap['actions']) if k=='action' else True
