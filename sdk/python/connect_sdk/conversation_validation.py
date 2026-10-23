"""Bounded current conversation views shared by all transport clients."""
import math


def _metadata(value, depth=0):
    if depth > 16:
        return False
    if value is None or type(value) in (str, bool):
        return True
    if type(value) is int:
        return abs(value) <= 9007199254740991
    if type(value) is float:
        return math.isfinite(value) and (not value.is_integer() or abs(value) <= 9007199254740991)
    if isinstance(value, list):
        return all(_metadata(v, depth+1) for v in value)
    return isinstance(value, dict) and all(isinstance(k, str) and _metadata(v, depth+1) for k, v in value.items())


def valid_conversation(data, valid_receipt):
    keys={'id','tenant_id','policy','idle_close_secs','metadata','participants','opened_at','revision','closed','closed_at','reason_code','reason'}
    if not isinstance(data,dict) or not keys <= data.keys() or data.keys()-keys-{'receipt'}:
        return False
    if not all(isinstance(data[k],str) and data[k] for k in ('id','tenant_id')) or data['policy'] not in ('persistent','ephemeral'):
        return False
    idle=data['idle_close_secs']
    if (idle is not None and (type(idle) is not int or not 1<=idle<=604800)) or (data['policy']=='ephemeral' and idle is None):
        return False
    if not _metadata(data['metadata']) or any(type(data[k]) is not int or not 0<=data[k]<=9007199254740991 for k in ('opened_at','revision')) or type(data['closed']) is not bool:
        return False
    if data['closed']:
        if type(data['closed_at']) is not int or data['closed_at']<0 or type(data['reason_code']) is not int or not 0<=data['reason_code']<=9007199254740991 or not isinstance(data['reason'],str):
            return False
    elif any(data[k] is not None for k in ('closed_at','reason_code','reason')):
        return False
    ps=data['participants']
    if not isinstance(ps,list) or len(ps)>128:
        return False
    seen=set()
    for p in ps:
        if not isinstance(p,dict) or set(p)!={'participant_id','identity_id','kind','role','display_name'} or not all(isinstance(p[k],str) and p[k] for k in ('participant_id','identity_id','role')) or p['kind'] not in ('human','ai','system','external') or (p['display_name'] is not None and not isinstance(p['display_name'],str)) or p['participant_id'] in seen:
            return False
        seen.add(p['participant_id'])
    return 'receipt' not in data or (isinstance(data['receipt'],dict) and isinstance(data['receipt'].get('operation_id'),str) and bool(data['receipt']['operation_id']) and valid_receipt(data['receipt'],data['receipt']['operation_id'],data['id']))
