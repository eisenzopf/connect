"""Closed membership outcomes and exact local mutation references."""
def closed(v,keys):return isinstance(v,dict) and set(v)==set(keys.split())
def text(v,maximum=128):return isinstance(v,str) and 0<len(v.encode("utf-8",errors="surrogatepass"))<=maximum and not any(ord(c)<32 or 127<=ord(c)<=159 for c in v)
def identifier(v):return text(v) and all(c.isascii() and (c.isalnum() or c in "._~-") for c in v)
def number(v):return type(v) is int and 0<=v<=9007199254740991
def epoch(v):return number(v) and v>0
def reason(v):return v is None or v=="" or text(v,512)
def member(v):return closed(v,'participant_id conversation_id identity_id kind role membership_epoch active removal_reason') and all(identifier(v[k]) for k in ('participant_id','conversation_id','identity_id')) and v['kind'] in ('human','ai','system','external') and text(v['role'],64) and epoch(v['membership_epoch']) and type(v['active']) is bool and reason(v['removal_reason'])
def invitation(v):return closed(v,'invitation_id conversation_id identity_id expires_at state conversation_revision') and all(identifier(v[k]) for k in ('invitation_id','conversation_id','identity_id')) and epoch(v['expires_at']) and number(v['conversation_revision']) and v['state'] in ('pending','joined','declined','cancelled','expired')
def receipt(v,cid):
 from .protocol import valid_mutation_receipt
 return isinstance(v,dict) and identifier(v.get('operation_id')) and valid_mutation_receipt(v,v['operation_id'],cid)
def valid_membership(kind,v):
 if not isinstance(v,dict):return False
 if kind=='membership_snapshot':return closed(v,'conversation_id revision participants') and identifier(v['conversation_id']) and number(v['revision']) and isinstance(v['participants'],list) and len(v['participants'])<=128 and all(member(p) and p['conversation_id']==v['conversation_id'] for p in v['participants'])
 if kind=='membership_invitations':return closed(v,'invitations next') and isinstance(v['invitations'],list) and len(v['invitations'])<=100 and all(invitation(i) for i in v['invitations']) and (v['next'] is None or text(v['next'],2048))
 if v.get('kind')=='participant':return closed(v,'kind participant revision receipt') and member(v['participant']) and number(v['revision']) and receipt(v['receipt'],v['participant']['conversation_id'])
 if v.get('kind')=='invitation':return closed(v,'kind invitation receipt') and invitation(v['invitation']) and receipt(v['receipt'],v['invitation']['conversation_id'])
 if v.get('kind')=='sharing':return closed(v,'kind sharing_id conversation_id revision active receipt') and identifier(v['sharing_id']) and identifier(v['conversation_id']) and number(v['revision']) and type(v['active']) is bool and receipt(v['receipt'],v['conversation_id'])
 return False
