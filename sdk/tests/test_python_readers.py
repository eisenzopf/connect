"""Owned reader behavior over synthetic Rust-profile replies; no server claim."""
import asyncio
from dataclasses import FrozenInstanceError

from connect_sdk import Cancellation, Cursor, RequestOptions, SdkCancelledError, SdkError
from _python_sdk_fixture import OPTIONS, PythonSdkFixture


class TypedPythonReaders(PythonSdkFixture):
    def events(self, client, options=OPTIONS, after=None, cid='room'):
        return client.conversations.by_id(cid).events(after,options)

    async def wait_calls(self, count):
        async with asyncio.timeout(2):
            while len(self.calls()) < count:await asyncio.sleep(.005)

    async def assert_error(self, kind, action):
        with self.assertRaises(SdkError) as error:await action
        self.assertEqual((error.exception.kind,error.exception.certainty),(kind,'read-only'))

    async def test_events_baseline_exact_ack_immutable_data_and_independent_control(self):
        client=self.client();stream=self.events(client,after=Cursor('saved.cursor~1'))
        self.assertEqual(await stream.ready(),'saved.cursor~1')
        first=await anext(stream)
        self.assertEqual((first.event_id,first.cursor,first.conversation_id),('event-1','cursor-1','room'))
        self.assertEqual(first.source.correlation_id,'original')
        with self.assertRaises(FrozenInstanceError):first.cursor='forged'
        with self.assertRaises(TypeError):first.data['private']='changed'
        with self.assertRaises(FrozenInstanceError):first.audience[0].membership_epoch=7
        self.assertNotIn('synthetic',repr(first));self.assertNotIn('saved.cursor',repr(stream))
        second=await stream.next()
        self.assertEqual(second.event_id,'event-2')
        polls=[v['request']['action'] for v in self.calls()]
        self.assertEqual([v['acknowledged'] for v in polls],[None,None,{'event_id':'event-1','cursor':'cursor-1'}])
        self.assertTrue(all(v['after']=='saved.cursor~1' for v in polls))
        await stream.close();await stream.close()
        self.assertEqual(sum(v['request']['action']['type']=='events_close' for v in self.calls()),1)
        self.assertEqual((await client.conversations.by_id('room').refresh(OPTIONS)).id,'room')
        with self.assertRaises(StopAsyncIteration):await anext(stream)

    async def test_readers_validate_locally_and_unstarted_close_has_no_process(self):
        client=self.client()
        for after in ['', 'bad cursor', '\n', 'é', 'x'*2049, 1]:
            with self.assertRaises(SdkError):self.events(client,after=after)
        with self.assertRaises(SdkError):self.events(client,options={})
        with self.assertRaises(SdkError):client.incoming_invitations({})
        for reader in [self.events(client),client.incoming_invitations(OPTIONS)]:
            await reader.aclose()
            with self.assertRaises(StopAsyncIteration):await anext(reader)
        self.assertIsNone(client._helper._process);self.assertEqual(self.calls(),[])

    async def test_event_local_timeout_preserves_unacknowledged_source_and_repolls(self):
        client=self.client({'eventsDelayMs':80});stream=self.events(client)
        await self.assert_error('deadline',stream.next(RequestOptions(20)))
        first=await stream.next(OPTIONS)
        self.assertEqual(first.event_id,'event-1')
        polls=[v['request']['action'] for v in self.calls()]
        self.assertGreaterEqual(len(polls),2);self.assertTrue(all(v['acknowledged'] is None for v in polls))
        await stream.close()

    async def test_event_task_cancellation_is_typed_and_does_not_cancel_original_poll(self):
        client=self.client({'eventsDelayMs':80});stream=self.events(client)
        waiter=asyncio.create_task(anext(stream));await self.sent();waiter.cancel('private')
        with self.assertRaises(SdkCancelledError) as error:await waiter
        self.assertEqual((error.exception.kind,error.exception.certainty),('cancelled','read-only'))
        self.assertTrue(waiter.cancelled());self.assertEqual((await anext(stream)).event_id,'event-1')
        self.assertIsNotNone(client._helper._process);self.assertFalse(client._helper._closed)
        self.assertTrue(all(v['request']['action']['acknowledged'] is None for v in self.calls()))
        await stream.close()

    async def test_cancelled_event_wait_never_returns_stale_reply_after_revocation(self):
        client=self.client({'mode':'events-revoked-after-first','eventsDelayMs':60});stream=self.events(client)
        waiter=asyncio.create_task(anext(stream));await self.sent();waiter.cancel()
        with self.assertRaises(SdkCancelledError):await waiter
        await self.assert_error('authentication',anext(stream))
        self.assertEqual([v['request']['action']['acknowledged'] for v in self.calls()],[None,None])
        await stream.close()

    async def test_explicit_signal_pre_cancel_and_cancelled_wait_are_read_only(self):
        signal=Cancellation();signal.cancel();client=self.client()
        stream=self.events(client,RequestOptions(1000,signal))
        await self.assert_error('cancelled',anext(stream));self.assertEqual(self.calls(),[])
        signal2=Cancellation();client2=self.client({'eventsDelayMs':70});stream2=self.events(client2,RequestOptions(1000,signal2))
        waiter=asyncio.create_task(anext(stream2));await self.sent();signal2.cancel()
        await self.assert_error('cancelled',waiter)
        self.assertEqual((await stream2.next(OPTIONS)).event_id,'event-1')
        await stream2.close()

    async def test_event_wait_concurrency_conflicts_and_close_wakes_pending_wait(self):
        client=self.client({'mode':'events-empty','eventsDelayMs':70});stream=self.events(client)
        waiter=asyncio.create_task(anext(stream));await self.sent()
        await self.assert_error('conflict',anext(stream))
        await stream.close();await self.assert_error('cancelled',waiter)
        self.assertTrue(stream._detached)
        self.assertEqual((await client.conversations.by_id('room').refresh(OPTIONS)).id,'room')

    async def test_event_context_closes_on_loop_break_and_application_exception(self):
        for fails in [False,True]:
            client=self.client();stream=self.events(client)
            try:
                async with stream:
                    async for fact in stream:
                        if fails:raise RuntimeError('application failed')
                        break
            except RuntimeError:
                self.assertTrue(fails)
            self.assertTrue(stream._detached)
            self.assertEqual([v['request']['action']['type'] for v in self.calls()],['events_poll','events_close'])

    async def test_event_corrupt_scope_cursor_audience_reader_and_shape_fence_owner(self):
        for mode in ['events-foreign','events-nested','events-resource','events-audience','events-cursor','events-extra','events-reader']:
            client=self.client({'mode':mode});stream=self.events(client)
            await self.assert_error('protocol',anext(stream));self.assertTrue(client._helper._closed)
            await client.close(OPTIONS)

    async def test_event_resync_and_source_protocol_remain_explicit_without_fencing_controls(self):
        for mode,kind in [('events-resync','resync-required'),('events-protocol','protocol')]:
            client=self.client({'mode':mode});stream=self.events(client)
            await self.assert_error(kind,anext(stream));self.assertFalse(client._helper._closed)
            await stream.close()
            self.assertEqual((await client.conversations.by_id('room').refresh(OPTIONS)).id,'room')

    async def test_duplicate_event_after_ack_fences_instead_of_silently_replaying(self):
        client=self.client({'mode':'events-duplicate'});stream=self.events(client)
        self.assertEqual((await anext(stream)).event_id,'event-1')
        await self.assert_error('protocol',anext(stream));self.assertTrue(client._helper._closed)

    async def test_large_and_dense_journal_data_preserves_original_owned_limits(self):
        for config,size in [({'eventSize':600000},600000),({'eventNodes':200000},200000)]:
            client=self.client(config);stream=self.events(client);event=await anext(stream)
            self.assertEqual(len(event.data['private']),size)
            await stream.close()

    async def test_event_reader_budget_and_one_source_per_conversation_are_enforced(self):
        client=self.client();streams=[self.events(client,cid=f'room-{i}') for i in range(8)]
        for stream in streams:self.assertEqual(await stream.ready(),'baseline-original')
        overflow=self.events(client,cid='overflow');await self.assert_error('configuration',overflow.ready())
        await overflow.close();await streams[0].close()
        duplicate=self.events(client,cid='room-1');await self.assert_error('conflict',duplicate.ready())
        await duplicate.close()
        replacement=self.events(client,cid='replacement');self.assertEqual(await replacement.ready(),'baseline-original')
        for stream in streams[1:]+[replacement]:await stream.close()

    async def test_incoming_is_immutable_typed_hint_and_ack_only_on_next_or_close(self):
        client=self.client();stream=client.incoming_invitations(OPTIONS);hint=await anext(stream)
        self.assertEqual((hint.session.conversation_id,hint.session.id,hint.from_participant,hint.to_participant,hint.operation_id),('room','session','caller','callee','original-invite'))
        self.assertEqual((hint.source.kind,hint.source.correlation_id),('session.invite','original-invite'))
        self.assertEqual(hint.capabilities['audio'],('g.711-mu',))
        with self.assertRaises(FrozenInstanceError):hint.intent='changed'
        with self.assertRaises(TypeError):hint.capabilities['data']=[]
        self.assertNotIn('synchronous-engagement',repr(hint))
        self.assertEqual(self.calls()[0]['request']['action']['acknowledged'],None)
        self.assertEqual((await hint.session.refresh(OPTIONS)).id,'session')
        await stream.close();await stream.close()
        self.assertEqual(self.calls()[-1]['request']['action']['acknowledged'],'delivery-original')
        with self.assertRaises(StopAsyncIteration):await anext(stream)

    async def test_incoming_timeout_retains_hint_and_current_health_is_rechecked(self):
        for mode,kind in [('',None),('incoming-revoked-after-first','authentication')]:
            client=self.client({'mode':mode,'incomingDelayMs':70});stream=client.incoming_invitations(OPTIONS)
            await self.assert_error('deadline',stream.next(RequestOptions(20)))
            if kind:await self.assert_error(kind,stream.next())
            else:self.assertEqual((await stream.next()).operation_id,'original-invite')
            self.assertEqual([v['request']['action']['acknowledged'] for v in self.calls()],[None,None])
            await stream.close()

    async def test_incoming_task_cancel_close_retains_unconsumed_hint_for_new_reader(self):
        client=self.client({'incomingDelayMs':70});stream=client.incoming_invitations(OPTIONS)
        waiter=asyncio.create_task(anext(stream));await self.sent();waiter.cancel()
        with self.assertRaises(SdkCancelledError):await waiter
        await stream.close();self.assertIsNone(self.calls()[-1]['request']['action']['acknowledged'])
        async with client.incoming_invitations(OPTIONS) as replacement:
            self.assertEqual((await anext(replacement)).operation_id,'original-invite')
        self.assertEqual(self.calls()[-1]['request']['action']['acknowledged'],'delivery-original')

    async def test_incoming_one_owner_and_one_waiter_are_enforced(self):
        client=self.client({'mode':'incoming-empty','incomingDelayMs':60});stream=client.incoming_invitations(OPTIONS)
        waiter=asyncio.create_task(anext(stream));await self.sent()
        await self.assert_error('conflict',anext(stream))
        other=client.incoming_invitations(OPTIONS)
        await self.assert_error('conflict',anext(other))
        await stream.close();await self.assert_error('cancelled',waiter)
        await other.close()

    async def test_incoming_forged_source_medium_participants_intent_shape_or_reader_fences(self):
        for mode in ['incoming-source','incoming-medium','incoming-participants','incoming-intent','incoming-extra','incoming-reader','incoming-null']:
            client=self.client({'mode':mode});stream=client.incoming_invitations(OPTIONS)
            await self.assert_error('protocol',anext(stream));self.assertTrue(client._helper._closed)
            await client.close(OPTIONS)

    async def test_incoming_pending_polls_deliver_once_and_ack_on_following_poll(self):
        client=self.client({'incomingPendingPolls':2});stream=client.incoming_invitations(OPTIONS)
        self.assertEqual((await stream.next()).operation_id,'original-invite')
        self.assertEqual(len(self.calls()),3)
        await self.assert_error('deadline',stream.next(RequestOptions(40)))
        self.assertTrue(all(v['request']['action']['acknowledged']=='delivery-original' for v in self.calls()[3:]))
        await stream.close()

    async def test_client_close_joins_original_reader_tasks_and_unblocks_waiters(self):
        for incoming in [False,True]:
            client=self.client({'eventsDelayMs':70,'incomingDelayMs':70})
            stream=client.incoming_invitations(OPTIONS) if incoming else self.events(client)
            waiter=asyncio.create_task(anext(stream));await self.sent()
            await client.close(OPTIONS)
            await self.assert_error('cancelled',waiter)
            self.assertEqual(client._helper._tasks,set())
            self.assertIsNotNone(client._helper._process.returncode)
