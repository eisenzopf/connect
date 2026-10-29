"""Typed public Session control over the original synthetic SDK child."""
import asyncio
from dataclasses import FrozenInstanceError
from connect_sdk import (
    CodecRenegotiate, MediumUpgrade, NonEmpty, OperationId, MutationOptions,
    ParticipantId, ParticipantLeave, RequestOptions, RoleChange, SdkError,
    Session, SessionDecision, SessionId, TopologyRevision,
)
from _python_sdk_fixture import PythonSdkFixture

OPTIONS = RequestOptions(1500)
REVISION = TopologyRevision(2)
CAPS = {'audio':['g.711-mu']}


def mutation(id='original'):
    return MutationOptions(OperationId(id),OPTIONS)


def reference(command='session.invite',scope=None):
    return {'schema':'connect-operation-reference/1','operation_id':'original','command':command,
            'scope':{'conversation_id':'room','session_id':'session'} if scope is None else scope}


class TypedPythonSessions(PythonSdkFixture):
    async def test_solo_and_existing_session_invites_keep_original_scope_and_current_snapshot(self):
        client=self.client();room=client.conversations.by_id('room')
        operation=await room.sessions.invite(SessionId('session'),[],'voice','synthetic',CAPS,mutation())
        self.assertEqual(operation.reference_json(),reference())
        session=(await operation.wait(OPTIONS)).value
        self.assertIsInstance(session,Session);self.assertEqual((session.id,session.conversation_id),('session','room'))
        current=await session.refresh(OPTIONS)
        self.assertEqual((current.revision,current.topology_revision),(1,2))
        self.assertEqual((current.media,current.codec_preferences),(('audio',),()))
        self.assertEqual(current.capabilities_offer,{'audio':('g.711-mu',)})
        with self.assertRaises(TypeError):current.capabilities_offer['audio']=[]
        with self.assertRaises(FrozenInstanceError):session.id='foreign'
        extended=await session.invite_participants(NonEmpty(ParticipantId('peer')),'voice','join',CAPS,REVISION,mutation('extend'))
        self.assertEqual((await extended.wait(OPTIONS)).value.id,'session')
        action=self.calls()[-2]['request']['action']['action']
        self.assertEqual((action['to'],action['expected_revision']),(['peer'],2))
        restored=client.operations.restore(operation.reference_json(),client.operation_kinds.session_invite)
        self.assertEqual((await restored.status(OPTIONS)).record_id,'record-original')

    async def test_accept_and_all_session_update_variants_delegate_topology_preconditions(self):
        client=self.client();session=client.conversations.by_id('room').sessions.by_id('session')
        self.assertEqual((await (await session.accept(CAPS,REVISION,mutation('accept'))).wait(OPTIONS)).value.id,'session')
        inputs=[ParticipantLeave(ParticipantId('peer')),RoleChange(ParticipantId('peer'),'observer'),
                MediumUpgrade(NonEmpty('audio',['video'])),CodecRenegotiate(NonEmpty('g.711-mu',['vp8']))]
        for i,input in enumerate(inputs):
            operation=await session.update(input,REVISION,mutation('update-'+str(i)))
            self.assertEqual((await operation.wait(OPTIONS)).value.id,'session')
        actions=[c['request']['action']['action'] for c in self.calls() if c['request']['action']['type']=='session']
        self.assertEqual([a['expected_revision'] for a in actions],[2]*5)
        self.assertEqual([a['update']['action'] for a in actions[1:]],['participant-leave','role-change','medium-upgrade','codec-renegotiate'])
        self.assertEqual(actions[3]['update']['media'],['audio','video'])

    async def test_invitation_reject_cancel_and_session_end_have_distinct_typed_results(self):
        client=self.client();session=client.conversations.by_id('room').sessions.by_id('session')
        reject=await session.reject(486,'unavailable',REVISION,mutation('reject'))
        rejected=(await reject.wait(OPTIONS)).value
        self.assertIsInstance(rejected,SessionDecision);self.assertEqual((rejected.state,rejected.session.id,rejected.topology_revision),('rejected','session',2))
        cancel=await session.cancel_invitation(487,'cancelled',REVISION,mutation('cancel'))
        self.assertEqual((await cancel.wait(OPTIONS)).value.state,'cancelled')
        end=await session.end(1000,'done',REVISION,mutation('end'))
        self.assertIsInstance((await end.wait(OPTIONS)).value,Session)
        recovered=client.operations.restore(reject.reference_json(),client.operation_kinds.session_reject)
        self.assertEqual((await recovered.wait(OPTIONS)).value.state,'rejected')
        decision=await end.cancel(mutation('decision'))
        self.assertEqual((decision.reference.scope.conversation_id,decision.reference.scope.session_id,decision.reference.scope.connection_id),('room','session',None))

    async def test_session_restore_rejects_wrong_kind_scope_and_context_before_process_start(self):
        client=self.client()
        for value in [reference(scope={}),reference(scope={'session_id':'session'}),
                      reference(scope={'conversation_id':'room'}),reference(scope={'conversation_id':'room','session_id':'session','connection_id':'connection'}),
                      dict(reference(),continuation={}),reference('session.reject')]:
            with self.assertRaises(SdkError):client.operations.restore(value,client.operation_kinds.session_invite)
        self.assertIsNone(client._helper._process);self.assertEqual(self.calls(),[])

    async def test_session_inputs_fail_locally_without_silent_revision_or_empty_target_coercion(self):
        client=self.client();sessions=client.conversations.by_id('room').sessions;session=sessions.by_id('session')
        invalid=[lambda:sessions.invite(SessionId('bad!'),[],'voice','intent',CAPS,mutation()),
                 lambda:sessions.invite(SessionId('session'),[],'invented','intent',CAPS,mutation()),
                 lambda:session.invite_participants([], 'voice','intent',CAPS,REVISION,mutation()),
                 lambda:session.accept(CAPS,True,mutation()),lambda:session.reject(True,'reason',REVISION,mutation()),
                 lambda:session.cancel_invitation(65536,'reason',REVISION,mutation()),
                 lambda:session.end(1000,[],REVISION,mutation()),lambda:session.update({},REVISION,mutation()),
                 lambda:session.update(ParticipantLeave(ParticipantId('bad!')),REVISION,mutation()),
                 lambda:session.update(RoleChange(ParticipantId('peer'),['observer']),REVISION,mutation()),
                 lambda:session.update(MediumUpgrade(NonEmpty('invented')),REVISION,mutation()),
                 lambda:session.update(CodecRenegotiate(NonEmpty('')),REVISION,mutation())]
        for call in invalid:
            with self.assertRaises(SdkError) as observed:await call()
            self.assertEqual(observed.exception.kind,'validation')
        self.assertIsNone(client._helper._process);self.assertEqual(self.calls(),[])

    async def test_queued_session_intent_and_capabilities_are_captured_by_original_owner(self):
        client=self.client({'delayMs':100});sessions=client.conversations.by_id('room').sessions
        first=asyncio.create_task(client.capabilities.refresh(OPTIONS));await self.sent()
        caps={'audio':['g.711-mu']};targets=[ParticipantId('peer')]
        pending=asyncio.create_task(sessions.invite(SessionId('session'),targets,'voice','original',caps,mutation()))
        await asyncio.sleep(0);caps['audio'][0]='changed';targets[0]='foreign'
        operation=await pending;await first
        action=self.calls()[1]['request']['action']['action']
        self.assertEqual((action['capabilities'],action['to']),({'audio':['g.711-mu']},['peer']))
        self.assertEqual(operation.reference.operation_id,'original')

    async def test_foreign_session_results_and_coercible_snapshots_fence_original_control(self):
        for mode,method in [('wrong-session','wait'),('wrong-session-decision','decision'),('state-array','refresh')]:
            client=self.client({'mode':mode});session=client.conversations.by_id('room').sessions.by_id('session')
            operation=None
            if method=='wait':operation=await session.accept(CAPS,REVISION,mutation())
            if method=='decision':operation=await session.reject(486,'reason',REVISION,mutation())
            with self.assertRaises(SdkError) as observed:
                if method=='refresh':await session.refresh(OPTIONS)
                else:await operation.wait(OPTIONS)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','read-only'))
            with self.assertRaises(SdkError) as later:await session.refresh(OPTIONS)
            self.assertEqual(later.exception.kind,'configuration')
            await client.close(OPTIONS)

    async def test_current_application_conflict_preserves_original_session_intent_and_control(self):
        client=self.client({'mode':'refuse','kind':'conflict','certainty':'refused'})
        session=client.conversations.by_id('room').sessions.by_id('session')
        with self.assertRaises(SdkError) as observed:await session.accept(CAPS,REVISION,mutation())
        error=observed.exception
        self.assertEqual((error.kind,error.certainty,error.operation.operation_id,error.operation.scope.session_id),('conflict','refused','original','session'))
        self.assertEqual((await session.refresh(OPTIONS)).id,'session')
        self.assertEqual(len(self.calls()),2)
