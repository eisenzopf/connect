#!/usr/bin/env python3
"""Generate the implemented enrollment HTTP reference from Rust DTO schemas."""
import argparse
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]

def render():
    wire = json.loads((ROOT / 'contracts/schemas/enrollment-1.schema.json').read_text())
    def rewrite(v):
        if isinstance(v, list): return [rewrite(x) for x in v]
        if isinstance(v, dict): return {k:rewrite(x) for k,x in v.items()}
        if isinstance(v, str) and v.startswith('#/$defs/'): return '#/components/schemas/'+v[8:]
        return v
    schemas = rewrite(wire['$defs'])
    def field(name): return rewrite(wire['properties'][name])
    for name in ('offer_response','credential_response','revoke_response'):
        schemas[field(name)['$ref'].rsplit('/',1)[1]]['properties']['profile']={'const':'connect-enrollment/1'}
    schemas['EnrollmentOffer']['properties']['profile']={'const':'connect-enrollment/1'}
    schemas['EnrollmentRequest']['x-normalization']='Literal scopes/recipients/capabilities sort and deduplicate; address normalizes with the existing Endpoint validator. Validation cannot grant authority.'
    schemas['EnrollmentRequest']['x-time-bounds-seconds']={'challenge':600,'browser_credential':3600,'native_credential':86400,'issuer_ceiling':True}
    for name in ('EnrollmentRequest','EnrollmentOffer','EnrollmentClaim','EnrollmentCredential'):
        schemas[name]['x-sensitive']=True
    def content(schema): return {'application/json':{'schema':schema}}
    status = {200:'Original approved offer, admitted credential or revocation recovered under current authority.',400:'Malformed/closed JSON, invalid bounded intent, unsupported HTTP version or Upgrade.',401:'Current administrator unavailable or original issuer/child expired or revoked.',403:'Wrong host, Origin, route/body binding, bearer on public claim, provider headers, tenant/scope/key/proof authority or terminal approval.',409:'Changed original intent, reserved operation ID, Device/key/Endpoint or revision conflict.',410:'Expired pending challenge or tombstoned original receipt; no new grant.',413:'Encoded request body exceeds65536 bytes.',429:'Shared admission/delivery, durable receipt or32pending-per-Identity capacity exhausted.',503:'Authority/storage unavailable, cancellation or five-second observation deadline. A transaction may have committed; preserve original IDs and exact intent.'}
    def operation(name, request, response, admin, description, path=False):
        value = dict(operationId=name, security=[{'bearerAuth':[]}] if admin else [], description=description,
            requestBody=dict(required=True,content=content(field(request))),
            responses={str(code):dict(description=text,headers={'Cache-Control':{'schema':{'const':'no-store'}},'Pragma':{'schema':{'const':'no-cache'}},'Connection':{'schema':{'const':'close'}}},content=content(field(response))) for code,text in status.items()},
            **{'x-max-request-bytes':65536,'x-admission-deadline-seconds':5,'x-automatic-retries':0,'x-closed-query':True,'x-http-version':'1.1','x-response-custody':'Store authority and shared capacity retained by the physical socket until close, cancellation or earliest admission/credential/issuer expiry.'})
        if path: value['parameters']=[{'name':'id','in':'path','required':True,'schema':{'type':'string','pattern':r'^[A-Za-z0-9._~-]{1,128}$'}}]
        value['responses']['403']['content']=content({'oneOf':[field(response),field('hosting_rejection')]})
        return {'post':value}
    common = 'Host must match the configured HTTPS authority or concrete loopback listener/localhost:port. Query parameters, X-Call-ID, X-Chat-ID and Upgrade refuse. Closing HTTP/1.1 only; request body collection, Store work and physical response delivery share one five-second deadline. Cache-Control:no-store and Pragma:no-cache. An interrupted response can follow a commit: explicitly recover original intent; no automatic retry or renewal. '
    policy = {'physical':'Approved QUIC, WebTransport or WSS transport, exact driver-captured browser Origin (absent for QUIC), approved Device and exact signing key under the verified current Store lease before auth.session and credential refresh/step-up successor installation.',
              'http':'Finite application HTTP checks the exact enrolled Origin, including native absence. Explicit Task bearers retain their enrolled backing credential ceiling. This is an additional constraint, never operation authorization.',
              'current':'Current metadata/peer proofs require active original enrolled child/key/Device/Endpoint and receipt. Another active key attached to the same credential cannot replace the approved key ID.',
              'unbound':'Ingress without a trusted canonical physical context refuses enrolled credentials and their explicit Task bearers. Dedicated Origin-free record HTTPS cannot serve browser-enrolled credentials.',
              'rotation':'Use a fresh approval/key and fresh correctly bound peer; retiring the previous enrollment retains shared Device/Endpoint resources. No automatic renewal, transport migration or key substitution.'}
    paths = {
        '/v2/enrollments':operation('approveEnrollment','request','offer_response',True,common+'Administration surface only; absent from application, callback and health surfaces. Refuse any Origin. Authenticate current ordinary admin before parsing JSON; model/task grants cannot administer. Current target/recipient/tenant and issuer scope/expiry ceilings apply. Approval reserves distinct original approval/completion IDs and a fresh public key but creates no Device, Endpoint or credential. Challenge<=600seconds; browser credential<=3600seconds, QUIC<=86400seconds and within issuer expiry. IDs/scopes/sets/addresses normalize before immutable JCS intent. Store at most32unexpired pending approvals per Identity and reserve two receipt slots. Recover the same protected offer with original current issuer and exact intent. No provider/operator secret fields.'),
        '/v2/enrollments/{id}/claim':operation('claimEnrollment','claim','credential_response',False,common+'Application surface only. No bearer is accepted, including operator, ordinary client or task credentials. Exact Origin comes only from the request header; browser Origin must also be allowed by this HTTPS listener and equal the approved serialized HTTPS Origin. QUIC claims require no Origin. Path id must equal body enrollment_id. Verify canonical64-byte raw Ed25519 or ES256(SHA256) signature over ASCII(connect-enrollment-proof/1)||NUL||SHA256(JCS(normalized full offer)); challenge alone grants nothing. Wrong bindings make no effects; five matching invalid signatures exhaust pending approval. Fresh valid claim atomically creates scoped credential, immutable Device/key binding and unverified Endpoint, or reuses the exact active Endpoint for rotation; late conflicts roll back. Completed signed recovery may outlive challenge expiry but rechecks current original issuer, child, key, Device, Endpoint, recipient ceilings and original receipt; it returns the same bearer and absolute expiry. Provider consent, route verification/reachability and media readiness are separate.',True),
        '/v2/enrollments/{id}/revoke':operation('revokeEnrollment','revoke','revoke_response',True,common+'Administration surface only, absent from application/callback/health. Refuse any Origin and authenticate current ordinary admin before parsing. Current tenant/target scope required, including replay. A currently authorized backend can revoke even when the original issuer is inactive. Revoke pending approval/completion or completed child credential/key atomically; retain shared Device/Endpoint so an admitted replacement can continue. Preserve original revoke operation ID. No new credential or expiry extension.',True),
    }
    return json.dumps(dict(openapi='3.1.0',info=dict(title='Connect remote enrollment HTTP',version='connect-enrollment/1',description='Implemented HTTP component and Store contract. The legacy loopback HTTP listener exposes the same bounded routes for local use. Remote use requires explicit HTTPS hosting and trusted certificates. Physical transport, exact accepted Origin, Device and key constraints apply before canonical peer acceptance and successor installation; finite HTTP and explicit Task bearers retain the enrolled Origin ceiling. Browser/native public enrollment installers and complete end-to-end qualification remain separate G6-055 work; schema generation alone is not runtime qualification.'),paths=paths,**{'x-connect-enrolled-credential-admission':policy},components=dict(securitySchemes={'bearerAuth':dict(type='http',scheme='bearer')},schemas=schemas)),indent=2)+'\n'

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--check',action='store_true');args=parser.parse_args();path=ROOT/'contracts/openapi-enrollment.json';value=render()
    if args.check: assert path.read_text()==value, 'Enrollment HTTP reference is stale'
    else: path.write_text(value)
if __name__=='__main__':main()
