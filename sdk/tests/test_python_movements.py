"""Typed continuation/topology control over synthetic original SDK replies."""
from dataclasses import FrozenInstanceError, replace
from connect_sdk import (
    CandidateTarget, ConnectionId, ConsultationResult, ConsultInput,
    ConversationRevision, ConversationRole, EndpointId, EndpointTarget,
    HandoffInput, HandoffResult, MoveInput, MoveResult, MutationOptions, NonEmpty,
    OperationId, ParticipantId, RequestOptions, Scope, SdkError, Session, SessionRole,
    TopologyRevision,
)
from _python_sdk_fixture import PythonSdkFixture

OPTIONS=RequestOptions(1500)
REVISION=TopologyRevision(2)


def mutation(id='original'):
    return MutationOptions(OperationId(id),OPTIONS)


def consultation():
    return ConsultInput(ConnectionId('controller'),ConnectionId('retained'),NonEmpty(ParticipantId('private-peer')),'silence',REVISION)


def handoff(**values):
    return HandoffInput(**dict(via=ConnectionId('controller'),from_participant=ParticipantId('from'),
                              to=ParticipantId('to'),retain=ConnectionId('retained'),outgoing='leave',
                              role=SessionRole(),expected_topology_revision=REVISION,**values))


def consult_saved(record=True):
    context={'kind':'consultation','retained_connection_id':'retained'}
    if record:context['record_id']='record-original'
    return {'schema':'connect-operation-reference/1','operation_id':'original','command':'session.consult',
            'scope':{'conversation_id':'room','session_id':'session','connection_id':'controller'},'continuation':context}


class TypedPythonMovements(PythonSdkFixture):
    def session(self,client):
        return client.conversations.by_id('room').sessions.by_id('session')

    async def test_consultation_preserves_retained_original_record_and_typed_private_topology(self):
        client=self.client();operation=await self.session(client).consult(consultation(),mutation())
        self.assertEqual(operation.reference_json(),consult_saved())
        result=(await operation.wait(OPTIONS)).value
        self.assertIsInstance(result,ConsultationResult)
        self.assertEqual((result.session.id,result.retained[0].id,result.admitted[0].id,result.retired),('session','retained','private',()))
        self.assertIsNone(result.retained[0].receipt_stream_revision)
        with self.assertRaises(FrozenInstanceError):result.topology_revision=100
        self.assertFalse(hasattr(result.admitted[0],'send_audio'))

    async def test_consultation_return_and_recovery_keep_bound_record_and_distinct_logical_ids(self):
        client=self.client();operation=await self.session(client).consult(consultation(),mutation())
        returning=await operation.return_to_retained(TopologyRevision(3),mutation('return'))
        saved=returning.reference_json()
        self.assertEqual(saved['continuation'],{'kind':'consultation-return','retained_connection_id':'retained','consultation_record_id':'record-original'})
        self.assertIsInstance((await returning.wait(OPTIONS)).value,Session)
        self.assertEqual((await client.operations.recover(returning.reference).wait(OPTIONS)).value.id,'session')
        action=self.calls()[1]['request']['action']
        self.assertEqual((action['reference']['continuation']['record_id'],action['mutation']['operation_id']),('record-original','return'))
        with self.assertRaises(SdkError):await operation.return_to_retained(TopologyRevision(3),mutation())

    async def test_unbound_metadata_status_binds_record_before_export_and_fresh_recovery(self):
        client=self.client();operation=client.operations.restore(consult_saved(False),client.operation_kinds.session_consult)
        self.assertIsNone(client._helper._process)
        self.assertNotIn('record_id',operation.reference_json()['continuation'])
        await operation.status(OPTIONS)
        self.assertEqual(operation.reference_json()['continuation']['record_id'],'record-original')
        await client.close(OPTIONS)
        fresh=self.client();restored=fresh.operations.restore(operation.reference_json(),fresh.operation_kinds.session_consult)
        self.assertEqual((await restored.status(OPTIONS)).record_id,'record-original')
        self.assertEqual(len(self.calls()),1)

    async def test_handoff_scopes_dispositions_candidate_and_optional_capture_are_truthful(self):
        for role in [SessionRole(),ConversationRole(ConversationRevision(7))]:
            for outgoing in ['leave','observe']:
                client=self.client();input=replace(handoff(),role=role,outgoing=outgoing,candidate=ConnectionId('candidate'))
                operation=await self.session(client).handoff(input,mutation())
                value=(await operation.wait(OPTIONS)).value
                self.assertIsInstance(value,HandoffResult);self.assertEqual(value.admitted[0].id,'candidate')
                self.assertEqual(value.retired,('controller',) if outgoing=='leave' else ())
                self.assertEqual(value.conversation_revision,8 if role.scope=='conversation' else None)
                self.assertEqual((value.role_change.source_participant,value.role_change.target_participant),('from','to'))
                self.assertEqual(value.role_change.cleanup,'pending' if outgoing=='leave' else 'not_requested')
                await client.close(OPTIONS)
        client=self.client({'mode':'handoff-no-native'})
        self.assertIsNone((await (await self.session(client).handoff(handoff(),mutation())).wait(OPTIONS)).value.role_change)

    async def test_handoff_with_consultation_uses_exact_sdk_saved_context_without_resending_consult(self):
        client=self.client();session=self.session(client);consult=await session.consult(consultation(),mutation('consult'))
        await consult.wait(OPTIONS)
        operation=await session.handoff(replace(handoff(),consultation=consult),mutation('handoff'))
        self.assertEqual((await operation.wait(OPTIONS)).value.retained[0].id,'retained')
        requests=[c['request']['action'] for c in self.calls()]
        self.assertEqual(sum(a['type']=='session' and a['action']['type']=='consult' for a in requests),1)
        actual=next(a for a in requests if a['type']=='session_handoff')['input']['consultation']
        self.assertEqual((actual['operation_id'],actual['continuation']['record_id']),('consult','record-original'))

    async def test_endpoint_and_candidate_moves_preserve_original_target_binding_and_observation_only_handles(self):
        for target in [EndpointTarget(EndpointId('endpoint')),CandidateTarget(ConnectionId('candidate'))]:
            client=self.client();connection=self.session(client).connections.by_id('controller')
            operation=await connection.replace_device(MoveInput(ConnectionId('retained'),target,REVISION),mutation())
            result=(await operation.wait(OPTIONS)).value
            self.assertIsInstance(result,MoveResult)
            self.assertEqual((result.connection.id,result.binding.generation,result.binding.membership_epoch),('candidate' if isinstance(target,CandidateTarget) else 'admitted',3,2))
            self.assertEqual((result.retained[0].id,result.retired,result.binding.endpoint_id,result.binding.device_id),('retained',('controller',),'endpoint','device'))
            self.assertIsNone(result.connection.receipt_stream_revision)
            self.assertEqual((await client.operations.restore(operation.reference_json(),client.operation_kinds.connection_replace).status(OPTIONS)).record_id,'record-original')
            await client.close(OPTIONS)

    async def test_changed_handoff_topology_role_revision_and_cleanup_fence_original_owner(self):
        for mode in ['movement-duplicate','movement-local-receipt','handoff-candidate','handoff-role','handoff-revision','handoff-outgoing','handoff-cleanup']:
            client=self.client({'mode':mode});operation=await self.session(client).handoff(replace(handoff(),candidate=ConnectionId('candidate')),mutation())
            with self.assertRaises(SdkError) as observed:await operation.wait(OPTIONS)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','read-only'))
            await client.close(OPTIONS)

    async def test_changed_move_binding_scope_generation_target_and_retirement_fail_closed(self):
        for mode in ['movement-duplicate','movement-local-receipt','move-endpoint','move-generation','move-foreign-connection','move-unretired','move-candidate']:
            client=self.client({'mode':mode});target=CandidateTarget(ConnectionId('candidate')) if mode=='move-candidate' else EndpointTarget(EndpointId('endpoint'))
            operation=await self.session(client).connections.by_id('controller').replace_device(MoveInput(ConnectionId('retained'),target,REVISION),mutation())
            with self.assertRaises(SdkError) as observed:await operation.wait(OPTIONS)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','read-only'))
            await client.close(OPTIONS)

    async def test_changed_consultation_record_topology_or_return_context_is_not_rebound(self):
        for mode in ['consult-foreign-context','consult-foreign-retained','consult-duplicate','consult-foreign-record','record-change']:
            client=self.client({'mode':mode})
            if mode=='consult-foreign-context':
                with self.assertRaises(SdkError) as observed:await self.session(client).consult(consultation(),mutation())
                self.assertEqual(observed.exception.certainty,'uncertain')
            else:
                operation=await self.session(client).consult(consultation(),mutation())
                with self.assertRaises(SdkError) as observed:
                    if mode=='consult-foreign-record':await operation.return_to_retained(TopologyRevision(3),mutation('return'))
                    elif mode=='record-change':await operation.status(OPTIONS)
                    else:await operation.wait(OPTIONS)
                self.assertEqual(observed.exception.kind,'protocol')
            await client.close(OPTIONS)

    async def test_invalid_transition_inputs_and_ambiguous_targets_refuse_without_process_start(self):
        client=self.client();session=self.session(client);connection=session.connections.by_id('controller')
        badconsults=[replace(consultation(),retain=ConnectionId('controller')),replace(consultation(),hold='invented'),
                     replace(consultation(),with_participants=[]),replace(consultation(),expected_topology_revision=True)]
        for input in badconsults:
            with self.assertRaises(SdkError):await session.consult(input,mutation())
        badhandoffs=[replace(handoff(),to=ParticipantId('from')),replace(handoff(),candidate=ConnectionId('controller')),
                     replace(handoff(),retain=ConnectionId('controller')),replace(handoff(),role=SessionRole('invented')),
                     replace(handoff(),outgoing='invented'),replace(handoff(),consultation=object())]
        for input in badhandoffs:
            with self.assertRaises(SdkError):await session.handoff(input,mutation())
        for target in [{'endpoint':'endpoint','candidate':'candidate'},CandidateTarget(ConnectionId('controller')),CandidateTarget(ConnectionId('retained'))]:
            with self.assertRaises(SdkError):await connection.replace_device(MoveInput(ConnectionId('retained'),target,REVISION),mutation())
        self.assertIsNone(client._helper._process);self.assertEqual(self.calls(),[])

    async def test_invalid_restored_contexts_and_nonconsult_return_refuse_locally(self):
        client=self.client()
        values=[{**consult_saved(),'continuation':None},
                {**consult_saved(),'continuation':{'kind':'consultation','retained_connection_id':'controller'}},
                {**consult_saved(),'continuation':{**consult_saved()['continuation'],'extra':True}},
                {**consult_saved(),'scope':{'conversation_id':'room','session_id':'session'}},
                {**consult_saved(),'command':'session.consult.return'}]
        for value in values:
            with self.assertRaises(SdkError):client.operations.restore(value,client.operation_kinds.session_consult)
        ordinary=client.operations.restore({'schema':'connect-operation-reference/1','operation_id':'original','command':'session.invite','scope':{'conversation_id':'room','session_id':'session'}},client.operation_kinds.session_invite)
        with self.assertRaises(SdkError):await ordinary.return_to_retained(REVISION,mutation('return'))
        self.assertIsNone(client._helper._process)

    async def test_current_refusal_preserves_transition_scope_original_id_and_unrelated_control(self):
        client=self.client({'mode':'refuse','kind':'authentication','certainty':'refused'});session=self.session(client)
        with self.assertRaises(SdkError) as observed:await session.handoff(handoff(),mutation())
        self.assertEqual((observed.exception.kind,observed.exception.certainty,observed.exception.operation.scope),('authentication','refused',Scope('room','session','controller')))
        self.assertEqual((await session.refresh(OPTIONS)).id,'session')
        self.assertEqual(len(self.calls()),2)
