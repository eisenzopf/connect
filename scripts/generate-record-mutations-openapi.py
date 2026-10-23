#!/usr/bin/env python3
"""Render the explicit local record erasure reference; no runtime proof."""
import argparse,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def render():
 def obj(properties,required=None):return dict(type='object',properties=properties,required=list(properties) if required is None else required,additionalProperties=False)
 ident=dict(type='string',pattern=r'^[A-Za-z0-9._~-]{1,128}$')
 rev=dict(type='integer',minimum=1,maximum=9007199254740991)
 uuid=dict(type='string',format='uuid',**{'not':{'const':'00000000-0000-0000-0000-000000000000'}},pattern=r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')
 target={'oneOf':[obj(dict(kind=dict(const=kind))) for kind in ['document','session-outcome','capture-diagnostics']] + [
  obj(dict(kind=dict(const='participant'),participant_id=ident,membership_epoch=rev)),obj(dict(kind=dict(const='connection'),connection_id=ident,generation=rev))]}
 source=obj(dict(target=target,mode=dict(enum=['content','metadata'])))
 action={'oneOf':[obj(dict(kind=dict(const='redact'),sources=dict(type='array',minItems=1,maxItems=128,uniqueItems=True,items=source,**{'x-unique-targets':True}))),obj(dict(kind=dict(const='delete')))]}
 request=obj(dict(operation_id=ident,conversation_id=ident,session_id=ident,uuid=uuid,expected_revision=dict(type='integer',minimum=1,maximum=9007199254740990),action=action))
 outcome=obj(dict(operation_id=ident,uuid=uuid,revision=rev,action=dict(enum=['redact','delete']),at=dict(type='string',format='date-time'),remote_deletion=dict(const='not-attempted')))
 response=obj(dict(profile=dict(const='connect/2'),result={'oneOf':[obj(dict(Ok={'$ref':'#/components/schemas/Outcome'})),obj(dict(Err={'$ref':'#/components/schemas/Error'}))]}))
 schemas=dict(Request=request,Outcome=outcome,Response=response,Error=obj(dict(code=dict(type='string',maxLength=128),message=dict(type='string',maxLength=2048))))
 statuses={200:'Original local mutation committed, or its exact receipt recovered under current authority.',400:'Invalid bounded request, malformed input or unsupported canonical redaction.',401:'Current credential unavailable.',403:'Named permission or current source footprint denied; unknown and unauthorized semantic source selectors share this refusal.',404:'Record or scope unavailable.',409:'Original actor/intent or expected revision conflicts.',413:'Encoded body exceeds64KiB.',429:'Mutation or journal capacity exhausted; transaction unchanged.',501:'Record mutation policy unavailable.',503:'Authority/storage unavailable or acceptance uncertain. Preserve original operation ID and complete request.'}
 return json.dumps(dict(openapi='3.1.0',info=dict(title='Connect local record mutations',version='connect/2',description='Named HTTP application API for current metadata outcome records; no physical-peer grant. Local erasure preserves private replay authority facts and reports remote_deletion:not-attempted. Publication, remote operating profiles and full G6-046 qualification are separate.'),paths={'/v2/records/{uuid}/mutations':{'post':dict(operationId='mutateRecord',security=[dict(bearerAuth=[])],parameters=[dict(name='uuid',**{'in':'path'},required=True,schema=uuid)],description='Authentication precedes parsing. Route and body UUID must agree. Ordinary callers require record_get and record_redact/record_delete plus current original Session source access; selected and actual dependency changes must all be authorized. Independent installation erasure requires current admin and record_admin plus the action permission. Neither membership nor role labels alone grants mutation. Expected revision binds the original intent without exposing full canonical hashes. Replays bind original identity and exact request (including array order), recheck current permissions/recipient ceilings/source epochs and return the original fact even after later deletion. Successful record availability journal hints commit atomically; replay appends none. Core canonical/dependency validation remains required. No provider deletion or physical disk sanitization is asserted.',requestBody=dict(required=True,content={'application/json':dict(schema={'$ref':'#/components/schemas/Request'})}),responses={str(code):dict(description=description,content={'application/json':dict(schema={'$ref':'#/components/schemas/Response'})}) for code,description in statuses.items()},**{'x-max-request-bytes':65536,'x-deadline-seconds':5,'x-cache-control':'no-store'})}},components=dict(securitySchemes=dict(bearerAuth=dict(type='http',scheme='bearer')),schemas=schemas)),indent=2)+'\n'
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--check',action='store_true');a=p.parse_args();path=ROOT/'contracts/openapi-record-mutations.json';value=render()
 if a.check:assert path.read_text()==value,'Record mutation reference is stale'
 else:path.write_text(value)
if __name__=='__main__':main()
