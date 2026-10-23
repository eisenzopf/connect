#!/usr/bin/env python3
"""Generate the closed current control reference for G6-036; no wire qualification."""
import argparse,copy,json,pathlib
ROOT=pathlib.Path(__file__).resolve().parents[1]
def ref(name):return {'$ref':'#/components/schemas/'+name}
def obj(properties,required=None):return dict(type='object',additionalProperties=False,properties=properties,required=list(properties) if required is None else required)
def nullable(value):return {'anyOf':[value,{'type':'null'}]}
def array(value,maximum):return dict(type='array',items=value,maxItems=maximum)
def union(*values):return dict(oneOf=list(values))
def tagged(kind,data):return obj({'kind':{'const':kind},'data':data})
N=dict(type='integer',minimum=0,maximum=9007199254740991);P=dict(N,minimum=1);B={'type':'boolean'}
ID=dict(type='string',minLength=1,maxLength=512,pattern=r'^[^\u0000-\u001f\u007f]+$')
INPUT=dict(type='string',minLength=1,maxLength=128,pattern='^[A-Za-z0-9._~-]+$')
TOKEN=dict(type='string',minLength=4,maxLength=2048,pattern='^ev1_[A-Za-z0-9_-]+$')
S={'type':'string'}
def enum(*values):return {'enum':list(values)}
def schemas():
    out={}
    def prefixed(value,prefix):
        if isinstance(value,dict):return {key:('#/components/schemas/'+prefix+item.split('/')[-1] if key=='$ref' and item.startswith('#/components/schemas/') else prefixed(item,prefix))for key,item in value.items()}
        if isinstance(value,list):return [prefixed(item,prefix)for item in value]
        return value
    for filename,prefix in [('openapi-messages.json','Msg_'),('openapi-operations.json','Ledger_')]:
        source=json.loads((ROOT/'contracts'/filename).read_text())['components']['schemas']
        out.update({prefix+key:prefixed(value,prefix)for key,value in source.items()})
    # Existing trusted provider messages use opaque voice:<session>:<digest> IDs.
    out['Msg_Message']['properties']['id']=copy.deepcopy(ID)
    out['Msg_Message']['properties']['in_reply_to_msg']=nullable(copy.deepcopy(ID))
    session=obj(dict(id=ID,conversation_id=ID,initiator_id=ID,participant_id=ID,state=ID,media=array(ID,16),created_at=N,updated_at=N,ended_reason=nullable(S)))
    out['Session']=session
    out['SessionSnapshot']=obj(dict(id=ID,conversation_id=ID,state=ID,media=array(ID,16),intent=nullable(S),capabilities_offer=nullable({'type':'object'}),codec_preferences=array(S,16),is_main=B,revision=N,topology_revision=N,created_at=N,updated_at=N,ended_reason=nullable(S)))
    out['SessionParticipantJoined']=obj(dict(fact_id=ID,session=ref('SessionSnapshot'),membership_epoch=P,participant=obj(dict(participant_id=ID,identity_id=ID,kind=enum('human','ai','system','external'),role=dict(S,minLength=1,maxLength=64),display_name=dict(S,minLength=1,maxLength=256),joined_at=P)),via_connection=ID))
    out['SessionParticipantJoined']['properties']['session']={'allOf':[ref('SessionSnapshot'),{'properties':{'state':{'const':'active'}}}]}
    out['SessionParticipantLeft']=obj(dict(fact_id=ID,session=ref('SessionSnapshot'),membership_epoch=P,participant_id=ID,left_at=P,reason=enum('explicit-leave','all-connections-ended','kicked','transferred-out')))
    out['SessionStartedFact']=obj(dict(session=ref('SessionSnapshot'),started_at=P,participants_present=dict(type='array',items=ID,minItems=1,maxItems=256,uniqueItems=True),active_connections=dict(type='array',minItems=1,maxItems=256,items=obj(dict(connid=ID,participant_id=ID,transport=enum('quic','webtransport','websocket+webrtc','sip-interop','webrtc-interop')))),negotiated_capabilities={'type':'object'}))
    out['SessionEndedFact']=obj(dict(session=ref('SessionSnapshot'),by_participant=nullable(ID),by_membership_epoch=nullable(P),reason_code=dict(N,maximum=65535),reason=dict(S,maxLength=512,pattern=r'^[^\u0000-\u001f\u007f-\u009f]*$',**{'x-max-utf8-bytes':512}),ended_at=P))
    out['SessionEndedFact']['allOf']=[{'if':{'properties':{'by_participant':{'type':'null'}}},'then':{'properties':{'by_membership_epoch':{'type':'null'}}},'else':{'properties':{'by_membership_epoch':P}}}]
    out['SessionEndedFact']['description']='Immutable original terminal Session snapshot and attribution, retained independently of later vCon storage/retention. Runtime paths without a supplied numeric reason use0 (unspecified); no provider status is inferred. Current Session source authority is still required on every read.'
    out['ConnectionPrepared']=obj(dict(connection_id=ID,conversation_id=ID,session_id=ID,participant_id=ID,membership_epoch=P,endpoint_id=ID,generation={'const':1},revision={'const':0},state={'const':'preparing'}))
    out['Member']=obj(dict(participant_id=ID,conversation_id=ID,identity_id=ID,kind=enum('human','ai','system','external'),role=dict(S,minLength=1,maxLength=64),membership_epoch=P,active=B,removal_reason=nullable(dict(S,maxLength=512))))
    out['ConversationState']=obj(dict(conversation_id=ID,revision=N,closed=B,policy=enum('persistent','ephemeral')))
    out['ConversationClosedFact']=obj(dict(conversation_id=ID,reason_code=N,reason=dict(S,maxLength=512,pattern=r'^[^\u0000-\u001f\u007f-\u009f]*$',**{'x-max-utf8-bytes':512}),closed_at=P))
    out['ConversationClosedFact']['description']='Immutable original closure shared by the cursor-bearing journal fact and signed conversation.closed observation. Observations remain hints; current membership and recipient ceilings govern every actual delivery.'
    out['Appearance']=obj(dict(session_id=ID,participant_id=ID,membership_epoch=P,role=dict(S,minLength=1,maxLength=64),state=enum('invited','joining','present','left','removed')))
    out['Connection']=obj(dict(connection_id=ID,session_id=ID,participant_id=ID,membership_epoch=P,endpoint_id=ID,device_id=ID,generation=P,revision=N,state=enum('preparing','ready','active','held','retiring','retired','interrupted','unknown','failed'),replaces=nullable(ID)))
    out['ConnectionReady']=copy.deepcopy(out['Connection']);out['ConnectionReady']['properties'].update(state={'const':'ready'},revision=P)
    out['ConnectionRetired']=copy.deepcopy(out['Connection']);out['ConnectionRetired']['properties'].update(state={'const':'retired'},generation=dict(P,minimum=2),revision=P)
    out['Stream']=obj(dict(stream_id=ID,session_id=ID,participant_id=ID,connection_id=nullable(ID),medium=ID,state=ID,revision=N))
    out['ProviderCall']=obj(dict(id=ID,operation_id=ID,conversation_id=ID,session_id=ID,participant_id=ID,membership_epoch=P,endpoint_id=ID,state=enum('starting','active','ended','failed','unknown'),direction=enum('inbound','outbound'),remote_identity_verified={'const':False},revision=P,observed_at=nullable(N),closed_at=nullable(N),ended_reason=nullable(dict(S,maxLength=256,pattern=r'^[^\u0000-\u001f\u007f]*$')),problem=nullable(dict(ID,maxLength=128)),local_media={'const':False}))
    out['RecordAvailability']=union(obj(dict(state={'const':'pending'},session_id=INPUT)),obj(dict(state={'const':'ready'},session_id=INPUT,uuid=dict(type='string',format='uuid'),revision=P)),obj(dict(state={'const':'unavailable'},session_id=INPUT)))
    out['RecordAvailability']['description']='Authorized shared metadata only. Requires record_get, current Session source access and original frozen membership epoch or explicit current Session history share. Ready UUID/revision is a hint, not a source grant, storage URL, canonical digest or captured-audio claim. Fetch independently projects current sources. Pending has no ready handle; unavailable has no UUID or digest.'
    out['SnapshotItem']=union(*(tagged(kind,ref(name))for kind,name in [('record','RecordAvailability'),('provider_call','ProviderCall'),('conversation','ConversationState'),('participant','Member'),('session','SessionSnapshot'),('session_participant','Appearance'),('connection','Connection'),('stream','Stream'),('operation','Ledger_OperationRecord')]))
    out['SnapshotPage']=obj(dict(conversation_id=ID,snapshot_id=ID,revision=N,cursor=TOKEN,expires_at=N,items=array(ref('SnapshotItem'),500),next=nullable(TOKEN),has_more=B))
    out['SnapshotPage']['allOf']=[{'if':{'properties':{'has_more':{'const':False}}},'then':{'properties':{'next':{'type':'null'}}},'else':{'properties':{'next':TOKEN}}}]
    out['Source']=obj(dict(kind=dict(ID,maxLength=128),correlation_id=ID))
    out['Audience']=obj(dict(participant_id=ID,membership_epoch=P))
    out['Artifact']=obj(dict(id=ID,conversation_id=ID,owner_id=ID,name=S,media_type=S,size=N,received=N,sha256=dict(S,pattern='^[a-f0-9]{64}$'),state=ID,created_at=N,expires_at=nullable(N)))
    out['Call']=obj(dict(id=ID,conversation_id=ID,route_id=ID,destination=S,state=ID,provider_id=nullable(S),ended_reason=nullable(S),direction=enum('inbound','outbound'),end_requested=B))
    out['Receipt']=obj(dict(conversation_id=ID,message_id=ID,participant_id=ID,membership_epoch=P,channel=enum('native','provider'),delivery_id=ID,state=enum('queued','accepted','delivered','read','failed','unknown'),provenance=enum('journal','client_reported','provider_observation','provider_state','local_attempt'),observed_at=N))
    out['Receipt']['allOf']=[{'if':{'properties':{'state':{'const':'read'}}},'then':{'properties':{'channel':{'const':'native'},'provenance':{'const':'client_reported'}}}}]
    out['NativeDataWrite']=obj(dict(message_id=ID,source_connection_id=ID,target_connection_id=ID,participant_id=ID,membership_epoch=P,generation=P,disposition=enum('not_started','local_written','submission_unknown'),provenance=enum('local_writer','runtime_recovery'),observed_at=N))
    out['NativeDataWrite']['allOf']=[{'if':{'properties':{'provenance':{'const':'runtime_recovery'}}},'then':{'properties':{'disposition':{'const':'submission_unknown'}}}}]
    states=enum('accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown')
    out['JournalData']=union(*(tagged(kind,value)for kind,value in [('session_participant_joined',ref('SessionParticipantJoined')),('session_participant_left',ref('SessionParticipantLeft')),('session_started',ref('SessionStartedFact')),('session_ended',ref('SessionEndedFact')),('conversation_closed',ref('ConversationClosedFact')),('record',ref('RecordAvailability')),('provider_call',ref('ProviderCall')),('connection_prepared',ref('ConnectionPrepared')),('connection_ready',ref('ConnectionReady')),('connection_retired',ref('ConnectionRetired')),('session_control',ref('SessionSnapshot')),('message',ref('Msg_Message')),('session',ref('Session')),('artifact',ref('Artifact')),('call',ref('Call')),('receipt',ref('Receipt')),('native_data_write',ref('NativeDataWrite')),('membership',obj(dict(participant_id=nullable(ID),identity_id=nullable(ID),role=nullable(S),active=nullable(B),membership_epoch=nullable(P),removal_reason=nullable(dict(S,maxLength=512))))),('operation',obj(dict(operation_id=ID,record_id=ID,state=states))),('changed',obj(dict(resource_kind=nullable(ID),resource_id=nullable(ID))))]))
    out['JournalEvent']=obj(dict(event_id=ID,cursor=TOKEN,conversation_id=ID,session_id=nullable(ID),connection_id=nullable(ID),participant_id=nullable(ID),operation_id=nullable(ID),kind=dict(ID,maxLength=128),revision=N,source=ref('Source'),observed_at=N,audience=dict(array(ref('Audience'),100),minItems=1),data=ref('JournalData')))
    out['EventPage']=obj(dict(conversation_id=ID,events=array(ref('JournalEvent'),256),cursor=TOKEN,has_more=B,lease=nullable(TOKEN),expires_at=nullable(N),resync_required=B,snapshot=nullable(ref('SnapshotPage'))))
    out['EventPage']['allOf']=[{'if':{'properties':{'resync_required':{'const':True}}},'then':{'properties':{'snapshot':ref('SnapshotPage')}}},{'if':{'properties':{'lease':{'type':'null'}}},'then':{'properties':{'expires_at':{'type':'null'}}},'else':{'properties':{'expires_at':N}}}]
    out['ObservationCapabilities']=obj(dict(transcript=B,speech=B,actions=dict(array(INPUT,16),uniqueItems=True)))
    out['ObservationWork']=obj(dict(task_id=ID,effect_id=ID,question_id=ID))
    out['ObservationWork']['description']='Server-verified committed question exported to this host work context. Historical provenance only, not a task grant, semantic answer or logical action identity. Current task authority is independently required.'
    out['ObservationRoute']=obj(dict(work=nullable(ref('ObservationWork')),correlation_id=INPUT,participant_id=ID,connection_id=ID,generation=P,source=INPUT,capabilities=ref('ObservationCapabilities')))
    completeness=enum('partial','final','unavailable'); role=nullable(enum('user','assistant','system','other'))
    provider_ref=dict(type='string',minLength=1,maxLength=512,description='At most512 UTF-8 bytes, nonblank and control-free. Data only; never a Core identifier.')
    out['AgentGap']=obj(dict(first_sequence=P,last_sequence=P,observations=P))
    out['AgentGap']['description']='first <= last and observations=last-first+1; missing_before.last < local_sequence.'
    out['AgentContent']=union(obj(dict(kind={'const':'transcript'},text=nullable(S),speaker_role=role,completeness=completeness)),obj(dict(kind={'const':'speech'},state=nullable(provider_ref),speaker_role=role,turn=nullable(N),completeness=completeness)),obj(dict(kind={'const':'action'},name=nullable(provider_ref),call_id=nullable(provider_ref),phase=enum('requested','result-observed','unknown'),arguments={},result={},completeness=completeness)),obj(dict(kind={'const':'provider-status'},state=provider_ref)),obj(dict(kind={'const':'unavailable'},issue=enum('missing-fields','malformed-event','mismatched-session','unsupported-event','sequence-exhausted'))),obj(dict(kind={'const':'gap'},first_sequence=P,last_sequence=P,observations=P)))
    out['AgentObservation']=obj(dict(local_sequence=P,capture_complete={'const':False},missing_before=nullable(ref('AgentGap')),provider_session_id=nullable(provider_ref),provider_event_id=nullable(provider_ref),captured_at=N,content=ref('AgentContent')))
    out['AgentObservation']['description']='At most65536 encoded UTF-8 bytes. Provider roles and references are data, reported results are not execution receipts, final describes one fragment, and capture_complete is always false. A content gap ends at local_sequence.'
    out['ObservationDraft']=union(obj(dict(kind={'const':'agent'},observation=ref('AgentObservation'))),obj(dict(kind={'const':'transcript'},text=dict(S,minLength=1),completeness=enum('interim','final','incomplete'),turn_id=nullable(INPUT),fragment_id=nullable(INPUT))),obj(dict(kind={'const':'speech'},state=enum('started','stopped'),turn_id=nullable(INPUT))),obj(dict(kind={'const':'action-request'},action=INPUT,provider_tool_call_id=INPUT,arguments={'type':'object'},turn_id=nullable(INPUT))))
    out['ObservationDraft']['description']='Generic drafts are at most16KiB encoded; agent drafts are at most70KiB with a65536-byte bounded inner observation. Source capabilities gate transcript/speech/action content. Provider metadata and all arguments/results are data only; no automatic dispatch or completion authority.'
    out['Observation']=obj(dict(work=nullable(ref('ObservationWork')),observation_id=ID,conversation_id=ID,session_id=ID,participant_id=ID,membership_epoch=P,connection_id=ID,generation=P,source=ref('Source'),observed_at=N,data=ref('ObservationDraft')))
    out['ObservationPage']=obj(dict(conversation_id=ID,session_id=ID,observations=array(ref('Observation'),256),expires_at=N,routes=array(ref('ObservationRoute'),128),cursor=TOKEN,has_more=B,resync_required=B))
    out['SnapshotQuery']=obj(dict(page=nullable(TOKEN),limit=nullable(dict(P,maximum=500))),[])
    out['EventQuery']=obj(dict(after=nullable(TOKEN),limit=nullable(dict(P,maximum=500)),lease=nullable(TOKEN),lease_seconds=nullable(dict(P,maximum=30)),wait_ms=dict(N,maximum=30000)),[])
    out['ObservationQuery']=obj(dict(after=nullable(TOKEN),limit=nullable(dict(P,maximum=500)),wait_ms=dict(N,maximum=30000)),[])
    out['Operation']=union(obj(dict(op={'const':'conversation_snapshot'},conversation_id=INPUT,query=ref('SnapshotQuery'))),obj(dict(op={'const':'conversation_events'},conversation_id=INPUT,query=ref('EventQuery'))),obj(dict(op={'const':'session_observations'},conversation_id=INPUT,session_id=INPUT,query=ref('ObservationQuery'))))
    out['Outcome']=union(tagged('conversation_snapshot',ref('SnapshotPage')),tagged('conversation_events',ref('EventPage')),tagged('session_observations',ref('ObservationPage')))
    out['Error']=obj(dict(code=S,message=S))
    out['Response']=obj(dict(profile={'const':'connect-control/1'},result=union(obj({'Ok':ref('Outcome')}),obj({'Err':ref('Error')}))))
    return out

def document():
    return dict(openapi='3.1.0',info=dict(title='Connect authorized snapshots, events and observations',version='schema-88',description='Current typed control facade. Canonical physical conversation.subscribe uses the same Store pages through explicit per-peer composition (63 facts with optional typed Conversation closure, Session termination, receipt and record readiness hints before cursor events and final ACK, eight live Conversations, fixed at-most-30-second lease); native QUIC confirmation and managed admission emit typed ready/retired facts; explicit native audio and VP8 resource subscriptions now retain one effect and record actual outcomes; additive live upgrades preserve old audio while new Connections negotiate media; owned connection.end records actual termination without closing other Connections; explicit screen VP8 upgrades and subscriptions preserve audio; native data admission freezes one durable message and original audience; exact signed physical publication and current queued-message authorization are installed; attributed local queue receipts grant no physical-write or delivery claim; native writer observations reserve capacity before IO and distinguish local writes, refusals and uncertainty, with separate runtime recovery provenance; complete media-profile qualification remains open. Canonical GET route dispatch is G6-037; physical/provider normalization is G6-038/G6-043/G6-044. Fresh candidate state only.'),paths={'/v1/operations':{'post':dict(operationId='conversationEventControl',description='One service authority. Snapshots capture a consistent watermark; drain pages before acting. Current credentials/member epochs/recipient ceilings and selected source owners are checked under owned reader fences on pages and events; changing read sources refuse the page for bounded retry of the same cursor. Leases end within 30 seconds/credential expiry; event catch-up holds at most256 references and512KiB encoded. Retention/expiry/eviction/restart is explicit resync, never an empty proof of catch-up. Source handles, provider publishing and maintenance are not public operation verbs. Missing speech is not silence, and an action proposal does not execute an operation.',security=[{'bearerAuth':[]}],requestBody=dict(required=True,content={'application/json':{'schema':ref('Operation')}}),responses={'200':dict(description='Authorized page or typed business rejection in result.Ok/result.Err.',content={'application/json':{'schema':ref('Response')}}),'401':{'description':'Missing/invalid current bearer credential'},'413':{'description':'Request exceeds encoded budget'},'422':{'description':'Malformed or unknown closed typed operation/query fields'},'503':dict(description='Bounded HTTP capacity or current authority busy/unavailable; retry the same logical mutation ID after delivery owners settle.',content={'application/json':{'schema':ref('Response')}})})}},components=dict(securitySchemes={'bearerAuth':{'type':'http','scheme':'bearer'}},schemas=schemas()))
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--check',action='store_true');args=parser.parse_args()
    out=ROOT/'contracts/openapi-events.json';value=json.dumps(document(),indent=2)+'\n'
    if args.check:
        if not out.exists() or out.read_text()!=value:raise SystemExit('Event reference is stale; run scripts/generate-gate6-event-reference.py')
    else:out.write_text(value)
if __name__=='__main__':main()
