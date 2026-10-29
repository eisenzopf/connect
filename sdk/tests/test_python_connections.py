"""Native control and historical stream observations, never decoded-media proof."""
from dataclasses import FrozenInstanceError
from connect_sdk import (
    CodecOutcome, Connection, ConnectionCodecRenegotiate, ConnectionId,
    GateOutcome, HoldConnection, Mute, MutationOptions, NonEmpty, OfferedStream,
    OperationId, ParticipantId, ParticipantStreams, Quality, QualityOutcome,
    RequestOptions, ResumeConnection, SdkError, StreamId, StreamRevision, StreamTarget,
)
from _python_sdk_fixture import PythonSdkFixture

OPTIONS=RequestOptions(1500)
REVISION=StreamRevision(1)


def mutation(id='original'):
    return MutationOptions(OperationId(id),OPTIONS)


def reference(command='connection.offer',scope=None):
    return {'schema':'connect-operation-reference/1','operation_id':'original','command':command,
            'scope':{'conversation_id':'room','session_id':'session','connection_id':'connection'} if scope is None else scope}


class TypedPythonConnections(PythonSdkFixture):
    def connection(self,client):
        return client.conversations.by_id('room').sessions.by_id('session').connections.by_id('connection')

    async def test_offer_returns_original_connection_and_receipt_capture_does_not_grant_python_media(self):
        client=self.client();connections=client.conversations.by_id('room').sessions.by_id('session').connections
        offered=await connections.offer(ConnectionId('connection'),NonEmpty(OfferedStream(StreamId('audio'),'audio','sendrecv',NonEmpty('g.711-mu'))),mutation())
        self.assertEqual(offered.reference_json(),reference())
        connection=(await offered.wait(OPTIONS)).value
        self.assertIsInstance(connection,Connection);self.assertEqual(connection.receipt_stream_revision,1)
        self.assertIsNone(connections.by_id('connection').receipt_stream_revision)
        current=await connection.refresh(OPTIONS)
        self.assertIsNone(current.stream_revision);self.assertEqual((current.generation,current.state),(1,'active'))
        for name in ['send_audio','receive_audio','publish','media']:
            self.assertFalse(hasattr(connection,name))
        self.assertEqual((await connection.streams.by_id('audio').refresh(OPTIONS)).ownership,'observation')
        with self.assertRaises(FrozenInstanceError):connection.receipt_stream_revision=100

    async def test_quality_gates_and_codecs_return_distinct_immutable_typed_receipts(self):
        client=self.client();connection=self.connection(client)
        inputs=[Quality(250),Mute('mute',NonEmpty(StreamId('audio'))),Mute('unmute',NonEmpty(StreamId('audio'))),
                HoldConnection('inactive'),ResumeConnection(),ConnectionCodecRenegotiate(NonEmpty('g.711-a'))]
        values=[]
        for index,input in enumerate(inputs):
            result=(await (await connection.update(input,REVISION,mutation('update-'+str(index)))).wait(OPTIONS)).value
            self.assertEqual((result.connection.id,result.connection_generation,result.stream_revision),('connection',1,2))
            values.append(result.outcome)
        self.assertIsInstance(values[0],QualityOutcome);self.assertEqual(values[0].interval_ms,250)
        self.assertIsInstance(values[1],GateOutcome);self.assertTrue(values[1].streams[0].source_muted)
        self.assertEqual(values[3].streams[0].state,'paused');self.assertIsInstance(values[5],CodecOutcome)
        self.assertTrue(values[5].resources_changed)
        with self.assertRaises(TypeError):values[5].streams[0].parameters.codec['name']='changed'
        updates=[c['request']['action']['action'] for c in self.calls() if c['request']['action']['type']=='connection']
        self.assertEqual([v['expected_revision'] for v in updates],[1]*6)
        self.assertIsNone(updates[3]['update']['streams'])

    async def test_stream_selection_distinguishes_captured_empty_and_unavailable_bindings(self):
        for capture,expected in [('unavailable',None),('empty',()),('present','captured')]:
            client=self.client({'capture':capture});connection=self.connection(client)
            selectors=NonEmpty(StreamTarget(StreamId('audio')),[ParticipantStreams(ParticipantId('peer'),NonEmpty('audio'))])
            operation=await connection.streams.subscribe(selectors,REVISION,mutation('subscribe'))
            value=(await operation.wait(OPTIONS)).value
            self.assertEqual((value.connection.id,value.stream_revision,value.resources_changed),('connection',2,True))
            if expected=='captured':self.assertEqual(value.bindings[0].parameters.stream_local_id,1)
            else:self.assertEqual(value.bindings,expected)
            unsub=await connection.streams.unsubscribe(NonEmpty(StreamId('audio')),REVISION,mutation('unsubscribe'))
            self.assertEqual((await unsub.wait(OPTIONS)).value.connection.id,'connection')
            await client.close(OPTIONS)

    async def test_stream_clock_keeps_exact_u64_and_absent_format_direction_are_truthful(self):
        client=self.client({'clock':True});stream=self.connection(client).streams.by_id('audio')
        value=await stream.refresh(OPTIONS)
        self.assertEqual((stream.id,stream.connection_id,stream.session_id,stream.conversation_id),('audio','connection','session','room'))
        self.assertEqual(value.clock.units_per_second,18446744073709551615)
        self.assertEqual((value.format,value.direction,value.resource_revision,value.ownership),(None,None,7,'observation'))
        self.assertFalse(value.clock.synchronized)
        with self.assertRaises(FrozenInstanceError):value.clock.generation=100
        with self.assertRaises(FrozenInstanceError):stream.id='foreign'

    async def test_foreign_connection_generation_and_corrupt_binding_results_fail_closed(self):
        for mode,kind in [('wrong-connection','update'),('binding-generation','subscribe'),('binding-local-handle','subscribe'),
                          ('missing-bindings','subscribe'),('binding-state-array','gates')]:
            client=self.client({'mode':mode});connection=self.connection(client)
            if kind=='subscribe':operation=await connection.streams.subscribe(NonEmpty(StreamTarget(StreamId('audio'))),REVISION,mutation())
            else:operation=await connection.update(Quality(250) if kind=='update' else Mute('mute',NonEmpty(StreamId('audio'))),REVISION,mutation())
            with self.assertRaises(SdkError) as observed:await operation.wait(OPTIONS)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','read-only'))
            self.assertEqual(observed.exception.operation.scope.connection_id,'connection')
            with self.assertRaises(SdkError) as later:await connection.refresh(OPTIONS)
            self.assertEqual(later.exception.kind,'configuration')
            await client.close(OPTIONS)

    async def test_source_clock_overflow_refuses_without_fabricating_a_media_clock(self):
        client=self.client({'clock':True,'mode':'bad-clock'})
        with self.assertRaises(SdkError) as observed:await self.connection(client).streams.by_id('audio').refresh(OPTIONS)
        self.assertEqual(observed.exception.kind,'protocol')

    async def test_native_restore_and_recovery_keep_all_three_scopes_without_io(self):
        client=self.client();value=reference();operation=client.operations.restore(value,client.operation_kinds.connection_offer)
        value['scope']['connection_id']='foreign'
        recovered=client.operations.recover(operation.reference)
        self.assertIsNone(client._helper._process)
        self.assertEqual((await recovered.wait(OPTIONS)).value.id,'connection')
        await client.close(OPTIONS)
        fresh=self.client();saved=operation.reference_json()
        restored=fresh.operations.restore(saved,fresh.operation_kinds.connection_offer)
        self.assertEqual((await restored.status(OPTIONS)).record_id,'record-original')
        self.assertEqual(len(self.calls()),1)

    async def test_native_restore_rejects_incomplete_foreign_family_and_extended_metadata(self):
        client=self.client()
        for value in [reference(scope={}),reference(scope={'conversation_id':'room'}),reference(scope={'conversation_id':'room','session_id':'session'}),
                      reference('connection.end'),dict(reference(),continuation={}),reference(scope={'conversation_id':'room','session_id':'session','connection_id':None})]:
            with self.assertRaises(SdkError):client.operations.restore(value,client.operation_kinds.connection_offer)
        self.assertIsNone(client._helper._process)

    async def test_native_input_validation_refuses_coercion_and_empty_selection_before_start(self):
        client=self.client();connection=self.connection(client);connections=client.conversations.by_id('room').sessions.by_id('session').connections
        invalid=[lambda:connections.offer(ConnectionId('connection'),[],mutation()),
                 lambda:connections.offer(ConnectionId('connection'),NonEmpty(OfferedStream(StreamId('audio'),'audio','invented',NonEmpty('g.711-mu'))),mutation()),
                 lambda:connection.update(Quality(True),REVISION,mutation()),lambda:connection.update(Quality(2**32),REVISION,mutation()),
                 lambda:connection.update(Mute('invented',NonEmpty(StreamId('audio'))),REVISION,mutation()),
                 lambda:connection.update(HoldConnection('sendrecv'),REVISION,mutation()),
                 lambda:connection.update(ConnectionCodecRenegotiate(NonEmpty('')),REVISION,mutation()),
                 lambda:connection.end(True,'reason',None,mutation()),lambda:connection.end(1000,'reason',True,mutation()),
                 lambda:connection.streams.subscribe(NonEmpty({'stream':'audio','participant':'peer'}),REVISION,mutation()),
                 lambda:connection.streams.subscribe(NonEmpty(ParticipantStreams(ParticipantId('peer'),NonEmpty('invented'))),REVISION,mutation()),
                 lambda:connection.streams.unsubscribe([],REVISION,mutation())]
        for call in invalid:
            with self.assertRaises(SdkError) as observed:await call()
            self.assertEqual(observed.exception.kind,'validation')
        self.assertIsNone(client._helper._process)

    async def test_native_end_and_decision_recovery_keep_target_connection_separate(self):
        client=self.client();connection=self.connection(client)
        operation=await connection.end(1000,'done',None,mutation())
        self.assertIsNone((await operation.wait(OPTIONS)).value.receipt_stream_revision)
        self.assertIsNone(self.calls()[0]['request']['action']['action']['expected_revision'])
        decision=await operation.cancel(mutation('decision'))
        self.assertEqual(decision.reference.scope.session_id,'session');self.assertIsNone(decision.reference.scope.connection_id)
        self.assertEqual(self.calls()[-1]['request']['action']['reference']['scope']['connection_id'],'connection')
        restored=client.operations.recover(decision.reference)
        self.assertEqual((await restored.wait(OPTIONS)).value.target,'original')

    async def test_foreign_native_submission_reference_retains_original_uncertainty(self):
        client=self.client({'mode':'wrong-reference'})
        with self.assertRaises(SdkError) as observed:await self.connection(client).update(Quality(250),REVISION,mutation())
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','uncertain'))
        self.assertEqual(observed.exception.operation.scope.connection_id,'connection')
        self.assertEqual(len(self.calls()),1)
