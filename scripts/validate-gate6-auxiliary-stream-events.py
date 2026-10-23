#!/usr/bin/env python3
"""Validate exact local audio lifecycle clause observations from production tests."""
import json,pathlib,datetime,argparse
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('observations',type=pathlib.Path)
parser.add_argument('--output',type=pathlib.Path)
parser.add_argument('--require-paired-replacement',action='store_true')
args=parser.parse_args()
rows=[json.loads(line) for line in args.observations.read_text().splitlines()]
groups={};opened=[];closed=[]
def time(s): return datetime.datetime.fromisoformat(s.replace('Z','+00:00'))
for row in rows:
 e=row['frame'];p=e['payload'];key=(row['algorithm'],e['cid'],e['connid']);events=groups.setdefault(key,[])
 assert e.get('sid')=='native-session' and 'signature' in e
 if e['type']=='connection.ready': events.append('ready')
 elif e['type']=='stream.opened':
  v=p['stream'];assert 'ready' in events
  assert v['kind']=='audio' and v['direction']=='sendrecv'
  assert isinstance(v['stream_local_id'],int) and 0<=v['stream_local_id']<=65535
  assert isinstance(v['codec']['params'],dict)
  assert v['codec']['params']['sample_rate']==8000 and v['codec']['params']['channels']==1
  assert time(v['opened_at'])<=time(e['ts'])
  assert isinstance(v['strm_id'],str) and v['strm_id']
  events.append('opened');opened.append(row)
 elif e['type']=='stream.closed':
  assert isinstance(p['strm_id'],str) and p['strm_id']
  assert p['reason']=='codec-renegotiated'
  assert isinstance(p['reason_code'],int) and 0<=p['reason_code']<=65535
  assert time(p['closed_at'])<=time(e['ts'])
  events.append('closed');closed.append(row)
assert {r['algorithm'] for r in opened}=={'EdDSA','ES256'}
assert closed and {r['algorithm'] for r in closed} <= {'EdDSA','ES256'}
if args.require_paired_replacement: assert {r['algorithm'] for r in closed} == {'EdDSA','ES256'}
delays=[]; publication_delays=[]; paired=0
for index,row in enumerate(rows):
 e=row['frame']
 if e['type']!='stream.closed': continue
 for following in rows[index+1:]:
  other=following['frame']
  if following['algorithm']==row['algorithm'] and other.get('cid')==e['cid'] and other.get('connid')==e['connid'] and other['type']=='stream.opened' and other['payload']['stream']['strm_id']==e['payload']['strm_id']:
   delays.append((time(other['payload']['stream']['opened_at'])-time(e['payload']['closed_at'])).total_seconds()*1000)
   publication_delays.append((time(other['ts'])-time(e['ts'])).total_seconds()*1000)
   if args.require_paired_replacement:
    assert rows[index+1] is following, 'replacement lifecycle frames interleaved'
    assert e['ts']==other['ts'] and e['id']!=other['id']
    assert other['payload']['stream']['codec']['name']=='pcma'
    assert time(e['payload']['closed_at'])<time(e['ts']), 'deferred close lost original observed time'
    paired+=1
   break
 else: raise AssertionError('closed codec has no observed replacement')
result={'paired_replacement_publications':paired,'close_to_open_publication_ms':publication_delays,'codec_close_to_replacement_open_ms':delays,'passed':True,'stream_opened':len(opened),'stream_closed':len(closed),'connection_groups':len(groups),'opened_algorithms':['EdDSA','ES256'],'closed_algorithms':sorted({r['algorithm'] for r in closed}),'scope':'Production native audio only; actual duplex before ready/open, exact opened and codec-close shapes. Paired replacement checks require adjacent wire observations and a common publication timestamp while preserving the earlier close time.' if args.require_paired_replacement else 'Production native audio only; actual duplex before ready/open, exact opened and codec-close shapes; immediate replacement is not qualified by this mode.'}
if args.output: args.output.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result))
