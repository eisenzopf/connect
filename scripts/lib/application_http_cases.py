"""Actual named-operation wire/dispatch cases for the isolated protected checker.

No provider/model account or packet/media qualification. Synthetic IDs intentionally
exercise typed refusals; curated reads/mutations use real fixture resources.
"""
import json
from pathlib import Path
import time

ROOT=Path(__file__).resolve().parents[2]


def qualify(exchange,validate,document,operator,reviews,agent,grant,cid,task_id):
    contract=("application","/v1/operations")
    def check(case,body,token=None,**kwargs):
        return exchange("application-"+case,"POST","/v1/operations",token or operator['credential'],
                        body=body,contract=contract,**kwargs)
    def success(case,body,kind,token=None):
        status,response,_,_=check(case,body,token,expected=200)
        assert response['result'].get('Ok',{}).get('kind')==kind,case
        return response['result']['Ok']['data']
    catalog=json.loads((ROOT/'contracts/application-http-dispatch.json').read_text())
    matrix=[]
    for index,row in enumerate(catalog['cases']+catalog['task_cases']):
        body=row['sample'];supported=row['response_kind'] is not None
        validate(document,{'$ref':'#/components/schemas/WireOperation'},body,'wire-'+str(index))
        status,result,_,_=check('dispatch-'+str(index),body,request_valid=supported)
        assert status in ({200,403,501} if supported else {501}),(index,status)
        if not supported:assert result['result']['Err']['code']=='unsupported-command'
        if 'Ok' in result['result']:assert result['result']['Ok']['kind']==row['response_kind']
        # Authentication precedes dispatch, including a recognized retired DTO.
        exchange('application-no-auth-'+str(index),'POST','/v1/operations',None,body=body,
                 contract=contract,request_valid=supported,expected=401)
        selector=body['op']
        if selector=='membership':selector+='.'+body['request']['change']['action']
        elif selector=='message_receipt':selector+='.'+body['state']
        elif selector=='task':selector+='.'+body['request']['command']['action']
        matrix.append(dict(selector=selector,op=body['op'],supported=supported,status=status,
                           response_kind=result['result'].get('Ok',{}).get('kind'),
                           error_code=result['result'].get('Err',{}).get('code')))

    curated=[]
    def ok(case,body,kind,token=None):
        value=success(case,body,kind,token);curated.append(dict(case=case,op=body['op'],kind=kind));return value
    ok('health',dict(op='health'),'health')
    identity=ok('resolve',dict(op='resolve',address='connect:agent@local'),'identity')
    assert identity['id']==agent['identity_id']
    ok('capabilities',dict(op='capabilities',address='connect:agent@local',conversation_id=None),'capabilities')
    _,unconfigured,_,_=check('browser-binding-unconfigured',dict(op='browser_binding'),expected=200)
    assert unconfigured['result']['Err']['code']=='unsupported-operation'
    ok('contact-state',dict(op='contact_state',peer_identity_id=reviews['identity_id']),'contact')
    pairing_request=dict(op='contact_pairing_start',request_id='application-pairing-original',
                         peer_identity_id=reviews['identity_id'],expires_at=int(time.time())+120)
    pairing=ok('pairing-start',pairing_request,'contact_pairing',agent['credential'])
    replay=ok('pairing-replay',pairing_request,'contact_pairing',agent['credential']);assert replay==pairing
    assert pairing['challenge']
    # Only the initiating owner sees the one-use challenge. Peer completes with
    # its own current Identity credential; a configured caller ID is not consent.
    completed=ok('pairing-complete',dict(op='contact_pairing_complete',request_id='application-pairing-complete',
        pairing_id=pairing['id'],challenge=pairing['challenge']),'contact_pairing',reviews['credential'])
    assert completed['state']=='verified_native' and 'challenge' not in completed
    contact=ok('paired-state',dict(op='contact_state',peer_identity_id=reviews['identity_id']),'contact',agent['credential'])
    assert contact['sms_consent'] is False
    ok('pairing-revoke',dict(op='contact_pairing_revoke',request_id='application-pairing-revoke',pairing_id=pairing['id']),'contact_pairing',agent['credential'])
    availability=dict(op='availability_set',request_id='application-availability-original',state='available',
                      preferred_route=None,expires_at=int(time.time())+120)
    observed=ok('availability-set',availability,'availability')
    assert ok('availability-replay',availability,'availability')==observed
    ok('availability-get',dict(op='availability_get',identity_id=operator['identity_id']),'availability')
    ok('inbox',dict(op='inbox',cursor=None,limit=10,wait_ms=0),'inbox')
    ok('membership-snapshot',dict(op='membership_snapshot',conversation_id=cid),'membership_snapshot')
    ok('membership-invitations',dict(op='membership_invitations',after=None,limit=10),'membership_invitations')
    ok('context-view',dict(op='context_view',conversation_id=cid,limit=10),'context_view')
    ok('context-deliveries',dict(op='context_deliveries',conversation_id=cid,before=None,limit=10),'context_deliveries')
    ok('provider-status',dict(op='provider_status',conversation_id=cid),'provider_status')
    ok('preferences-get',dict(op='receipt_preferences_get',conversation_id=cid),'receipt_preferences')
    preferences=dict(op='receipt_preferences_set',request_id='application-preferences-original',conversation_id=cid,delivery=True,read=False)
    set_preferences=ok('preferences-set',preferences,'receipt_preferences')
    assert ok('preferences-replay',preferences,'receipt_preferences')==set_preferences
    def task(command):return dict(op='task',request=dict(task_profile='connect-task/1',command=command))
    ok('task-get',task(dict(action='get',task_id=task_id)),'task')
    ok('task-replies',task(dict(action='replies',conversation_id=cid,after=None,limit=10)),'task')
    ok('task-inbox',task(dict(action='inbox',task_id=task_id,after=None,limit=10,wait_ms=0)),'task')
    ok('task-recall',task(dict(action='recall',task_id=task_id,after=None,limit=10)),'task')
    ok('task-wait',task(dict(action='wait',task_id=task_id,after=None,limit=10,wait_ms=0)),'task')
    ok('grant-task-get',task(dict(action='get',task_id=task_id)),'task',grant['bearer'])
    check('grant-cannot-create',task(dict(action='create',request_id='application-forged-task',definition=dict(
        conversation_id=cid,scope=dict(operations=['history'],recipients=[reviews['identity_id']]),
        expires_at=int(time.time())+60,initiating_event_id=None))),grant['bearer'],expected=403)
    check('grant-provider-header',task(dict(action='get',task_id=task_id)),grant['bearer'],headers=[('X-Call-ID','not-origin-proof')],expected=403)

    # Framework parsing precedes authentication on this named route, unlike the
    # separate protected issuance exchange. Never document all refusals as JSON.
    for case,raw,content_type,status in [
        ('bad-json',b'{','application/json',400),
        ('bad-content-type',b'{}','text/plain',415),
        ('unknown-op',b'{"op":"unknown"}','application/json',422),
        ('authority-selector',b'{"op":"health","actor":"forged"}','application/json',422),
        ('duplicate-tag',b'{"op":"health","op":"health"}','application/json',422),
        ('oversize',b' '*65537,'application/json',413),
    ]:
        exchange('application-'+case,'POST','/v1/operations','invalid',raw=raw,
                 headers=[('Content-Type',content_type)],expected=status,contract=contract,
                 request_valid=case=='duplicate-tag')
    return dict(scope='Actual isolated current CLI/server application serde/dispatch and selected resource/Task/read/replay authority. No provider call or media/host/remote qualification.',
                dispatch_cases=len(matrix),supported_cases=sum(r['supported'] for r in matrix),
                retired_cases=sum(not r['supported'] for r in matrix),authentication_cases=len(matrix),
                matrix=matrix,curated=curated,
                feature_refusals=[dict(case='browser-binding-unconfigured',status=200,error_code='unsupported-operation')],
                task_create_schema_checked=True)
