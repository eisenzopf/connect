#!/usr/bin/env python3
"""Build/check the source-bound Gate 6 UCTP requirement inventory.

This audits source coverage, not runtime conformance. Each rule retains the
source excerpt and line range; assigned runtime evidence starts unrun.
"""
import argparse
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUTPUT = ROOT / 'contracts/uctp-gate-6-requirements.json'
SOURCE = ROOT / '.deps/rvoip/docs/CONVERSATION_PROTOCOL.md'
REVISION = 'f4d528eaabbac935bce9a6404d3ce4fdec35ed0f'
TASKS = {
 '1': ('model', [3, 4, 16], [1, 2]),
 '2': ('model', [3, 4, 16], [1, 2]),
 '3': ('uctp-envelope', [7, 10, 16, 37, 94], [2, 10, 16]),
 '4': ('uctp-transports', [11, 24, 48, 49, 51, 94], [2, 13, 14]),
 '5': ('identity-security', [4, 5, 17, 26, 30, 31, 55, 94], [2, 3, 17]),
 '6': ('uctp-registry', [2, 16, 94], [2, 16]),
 '7': ('session-media', [4, 6, 9, 19, 20, 21, 22, 23, 27, 38, 39, 40, 41, 46, 94], [2, 6, 7, 8, 9, 18]),
 '8': ('capabilities', [9, 20, 24, 32, 38, 94], [2, 6, 17]),
 '9': ('messages-audiences', [5, 18, 33, 34, 36, 94], [2, 4, 5]),
 '10': ('streams-transports', [9, 20, 24, 48, 49, 50, 51, 54, 64, 94], [2, 6, 13, 14, 17]),
 '11': ('errors-operations', [7, 16, 35, 37, 94], [2, 10, 17]),
 '12': ('gateway-adapters', [15, 24, 32, 44, 45, 54, 67, 100], [12, 13, 14, 28]),
 '13': ('reserved-federation', [2, 13], [2]),
 '14': ('explicit-design-decisions', [2, 4, 5, 9, 11, 13], [1, 2]),
 '15': ('model-cross-reference', [2, 4, 13], [1, 2]),
 '16': ('versioning', [10, 16, 37, 94], [2, 16]),
}
RESOLUTIONS = [
 ('GAP6-01','Identifier encoding', '3.1 / 5.5.2', 'ULID-only prose conflicts with bounded opaque IDs and UUID generation.', 'Bounded URI-unreserved IDs; UUID defaults; retain local/federated distinction.', 'contracts/connect-uctp.md'),
 ('GAP6-02','Signed retry', '2 / 5.5.1 / 11.3', 'Reusing an envelope for idempotency conflicts with signature replay rejection.', 'Fresh envelope IDs and signatures; stable logical operation ID and digest.', 'contracts/connect-operations.md'),
 ('GAP6-03','Session end and grace', '7.3', 'Immediate last-connection end conflicts with 30-second reconnect grace.', 'Explicit end is terminal; loss starts grace and ends only after expiry with no permitted negotiation.', 'docs/decisions/013-gate-6-model.md'),
 ('GAP6-04','Concurrent topology mutation', '7.2 session.update', 'Latest-wins cannot safely arbitrate competing handoff/device operations.', 'Expected revision, resource fencing, readiness-gated commit and explicit conflict.', 'contracts/connect-transitions.md'),
 ('GAP6-05','Private receipts/history', '9.2 / 9.3', 'Conversation-wide receipt relay and history visibility do not define recipient privacy.', 'Current authority plus send-time audience/membership epoch across all projections.', 'contracts/connect-audiences.md'),
 ('GAP6-06','Private session records', '7.6', 'All session participants cannot receive a full private-consultation record.', 'Per-source audience projections with their own byte digests and authorized handles.', 'contracts/connect-records.md'),
 ('GAP6-07','Compound error codes', '5.5 / 11.2', '401-1 notation is inconsistent with the typed integer error code.', 'Integer code plus stable machine-readable reason and details.', 'contracts/connect-uctp.md'),
 ('GAP6-08','Registry omissions', '6 / 7', 'Overview omits auth.refresh, conversation.close, session.end and connection.ice-candidate.', 'Registry is inventory authority; all 49 entries receive payload and behavior assignments.', 'contracts/uctp-gate-6-requirements.json'),
 ('GAP6-09','Shared transitions', '7.2 / 7.4', 'Generic update details do not specify consultation, participant handoff or device replacement.', 'Typed session.consult, session.handoff and connection.replace with causal operation state.', 'contracts/connect-transitions.md'),
 ('GAP6-10','Snapshots and observations', '7 / 9 / 14', 'No complete durable cursor/lease contract or attributed agent observation surface.', 'Audience-filtered snapshots/events and typed provider observations with bounded delivery.', 'contracts/connect-events.md'),
 ('GAP6-11','Document versus envelope version', '16', 'Working-draft v0 label and wire integer 1 can be mistaken for release semver.', 'Envelope 2, native ALPN uctp/2, app connect/2; media header remains 1; crate semver independent.', 'docs/decisions/014-gate-6-contract-version.md'),
 ('GAP6-12','Membership and endpoint semantics', '5.7 / 7', 'Initial roles do not define current membership epochs, endpoint admission or history sharing.', 'Conversation-scoped Participant, distinct session presence and verified endpoint; explicit authority.', 'docs/decisions/013-gate-6-model.md'),
]

def build():
 text = SOURCE.read_text()
 lines = text.splitlines()
 sections = []
 units = []
 heading = 'Preamble'
 section = '0'
 buffer = []
 start = 1
 fence = False

 def flush(end):
  nonlocal buffer
  if not buffer:
   return
  content = '\n'.join(buffer).strip()
  buffer = []
  if not content or content == '---':
   return
  major = section.split('.')[0]
  component, owners, criteria = TASKS.get(major, ('source-context', [2, 13], [2]))
  modal = re.findall(r'\b(?:MUST NOT|MUST|SHOULD NOT|SHOULD|SHALL NOT|SHALL|REQUIRED|RECOMMENDED|MAY|OPTIONAL)\b', content)
  kind = 'wire-example' if content.startswith('```') else 'explicit-normative' if modal else 'behavior-or-context'
  if major == '13':
   kind = 'reserved'
  elif major == '14':
   kind = 'open-question-with-assigned-decision'
  elif major in ('0','15'):
   kind = 'context-cross-reference'
  applicability = 'mandatory-behavior'
  rationale = 'Declared wire/lifecycle behavior is an implementation obligation; example placeholders are illustrative, not literal required values.'
  if kind in ('reserved','context-cross-reference'):
   applicability = kind
   rationale = 'Reserved future namespace or explanatory cross-reference; retained in the audit rather than advertised as implemented behavior.'
  elif major == '14':
   applicability = 'explicit-decision'
   rationale = 'Open questions are resolved by the Gate 6 contracts or explicitly retained as reserved future work.'
  elif 'MUST' in modal or 'MUST NOT' in modal or 'REQUIRED' in modal:
   applicability = 'mandatory-when-condition-applies'
   rationale = 'The source condition and prohibition are retained verbatim; the assigned integration vector must exercise both admitted and rejected behavior.'
  elif 'MAY' in modal or 'OPTIONAL' in modal or section == '5.4':
   applicability = 'negotiated-or-policy-conditional'
   rationale = 'Explicit optional/policy behavior or installed auth backend is advertised honestly; required negotiation/refusal behavior still applies.'
  elif 'SHOULD' in modal or 'SHOULD NOT' in modal:
   applicability = 'recommended-with-explicit-deviation'
   rationale = 'Implement the recommendation or record a justified reviewed deviation; no silent downgrade of a mandatory feature.'
  identifier = f'U6-REQ-{len(units)+1:04}'
  units.append(dict(id=identifier,section=section,heading=heading,source_lines=[start,end],source_excerpt=content,classification=kind,applicability=applicability,applicability_rationale=rationale,modal_words=sorted(set(modal)),owner_component=component,implementation_items=[f'G6-{x:03}' for x in owners],acceptance=[f'A6-{x:02}' for x in criteria],verification=dict(case_id=identifier.lower(),lane='independent-conformance' if major not in ('0','13','14','15') else 'contract-decisions',positive='Exercise the stated behavior and response at the assigned source-bound integration case.',negative='Apply malformed/unauthorized/conflicting/replayed input where the source behavior admits input.',evidence=[]),runtime_status='not-run'))

 for number, line in enumerate(lines,1):
  match = re.match(r'^(#{2,6})\s+(.+)',line)
  if match and not fence:
   flush(number-1)
   heading = match[2]
   numeric = re.match(r'(\d+(?:\.\d+)*)\.?\s',heading)
   if numeric:
    section = numeric[1]
   component,owners,_ = TASKS.get(section.split('.')[0],('source-context',[2,13],[2]))
   sections.append(dict(section=section,heading=heading,line=number,owner_component=component,implementation_items=[f'G6-{x:03}' for x in owners]))
   continue
  if line.startswith('```'):
   if not buffer:
    start = number
   buffer.append(line)
   fence = not fence
   if not fence:
    flush(number)
   continue
  if not line.strip() and not fence:
   flush(number-1)
  else:
   if not buffer:
    start = number
   buffer.append(line)
 flush(len(lines))
 payloads = {}
 directory = SOURCE.parent.parent / 'crates/uctp/rvoip-uctp/src/payloads'
 for path in sorted(directory.glob('*.rs')):
  content = path.read_text()
  for match in re.finditer(r'pub struct (\w+)\s*\{(.*?)\n\}',content,re.S):
   fields = []
   for field in re.finditer(r'(?P<before>(?:(?:\s*#\[[^\n]+\]|\s*///[^\n]*)\n)*)\s*pub (\w+): ([^\n]+),',match[2]):
    fields.append(dict(name=field[2],rust_type=field[3],optional='Option<' in field[3] or 'default' in field['before']))
   payloads[match[1]]=dict(source=str(path.relative_to(ROOT / '.deps/rvoip')),fields=fields)
 registry = (directory.parent/'types.rs').read_text()
 names = re.findall(r'MessageType::\w+ => "([^"]+)"',registry)
 mapping = {'connection.ice-candidate':'IceCandidateInit','connection.ready':None}
 envelopes = []
 for i,name in enumerate(names,1):
  typename = mapping.get(name, ''.join(x.title() for x in re.split(r'[.\-]',name)))
  payload = payloads.get(typename)
  assigned = TASKS[{'auth':'5','identity':'5','conversation':'7','session':'7','connection':'7','stream':'7','message':'9','capability':'8','dtmf':'7','recording':'7','error':'11','ack':'11'}[name.split('.')[0]]]
  envelopes.append(dict(id=f'U6-{i:03}',type=name,payload_struct=typename,payload=payload,implementation_items=[f'G6-{x:03}' for x in assigned[1]],contract_gap='Readiness binding payload is constructed by coordinator; explicitly specify typed shape in G6-016.' if not payload else None,runtime_status='not-run'))
 assert len(names)==49 and len(set(names))==49
 return dict(gate=6,format=1,status='audited-implementation-contract-runtime-unqualified',source=dict(repository='eisenzopf/rvoip',revision=REVISION,path='docs/CONVERSATION_PROTOCOL.md',sha256=hashlib.sha256(text.encode()).hexdigest(),prepared_revision='ca7861af4c9920f6949422ae1f40507bf56ede97'),target=dict(envelope_version=2,native_alpn='uctp/2',application_profile='connect/2',media_datagram_version=1,media_profile='rtp-datagram/1',legacy_connect_compatibility=False),sections=sections,requirements=units,envelopes=envelopes,resolutions=[dict(id=x[0],title=x[1],source_sections=x[2],finding=x[3],resolution=x[4],contract=x[5],status='specified-runtime-not-qualified') for x in RESOLUTIONS])

def main():
 parser = argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--check',action='store_true')
 args = parser.parse_args()
 result = build()
 encoded = json.dumps(result,indent=2,ensure_ascii=False)+'\n'
 if args.check:
  if not OUTPUT.exists() or OUTPUT.read_text()!=encoded:
   raise SystemExit('UCTP ledger differs from frozen source/audit; review before regeneration')
 else:
  OUTPUT.write_text(encoded)
 print(json.dumps(dict(status='source-inventory-verified',sections=len(result['sections']),clauses=len(result['requirements']),envelopes=len(result['envelopes']),resolutions=len(result['resolutions']),runtime_qualification=False)))

if __name__=='__main__':
 main()
