#!/usr/bin/env python3
"""Update one stable implementation item and validate the full dependency graph."""
import argparse, datetime, html, json, pathlib, re
from zoneinfo import ZoneInfo
ROOT=pathlib.Path(__file__).resolve().parents[1]
STATES={'not-started','in-progress','blocked','done','deferred'}
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--gate',choices=['1','2','3','4'],default='1')
parser.add_argument('--item',required=True)
parser.add_argument('--status',choices=sorted(STATES),required=True)
parser.add_argument('--owner',default='')
parser.add_argument('--evidence',default='')
parser.add_argument('--notes',default='')
args=parser.parse_args()
PLAN=ROOT/f'docs/connect-gate-{args.gate}-implementation.html'
source=PLAN.read_text()
match=re.search(r'(<script id="plan-data" type="application/json">)(.*?)(</script>)',source,re.S)
data=json.loads(match[2]); by_id={t['id']:t for t in data['tasks']}
if args.item not in by_id: parser.error('Unknown item ID')
task=by_id[args.item];task.update(status=args.status,owner=args.owner,evidence=args.evidence,notes=args.notes,updated=datetime.datetime.now(ZoneInfo('America/Los_Angeles')).date().isoformat())
for item in data['tasks']:
 if item['status'] not in STATES: parser.error('Invalid status')
 if any(dep not in by_id for dep in item['depends']): parser.error('Unknown dependency')
 if args.gate == '4' and item['status']=='done' and not item['owner'].strip():
  parser.error('Done requires an owner: '+item['id'])
 if item['status']=='done' and (not item['evidence'].strip() or any(by_id[dep]['status']!='done' for dep in item['depends'])):
  parser.error('Done requires evidence and completed dependencies: '+item['id'])
visiting=set();visited=set()
def visit(item):
 if item in visiting: parser.error('Cyclic dependency')
 if item in visited:return
 visiting.add(item)
 for dep in by_id[item]['depends']:visit(dep)
 visiting.remove(item);visited.add(item)
for item in by_id:visit(item)
data['updated']=task['updated']
if args.gate == '3':
 data['revision']=5
source=source[:match.start(2)]+json.dumps(data,indent=2).replace('<','\\u003c')+source[match.end(2):]
if args.gate == '3':
 source=source.replace(
  'Gate 3 · Implementation revision 01<br>October 8, 2026 · Planned / not started',
  'Gate 3 · Implementation revision 05<br>October 8, 2026 · Implementation and qualification',
 ).replace(
  'Gate 3 · Implementation revision 05<br>October 9, 2026 · Implementation and qualification',
  'Gate 3 · Implementation revision 05<br>October 8, 2026 · Implementation and qualification',
 ).replace(
  'Named future code, test scripts and evidence paths are planned outputs, not claims that they exist.',
  'Status and evidence distinguish implemented local work from remaining live qualification.',
 ).replace(
  'Connect · Gate 3 implementation · Planned, not implemented or qualified.',
  'Connect · Gate 3 implementation · Partial qualification; live acceptance remains open.',
 ).replace(
  'The existing scripts/update-plan.py is Gate 1-specific; do not use it for G3 IDs. Use the editor above or edit embedded plan-data and its static task card together.',
  'Use scripts/update-plan.py --gate 3 with a stable G3 item ID, or the editor above. Update the embedded plan data and static task card together.',
 )
# Keep the saved baseline readable without JavaScript as well.
pattern=r'(<details class="task" id="'+re.escape(args.item)+r'".*?</details>)'
card=re.search(pattern,source,re.S)[1]
labels={'not-started':'Not started','in-progress':'In progress','blocked':'Blocked','done':'Done','deferred':'Deferred'}
card=re.sub(r'data-status="[^"]+"','data-status="'+args.status+'"',card)
card=re.sub(r'(<span class="badge">).*?(</span>)',lambda m:m[1]+labels[args.status]+m[2],card)
card=re.sub(r'(<select data-field="status">)(.*?)(</select>)',lambda m:m[1]+re.sub(r' selected(?:="")?','',m[2]).replace('value="'+args.status+'"','value="'+args.status+'" selected')+m[3],card,flags=re.S)
card=re.sub(r'(<input data-field="owner"[^>]*)(>)',lambda m:re.sub(r' value="[^"]*"','',m[1])+' value="'+html.escape(args.owner,quote=True)+'"'+m[2],card)
for field,value in [('evidence',args.evidence),('notes',args.notes)]:
 card=re.sub(r'(<textarea data-field="'+field+r'"[^>]*>).*?(</textarea>)',lambda m:m[1]+html.escape(value)+m[2],card,flags=re.S)
card=re.sub(r'(<input type="checkbox" data-action="done"[^>]*)(>)',lambda m:re.sub(r' checked(?:="")?','',m[1])+(' checked' if args.status=='done' else '')+m[2],card)
card=re.sub(r'(Last updated: ).*?(</p>)',lambda m:m[1]+task['updated']+m[2],card)
source=re.sub(pattern,lambda _:card,source,count=1,flags=re.S)
for phase in ('M0','M1','M2','M3','M4','M5'):
 rows=[t for t in data['tasks'] if t['phase']==phase]
 label=f"{sum(t['status']=='done' for t in rows)} / {len(rows)} done"
 source=re.sub(r'(<span class="phase-count" data-phase="'+phase+r'">).*?(</span>)',lambda m:m[1]+label+m[2],source)
counts=f"{sum(t['status']=='done' for t in data['tasks'])} / {len(data['tasks'])} complete · {sum(t['status']=='in-progress' for t in data['tasks'])} in progress · {sum(t['status']=='blocked' for t in data['tasks'])} blocked"
source=re.sub(r'(<div class="summary-line" id="counts" aria-live="polite">).*?(</div>)',lambda m:m[1]+counts+m[2],source)
PLAN.write_text(source)
print(args.item,args.status)
