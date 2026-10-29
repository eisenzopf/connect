"""Public typed Python SDK through the synthetic original-owner fixture."""
import asyncio
from dataclasses import FrozenInstanceError
from _python_sdk_fixture import PythonSdkFixture

from connect_sdk import (Client, Conversation, ConversationRevision, IdentityId,
                         MessageId, MutationOptions, NonEmpty, OperationId,
                         ParticipantId, RequestOptions, SdkError, SdkCancelledError)

OPTIONS = RequestOptions(1500)


def mutation(id='original'):
    return MutationOptions(OperationId(id), OPTIONS)


def saved(command='message.send', scope=None, id='original'):
    return dict(schema='connect-operation-reference/1',operation_id=id,command=command,
                scope=dict(conversation_id='room') if scope is None else scope)


class TypedPythonSdk(PythonSdkFixture):
    async def test_mutation_returns_operation_and_only_explicit_wait_returns_typed_handle(self):
        client=self.client()
        self.assertEqual(self.calls(),[])
        self.assertFalse(hasattr(client,'request'));self.assertFalse(hasattr(client,'provision'))
        create=await client.conversations.create([IdentityId('peer')],'persistent',mutation('create'))
        self.assertEqual(create.reference.command,'conversation.create')
        self.assertEqual(len(self.calls()),1)
        self.assertEqual((await create.wait_until_ready(OPTIONS)).state,'completed')
        completed=await create.wait(OPTIONS)
        self.assertIsInstance(completed.value,Conversation)
        self.assertEqual(completed.value.id,'room')
        self.assertEqual(completed.snapshot.record_id,'record-original')
        self.assertEqual((await completed.value.refresh(OPTIONS)).participants,())
        for owner,attribute in [(client,'operation_kinds'),(create,'reference'),(create.reference,'_key'),(completed,'value'),(completed.value,'id')]:
            with self.assertRaises(FrozenInstanceError):setattr(owner,attribute,None)

    async def test_saved_reference_recovery_is_local_typed_immutable_and_status_first(self):
        client=self.client();value=saved();operation=client.operations.restore(value,client.operation_kinds.message_send)
        value['operation_id']='changed';value['scope']['conversation_id']='foreign'
        copy=operation.reference_json();copy['operation_id']='changed'
        self.assertEqual(operation.reference_json(),saved())
        recovered=client.operations.recover(operation.reference)
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)
        self.assertEqual((await recovered.status(OPTIONS)).key.operation_id,'original')
        self.assertEqual((await recovered.wait(OPTIONS)).value.message_id,'message')
        self.assertEqual(self.calls()[0]['request']['action']['action']['type'],'status')
        await client.close(OPTIONS)
        fresh=self.client();restored=fresh.operations.restore(operation.reference_json(),fresh.operation_kinds.message_send)
        self.assertEqual((await restored.status(OPTIONS)).record_id,'record-original')
        self.assertEqual(len(self.calls()),1)

    async def test_forged_wrong_kind_scope_or_private_continuation_refuses_before_process_start(self):
        client=self.client()
        for value in [None,{},saved(command='conversation.create'),saved(scope={}),
                      saved(scope={'conversation_id':'room','session_id':'session'}),
                      saved(scope={'conversation_id':'room','foreign':'private'}),
                      saved(scope={'conversation_id':None}),dict(saved(),continuation={}),
                      dict(saved(),operation_id='bad!'),dict(saved(),private='x'*5000)]:
            with self.subTest(value=type(value).__name__):
                with self.assertRaises(SdkError) as observed:client.operations.restore(value,client.operation_kinds.message_send)
                self.assertEqual(observed.exception.kind,'validation')
        with self.assertRaises(SdkError):client.operations.restore(saved(),object())
        class EqualButForeign:
            command='message.send'
            def __eq__(self,other):return True
        with self.assertRaises(SdkError):client.operations.restore(saved(),EqualButForeign())
        with self.assertRaises(SdkError):client.operations.recover(object())
        with self.assertRaises(SdkError):client.operations.restore(saved('conversation.create',{'conversation_id':'room'}),client.operation_kinds.conversation_create)
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)

    async def test_input_capture_keeps_original_mutation_and_extensions_while_queued(self):
        client=self.client({'delayMs':100});room=client.conversations.by_id('room')
        extension={'vendor':{'private':'original'}};options=MutationOptions(OperationId('original'),OPTIONS,extension)
        extension['vendor']['private']='changed'
        rest=[ParticipantId('second')];to=NonEmpty(ParticipantId('first'),rest);rest[0]='changed'
        first=asyncio.create_task(client.capabilities.refresh(OPTIONS));await self.sent()
        pending=asyncio.create_task(room.send(MessageId('message'),to,'original',options));await asyncio.sleep(0)
        operation=await pending;await first
        submitted=self.calls()[1]['request']['action']
        self.assertEqual(submitted['to'],['first','second'])
        self.assertEqual(submitted['mutation']['operation_id'],'original')
        self.assertEqual(submitted['mutation']['extensions'],{'vendor':{'private':'original'}})
        self.assertEqual((await operation.wait(OPTIONS)).value.operation.operation_id,'original')
        with self.assertRaises(TypeError):options.extensions['vendor']['private']='changed'

    async def test_cancel_and_reconcile_keep_distinct_decision_identity_and_original_target(self):
        client=self.client();operation=client.operations.restore(saved(),client.operation_kinds.message_send)
        cancel=await operation.cancel(mutation('decision'))
        self.assertEqual(cancel.reference.command,'operation.cancel')
        result=await cancel.wait(OPTIONS)
        self.assertEqual(result.value.target,'original');self.assertEqual(result.value.decision.operation_id,'decision')
        self.assertIsNone(result.value.evidence_id)
        reconcile=await operation.reconcile('completed','evidence',mutation('reconcile'))
        self.assertEqual((await reconcile.wait(OPTIONS)).value.evidence_id,'evidence')
        self.assertEqual((await operation.status(OPTIONS)).state,'completed')
        self.assertEqual(len([c for c in self.calls() if c['request']['action']['type']=='conversation_send']),0)
        with self.assertRaises(SdkError):await operation.cancel(mutation())
        with self.assertRaises(SdkError):await operation.reconcile('completed','bad!',mutation('other'))

    async def test_pending_interrupted_unknown_states_are_observations_without_replay(self):
        for state in ['accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown']:
            client=self.client({'state':state});operation=client.operations.restore(saved(),client.operation_kinds.message_send)
            self.assertEqual((await operation.status(OPTIONS)).state,state)
            self.assertEqual(len(self.calls()),1)
            self.assertEqual(self.calls()[0]['request']['action']['action'],{'type':'status'})
            await client.close(OPTIONS)

    async def test_decision_completion_cannot_substitute_a_foreign_target_after_local_recovery(self):
        for recover in [False,True]:
            client=self.client({'mode':'decision-target'})
            original=client.operations.restore(saved(),client.operation_kinds.message_send)
            decision=await original.cancel(mutation('decision'))
            if recover:decision=client.operations.recover(decision.reference)
            with self.assertRaises(SdkError) as observed:await decision.wait(OPTIONS)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','read-only'))
            self.assertEqual(observed.exception.operation.operation_id,'decision')
            await client.close(OPTIONS)

    async def test_invalid_completion_ready_scope_result_or_record_fences_original_owner(self):
        for config,wait in [({'mode':'wrong-scope'},'status'),({'mode':'wrong-result'},'wait'),
                            ({'mode':'state-array'},'status'),({'state':'interrupted'},'wait'),
                            ({'state':'accepted'},'wait_until_ready'),({'mode':'record-change'},'record')]:
            client=self.client(config);operation=client.operations.restore(saved(),client.operation_kinds.message_send)
            if wait=='record':await operation.status(OPTIONS);wait='status'
            with self.assertRaises(SdkError) as observed:await getattr(operation,wait)(OPTIONS)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','read-only'))
            self.assertEqual(observed.exception.operation.operation_id,'original')
            with self.assertRaises(SdkError) as later:await operation.status(OPTIONS)
            self.assertEqual(later.exception.kind,'configuration')
            await client.close(OPTIONS)

    async def test_foreign_submission_reference_preserves_uncertainty_without_replay(self):
        client=self.client({'mode':'wrong-reference'})
        with self.assertRaises(SdkError) as observed:await client.conversations.by_id('room').send(MessageId('message'),NonEmpty(ParticipantId('participant')),'private text',mutation())
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','uncertain'))
        self.assertEqual(observed.exception.operation.operation_id,'original')
        self.assertEqual(len(self.calls()),1)
        self.assertNotIn('private text',repr(observed.exception))

    async def test_participant_controls_return_scoped_typed_handles_and_truthful_revisions(self):
        client=self.client();room=client.conversations.by_id('room')
        add=await room.participants.add(IdentityId('identity'),'human','participant',ConversationRevision(7),mutation('add'))
        participant=(await add.wait(OPTIONS)).value
        self.assertEqual((participant.id,participant.conversation_id),('participant','room'))
        snapshot=await participant.refresh(OPTIONS);self.assertEqual(snapshot.membership_epoch,1);self.assertTrue(snapshot.active)
        update=await participant.update('observer',ConversationRevision(8),mutation('update'))
        self.assertEqual((await update.wait(OPTIONS)).value.id,'participant')
        remove=await participant.remove(ConversationRevision(9),mutation('remove'))
        self.assertEqual((await remove.wait(OPTIONS)).value.id,'participant')
        submissions=[c['request']['action'] for c in self.calls() if c['request']['action']['type'].startswith('participant_') and c['request']['action']['type']!='participant_refresh']
        self.assertEqual([s['expected_revision'] for s in submissions],[7,8,9])
        for command in [add,update,remove]:self.assertEqual(command.reference.scope.conversation_id,'room')

    async def test_foreign_participant_result_and_coercible_kind_fail_closed(self):
        for mode in ['wrong-participant','participant-kind-array']:
            client=self.client({'mode':mode});participant=client.conversations.by_id('room').participants.by_id('participant')
            with self.assertRaises(SdkError) as observed:await participant.refresh(OPTIONS)
            self.assertEqual(observed.exception.kind,'protocol')
            await client.close(OPTIONS)

    async def test_capabilities_are_python_interface_support_and_unsupported_stays_typed(self):
        client=self.client();value=await client.capabilities.refresh(OPTIONS)
        self.assertEqual((value.runtime_role,value.selected_transport,value.features),('python','quic',frozenset(['original-id-operations','message-receipts','durable-events','incoming-invitations'])))
        await client.capabilities.require('original-id-operations',OPTIONS)
        for feature in ['native-programmatic-media','raw-canonical']:
            with self.assertRaises(SdkError) as observed:await client.capabilities.require(feature,OPTIONS)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('unsupported','read-only'))
        self.assertEqual((await client.conversations.by_id('room').refresh(OPTIONS)).id,'room')
        for mode in ['caps-foreign','caps-transport','caps-duplicate']:
            broken=self.client({'mode':mode})
            with self.assertRaises(SdkError) as observed:await broken.capabilities.refresh(OPTIONS)
            self.assertEqual(observed.exception.kind,'protocol')
            await broken.close(OPTIONS)

    async def test_receipts_are_immutable_scoped_per_recipient_observations(self):
        client=self.client();room=client.conversations.by_id('room')
        values=await room.receipts(MessageId('message'),OPTIONS)
        self.assertIsInstance(values,tuple);self.assertEqual(values[0].provenance,'client_reported')
        with self.assertRaises(FrozenInstanceError):values[0].state='read'
        for mode in ['receipt-cid','receipt-mid','receipt-duplicate','receipt-state-array','receipt-provenance','receipt-epoch','receipt-time','receipt-channel','receipt-overflow']:
            broken=self.client({'mode':mode})
            with self.assertRaises(SdkError) as observed:await broken.conversations.by_id('room').receipts(MessageId('message'),OPTIONS)
            self.assertEqual(observed.exception.kind,'protocol')
            await broken.close(OPTIONS)

    async def test_local_input_validation_never_starts_a_process(self):
        client=self.client();room=client.conversations.by_id('room')
        for make in [lambda:MutationOptions(OperationId('bad!'),OPTIONS),lambda:MutationOptions(OperationId('original'),OPTIONS,[]),
                     lambda:NonEmpty(ParticipantId('one'),'invalid-rest'),lambda:client.conversations.by_id('bad!'),
                     lambda:room.participants.by_id('bad!')]:
            with self.assertRaises(SdkError):make()
        for call in [lambda:room.send(MessageId('message'),NonEmpty(ParticipantId('same'),[ParticipantId('same')]),'text',mutation()),
                     lambda:room.participants.add(IdentityId('identity'),['human'],'participant',ConversationRevision(1),mutation()),
                     lambda:room.participants.by_id('participant').remove(True,mutation()),
                     lambda:client.conversations.create([],['persistent'],mutation())]:
            with self.assertRaises(SdkError):await call()
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)

    async def test_async_context_closes_original_helper_and_cancelled_mutation_retains_key(self):
        client=self.client({'delayMs':250})
        async with client:
            pending=asyncio.create_task(client.conversations.by_id('room').send(MessageId('message'),NonEmpty(ParticipantId('participant')),'private',mutation()))
            await self.sent();pending.cancel('private cancellation text')
            with self.assertRaises(SdkCancelledError) as observed:await pending
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('cancelled','uncertain'))
            self.assertEqual(observed.exception.operation.operation_id,'original')
            self.assertNotIn('private',str(observed.exception));self.assertTrue(pending.cancelled())
        self.assertIsNotNone(client._helper._process.returncode)

    async def test_client_config_is_fixed_and_diagnostics_omit_private_intent(self):
        client=self.client();room=client.conversations.by_id('room')
        operation=await room.send(MessageId('message'),NonEmpty(ParticipantId('participant')),'private-body',mutation())
        self.clients[-1][1].joinpath('config.json').write_text('not JSON after original owner starts')
        self.assertEqual((await operation.status(OPTIONS)).record_id,'record-original')
        for value in [client,operation,operation.reference,await operation.status(OPTIONS),await operation.wait(OPTIONS)]:
            self.assertNotIn('private-body',repr(value));self.assertNotIn(str(self.directory),repr(value))
