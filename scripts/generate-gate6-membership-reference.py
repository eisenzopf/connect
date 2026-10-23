#!/usr/bin/env python3
"""Closed schema-34 shared membership control HTTP reference."""
import json,pathlib
def obj(properties,required=None):return dict(type='object',additionalProperties=False,properties=properties,required=list(properties) if required is None else required)
def ref(name):return {'$ref':'#/components/schemas/'+name}
def nullable(value):return {'anyOf':[value,{'type':'null'}]}
ROOT=pathlib.Path(__file__).resolve().parents[1]
def main():
 identifier=dict(type='string',minLength=1,maxLength=128,pattern='^[A-Za-z0-9._~-]+$')
 integer=dict(type='integer',minimum=0,maximum=9007199254740991)
 epoch=dict(type='integer',minimum=1,maximum=9007199254740991)
 text=lambda maximum:dict(type='string',minLength=1,maxLength=maximum,pattern=r'^[^\x00-\x1f\x7f-\x9f]*$')
 role=text(64);kind={'enum':['human','ai','system','external']}
 member=obj(dict(participant_id=identifier,conversation_id=identifier,identity_id=identifier,kind=kind,role=role,membership_epoch=epoch,active={'type':'boolean'},removal_reason=nullable(dict(type='string',maxLength=512,pattern=r'^[^\x00-\x1f\x7f-\x9f]*$'))))
 invitation=obj(dict(invitation_id=identifier,conversation_id=identifier,identity_id=identifier,expires_at=epoch,state={'enum':['pending','joined','declined','cancelled','expired']},conversation_revision=integer))
 content=obj(dict(kind={'enum':['message','artifact','session','call','sip_leg','context_delivery','record','transcript','observation']},id=text(256)))
 actions=[]
 for action in ('add','invite','join','decline','cancel_invitation','leave','remove','update_role','share_history','revoke_history_share'):
  fields={'action':{'const':action}}
  if action in ('add','invite'):fields.update(identity_id=identifier,kind=kind,role=role)
  if action=='invite':fields['expires_at']=epoch
  if action in ('join','decline','cancel_invitation'):fields['invitation_id']=identifier
  if action in ('remove','update_role'):fields['participant_id']=identifier
  if action=='remove':fields['reason']=member['properties']['removal_reason']
  if action=='update_role':fields['role']=role
  if action=='share_history':fields.update(resources=dict(type='array',minItems=1,maxItems=100,uniqueItems=True,items=ref('ContentRef')),participants=dict(type='array',minItems=1,maxItems=100,uniqueItems=True,items=identifier),provenance=text(512))
  if action=='revoke_history_share':fields['sharing_id']=identifier
  required=[k for k in fields if k!='reason'];actions.append(obj(fields,required))
 base=json.loads((ROOT/'contracts/openapi-operations.json').read_text())
 receipt=json.loads((ROOT/'contracts/openapi-conversations.json').read_text())['components']['schemas']['MutationReceipt']
 schemas=dict(Member=member,Invitation=invitation,ContentRef=content,MembershipAction={'oneOf':actions},MembershipRequest=obj(dict(operation_id=identifier,conversation_id=identifier,expected_revision=dict(type='integer',minimum=0,maximum=9007199254740990),change=ref('MembershipAction'))),MutationReceipt=receipt)
 schemas['MembershipOutcome']={'oneOf':[obj(dict(kind={'const':'participant'},participant=ref('Member'),revision=integer,receipt=ref('MutationReceipt'))),obj(dict(kind={'const':'invitation'},invitation=ref('Invitation'),receipt=ref('MutationReceipt'))),obj(dict(kind={'const':'sharing'},sharing_id=identifier,conversation_id=identifier,revision=integer,active={'type':'boolean'},receipt=ref('MutationReceipt')))]}
 schemas['MembershipSnapshot']=obj(dict(conversation_id=identifier,revision=integer,participants=dict(type='array',maxItems=128,items=ref('Member'))))
 schemas['InvitationPage']=obj(dict(invitations=dict(type='array',maxItems=100,items=ref('Invitation')),next=nullable(dict(type='string',minLength=1,maxLength=2048))))
 schemas['Operation']={'oneOf':[obj(dict(op={'const':'membership'},request=ref('MembershipRequest'))),obj(dict(op={'const':'membership_snapshot'},conversation_id=identifier)),obj(dict(op={'const':'membership_invitations'},after=nullable(dict(type='string',maxLength=2048)),limit=dict(type='integer',minimum=1,maximum=100)),['op','limit'])]}
 schemas['Outcome']={'oneOf':[obj(dict(kind={'const':kind},data=ref(name))) for kind,name in [('membership','MembershipOutcome'),('membership_snapshot','MembershipSnapshot'),('membership_invitations','InvitationPage')]]}
 schemas.update(Error=base['components']['schemas']['Error'],Response=base['components']['schemas']['Response'])
 api={**base,'info':dict(title='Connect membership control',version='schema-34',description='Fresh candidate state only. All ten explicit membership actions use the ordinary durable control ledger and current authority. Add is explicit admission, separate from invitation target acceptance. Role labels grant no capabilities. Readmission increments membership epoch. Outcomes include local-mutation receipts, never media completion. Wire string byte limits and safe integers are checked by the protocol; JSON Schema maxLength counts characters.'),'components':dict(securitySchemes=base['components']['securitySchemes'],schemas=schemas)}
 route=api['paths']['/v1/operations']['post'];route['operationId']='membershipControl';route['description']='Membership requires the action-specific named scope, current manager/member or invitation-target authority, same tenant and recipient ceilings. Preserve operation_id, expected_revision and normalized action for identical replay. A changed intent conflicts. Original member epoch is immutable; current authorization is required for status and replay. Only the original same-epoch self-removal receipt is readable while inactive. Query snapshots require current membership. Personal invitations are pending/live/unexpired only, count <=100 and encoded page <=384KiB, using a 30-second identity/credential/tenant/scope-bound opaque fixed-high-water cursor. New inserts require a fresh query; it is not a retained immutable snapshot.'
 route['responses']['200']['description']='Typed current membership projection with exact local-mutation receipt, bounded membership query, or typed business refusal. Storage and other infrastructure refusals create no logical operation ID.'
 route['responses'].update({'413':{'description':'Encoded request exceeds configured body budget.'},'422':{'description':'Malformed JSON or unknown closed typed field is refused before service execution.'}})
 (ROOT/'contracts/openapi-membership.json').write_text(json.dumps(api,indent=2)+'\n')
if __name__=='__main__':main()
