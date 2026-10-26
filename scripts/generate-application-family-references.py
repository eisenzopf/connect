#!/usr/bin/env python3
"""Reconcile shared family HTTP descriptions with current executable dispatch.

Preserve shared DTO/policy material; stop advertising retired HTTP successes.
This is source/reference metadata, not runtime qualification.
"""
import argparse
import copy
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]


def tags(value,schemas):
    if isinstance(value,dict):
        if 'op' in value.get('properties',{}):
            tag=value['properties']['op'];return {tag['const']} if 'const' in tag else set(tag['enum'])
        if '$ref' in value:return tags(schemas[value['$ref'].split('/')[-1]],schemas)
        return set().union(*(tags(v,schemas) for v in value.values()))
    if isinstance(value,list):return set().union(*(tags(v,schemas) for v in value))
    return set()


def render(document,dispatch):
    document=copy.deepcopy(document);post=document['paths']['/v1/operations']['post'];schemas=document['components']['schemas']
    selected=tags(schemas['Operation'],schemas);assert selected
    rows=[row for row in dispatch if row['selector'].split('.')[0] in selected]
    assert selected=={row['selector'].split('.')[0] for row in rows}
    supported=[row['selector'] for row in rows if row['supported']]
    retired=[row['selector'] for row in rows if not row['supported']]
    post.setdefault('x-connect-family-policy-notes',post.get('description',document['info'].get('description','')))
    document.setdefault('x-connect-original-description',document['info'].get('description',''))
    document['info']['description']='Shared DTO/policy family. Current server HTTP body/dispatch authority is openapi-application-http.json. Historical top-level success claims are retired explicitly; original domain-policy notes remain in x-connect-original-description and x-connect-family-policy-notes.'
    post['description']='Shared family wire models. Current named selectors: '+(', '.join(supported) if supported else 'none')+'. Retired selectors: '+(', '.join(retired) if retired else 'none')+'. Recognized retired top-level calls return HTTP501 after current authentication, without compatibility/fallback dispatch. Use /v2/commands for canonical communication. Current named calls retain service authorization and business-state checks. See openapi-application-http.json for the consolidated body/result/profile/framework contract; these DTOs may also be used inside separately authorized Task workflows.'
    post['x-connect-current-application-reference']='openapi-application-http.json#/paths/~1v1~1operations/post'
    post['x-connect-supported-selectors']=supported;post['x-connect-retired-selectors']=retired
    if not supported:
        post['deprecated']=True
        if '200' in post['responses']:
            post['x-connect-former-success-response']=post['responses'].pop('200')
    elif 'deprecated' in post:post.pop('deprecated')
    post['responses']['501']=dict(description='Recognized retired top-level family command/action, or an explicitly unsupported named capability; no fallback dispatch.',content={'application/json':{'schema':{'$ref':'#/components/schemas/Response'}}})
    for code,description in {
        '400':'Framework malformed JSON rejection before authentication.',
        '413':'Framework aggregate body limit before authentication: max(configured artifact_chunk_bytes,65536), default65536.',
        '415':'Framework JSON content type required before authentication.',
        '422':'Framework serde shape/type/enum/duplicate-field rejection before authentication.',
    }.items():post['responses'][code]=dict(description=description,content={'text/plain':{'schema':{'type':'string'}}})
    return document


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--check',action='store_true');args=parser.parse_args()
    dispatch=json.loads((ROOT/'contracts/openapi-application-http.json').read_text())['x-connect-dispatch'];rows=[]
    for path in sorted((ROOT/'contracts').glob('openapi-*.json')):
        if path.name=='openapi-application-http.json':continue
        document=json.loads(path.read_text())
        if '/v1/operations' not in document['paths']:continue
        current=render(document,dispatch);content=json.dumps(current,indent=2)+'\n'
        if args.check:assert path.read_text()==content,str(path)+' family dispatch reference is stale'
        else:path.write_text(content)
        post=current['paths']['/v1/operations']['post'];rows.append(dict(document=path.name,supported=len(post['x-connect-supported-selectors']),retired=len(post['x-connect-retired-selectors'])))
    assert len(rows)==8
    print(json.dumps(dict(scope='Family source/reference metadata only; no runtime qualification',families=rows)))


if __name__=='__main__':main()
