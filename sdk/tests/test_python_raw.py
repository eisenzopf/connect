"""Finite raw canonical metadata/exchange over a synthetic frozen-parser child."""
import asyncio
from dataclasses import FrozenInstanceError
import json
import os
from pathlib import Path
import time

from connect_sdk import (
    Cancellation, Client, MessageId, MutationOptions, NonEmpty, OperationId,
    ParticipantId, RequestOptions, SdkCancelledError, SdkError, ValidatedEnvelope,
)
from _python_sdk_fixture import OPTIONS, ROOT, PythonSdkFixture


def get():
    return {'v':2,'id':'saved-attempt','ts':'2026-10-28T00:00:00Z','type':'operation.get','cid':'room','payload':{'operation_id':'original'}}


def mutation():
    return {**get(),'type':'conversation.close','operation_id':'original-close','payload':{'reason':'private-done'}}


class TypedPythonRaw(PythonSdkFixture):
    def raw_client(self, config=None):
        self.client(config)
        _,directory=self.clients[-1]
        executable=directory/'connect.mjs'
        # Only the synthetic executable learns a fixture path. SDK validation
        # itself passes no configuration, credentials or network selector.
        executable.write_text('#!/usr/bin/env node\nprocess.env.CONNECT_SDK_FIXTURE_CONFIG='+json.dumps(str(directory/'config.json'))+';\nawait import('+json.dumps((ROOT/'sdk/tests/fixtures/sdk_helper.mjs').as_uri())+');\n')
        executable.chmod(0o700)
        client=Client(directory/'config.json',executable)
        self.clients[-1]=(client,directory)
        return client

    def validations(self, index=-1):
        path=self.clients[index][1]/'validation.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    async def asyncTearDown(self):
        try:
            for _,directory in self.clients:
                path=directory/'validation.jsonl'
                for value in ([json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []):
                    with self.assertRaises(ProcessLookupError):os.kill(value['pid'],0)
        finally:
            await super().asyncTearDown()

    async def assert_error(self, kind, certainty, action, id=None):
        with self.assertRaises(SdkError) as observed:await action
        self.assertEqual((observed.exception.kind,observed.exception.certainty),(kind,certainty))
        if id:self.assertEqual(observed.exception.operation.operation_id,id)

    async def test_local_frozen_validation_captures_immutable_original_intent_without_sdk_io(self):
        client=self.raw_client();value=get();value['payload']['vendor']={'optional':'preserved'}
        checked=client.raw.validate(value);value['cid']='foreign';value['payload']['vendor']['optional']='changed'
        self.assertEqual(checked.value['cid'],'room');self.assertEqual(checked.value['payload']['vendor']['optional'],'preserved')
        with self.assertRaises(TypeError):checked.value['payload']['vendor']['optional']='changed'
        with self.assertRaises(AttributeError):checked.value={}
        exported=checked.to_json();exported['cid']='changed-copy'
        self.assertEqual(checked.value['cid'],'room');self.assertIsNone(client._helper._process)
        self.assertEqual(self.calls(),[]);self.assertEqual(self.validations()[0]['argv'],['sdk-validate-envelope'])
        self.assertNotIn('preserved',repr(checked));self.assertNotIn(str(self.directory),repr(client.raw))

    async def test_raw_requests_require_original_sdk_token_identity(self):
        client=self.raw_client();checked=client.raw.validate(get())
        class Spoof:
            def __eq__(self,other):return True
            def __hash__(self):return hash(checked)
        for value in [None,{},{'value':checked.value},object.__new__(ValidatedEnvelope),Spoof()]:
            await self.assert_error('validation','not-submitted',client.raw.request(value,OPTIONS))
        with self.assertRaises(SdkError):ValidatedEnvelope()
        self.assertEqual(self.calls(),[])

    async def test_raw_exchange_preserves_payload_signature_and_returned_frame_identity(self):
        client=self.raw_client();checked=client.raw.validate(get());replies=await client.raw.request(checked,OPTIONS)
        self.assertIsInstance(replies,tuple);self.assertEqual(replies[-1].value['payload']['details']['original'],'preserved')
        self.assertEqual(replies[-1].value['in_reply_to'],'fresh-wire-request')
        self.assertNotEqual(replies[-1].value['in_reply_to'],checked.value['id'])
        self.assertEqual(replies[-1].value['signature']['alg'],'EdDSA')
        with self.assertRaises(TypeError):replies[0].value['signature']['alg']='ES256'
        await self.assert_error('unsupported','read-only',client.raw.request(replies[0],OPTIONS))
        self.assertEqual(len(self.calls()),1)
        self.assertEqual(self.calls()[0]['request']['action']['envelope'],get())

    async def test_full_schema_and_json_bounds_reject_before_managed_io(self):
        client=self.raw_client()
        deep={}
        for _ in range(34):deep={'nested':deep}
        for value in [None,{}, {**get(),'payload':{'operation_id':7}}, {**get(),'v':True},
                      {**get(),'ts':'not-a-time'},{**get(),'expected_revision':True},
                      {**get(),'payload':{'operation_id':'x','oversize':'x'*1048576}},
                      {**get(),'payload':{'operation_id':'x','deep':deep}},
                      {**get(),'payload':{'vendor':float('nan')}},{**get(),'payload':{'vendor':'\ud800'}}]:
            with self.assertRaises(SdkError) as observed:client.raw.validate(value)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('validation','not-submitted'))
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)

    async def test_finite_lifecycle_support_comes_from_original_rust_descriptor(self):
        client=self.raw_client()
        for value in [{**get(),'type':'conversation.subscribe','payload':{'after':'cursor','live':True}},
                      {**mutation(),'type':'connection.offer','sid':'session','connid':'connection'}]:
            checked=client.raw.validate(value)
            await self.assert_error('unsupported','read-only' if value['type']=='conversation.subscribe' else 'not-submitted',client.raw.request(checked,OPTIONS))
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)

    async def test_raw_larger_than_typed_write_preserves_original_logical_id(self):
        client=self.raw_client({'rawSize':80000});value=mutation();value['payload']['vendor']='x'*80000
        replies=await client.raw.request(client.raw.validate(value),OPTIONS)
        self.assertEqual(len(replies[0].value['payload']['details']['vendor']),80000)
        self.assertEqual(replies[0].value['operation_id'],'original-close')
        await self.assert_error('unsupported','not-submitted',client.raw.request(replies[0],OPTIONS),'original-close')
        await self.assert_error('validation','not-submitted',client.conversations.by_id('room').send(MessageId('m'),NonEmpty(ParticipantId('p')),'x'*80000,MutationOptions(OperationId('typed-too-large'),OPTIONS)),'typed-too-large')
        self.assertEqual(len(self.calls()),1)

    async def test_raw_depth_and_dense_payload_use_envelope_bounds_without_widening_typed_input(self):
        for config in [{'rawDepth':29},{'rawNodes':99960}]:
            client=self.raw_client(config);value=get();vendor='leaf'
            for _ in range(30):vendor={'nested':vendor}
            value['payload']['vendor']=vendor
            replies=await client.raw.request(client.raw.validate(value),OPTIONS)
            data=replies[0].value['payload']['details']['vendor']
            if 'rawDepth' in config:
                for _ in range(29):data=data['nested']
                self.assertIsInstance(data,str)
            else:self.assertEqual(len(data),99960)

    async def test_corrupt_validator_profile_duplicate_keys_or_classification_fences_owner(self):
        for validator in ['profile','boolean','foreign-key','error-array','duplicate']:
            client=self.raw_client({'validator':validator})
            with self.assertRaises(SdkError) as observed:client.raw.validate(mutation())
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','not-submitted'))
            self.assertTrue(client._helper._closed);self.assertEqual(self.calls(),[])
            await client.close(OPTIONS)

    async def test_bad_signed_batch_fences_and_preserves_submitted_original_mutation_uncertainty(self):
        for mode in ['raw-correlation','raw-scope','raw-logical','raw-alg-array','raw-bad-signature','raw-error-frame','raw-no-ack','raw-duplicate','raw-after-ack','raw-empty','raw-wrong-descriptor',
                     'raw-time','raw-id-array','raw-payload-array','raw-v-bool','raw-revision']:
            client=self.raw_client({'mode':mode});checked=client.raw.validate(mutation())
            await self.assert_error('protocol','uncertain',client.raw.request(checked,OPTIONS),'original-close')
            self.assertTrue(client._helper._closed);self.assertEqual(len(self.calls()),1)
            await client.close(OPTIONS)

    async def test_bad_signed_read_remains_read_only(self):
        client=self.raw_client({'mode':'raw-correlation'});checked=client.raw.validate(get())
        await self.assert_error('protocol','read-only',client.raw.request(checked,OPTIONS))

    async def test_finite_journal_replay_preserves_fact_scopes_and_optional_data(self):
        client=self.raw_client({'mode':'raw-journal'});value={**get(),'type':'conversation.subscribe','payload':{'after':'saved-cursor','live':False}}
        replies=await client.raw.request(client.raw.validate(value),OPTIONS)
        self.assertEqual(len(replies),2);fact=replies[0].value
        self.assertEqual((fact['cid'],fact['sid'],fact['connid'],fact['operation_id']),('room','session','connection','original-fact'))
        self.assertEqual(fact['payload']['data']['data']['private'],'retained optional fact')
        self.assertEqual(replies[-1].value['type'],'ack')
        await self.assert_error('unsupported','not-submitted',client.raw.request(replies[0],OPTIONS),'original-fact')

    async def test_foreign_finite_journal_fact_identity_and_scope_are_rejected(self):
        for mode in ['raw-journal-correlation','raw-journal-scope']:
            client=self.raw_client({'mode':mode});value={**get(),'type':'conversation.subscribe','payload':{'after':'saved-cursor','live':False}}
            await self.assert_error('protocol','read-only',client.raw.request(client.raw.validate(value),OPTIONS))
            await client.close(OPTIONS)

    async def test_session_receipt_and_recording_replay_observations_keep_their_scope_rules(self):
        for mode,kind in [('raw-session-event','session.started'),('raw-receipt-event','message.delivered'),('raw-recording-event','recording.vcon-ready')]:
            client=self.raw_client({'mode':mode});value={**get(),'type':'conversation.subscribe','payload':{'live':False}}
            replies=await client.raw.request(client.raw.validate(value),OPTIONS)
            self.assertEqual([v.value['type'] for v in replies],[kind,'ack'])

    async def test_invalid_session_receipt_and_recording_scope_or_evidence_fence_the_owner(self):
        for mode in ['raw-session-event-connection','raw-receipt-event-evidence','raw-recording-event-handle']:
            client=self.raw_client({'mode':mode});value={**get(),'type':'conversation.subscribe','payload':{'live':False}}
            await self.assert_error('protocol','read-only',client.raw.request(client.raw.validate(value),OPTIONS))
            self.assertTrue(client._helper._closed);await client.close(OPTIONS)

    async def test_server_assigned_conversation_and_both_signature_algorithm_tags_are_preserved(self):
        client=self.raw_client({'mode':'raw-assigned'});value={**get(),'type':'conversation.create','operation_id':'original-create','payload':{'participants':[],'policy':'persistent'}}
        del value['cid']
        replies=await client.raw.request(client.raw.validate(value),OPTIONS)
        self.assertEqual((replies[-1].value['cid'],replies[-1].value['operation_id']),('assigned-room','original-create'))
        es=self.raw_client({'mode':'raw-es256'});replies=await es.raw.request(es.raw.validate(get()),OPTIONS)
        self.assertEqual(replies[-1].value['signature']['alg'],'ES256')

    async def test_pre_cancelled_raw_mutation_never_starts_owner_and_original_wait_is_not_retried(self):
        client=self.raw_client({'mode':'partial'});checked=client.raw.validate(mutation());signal=Cancellation();signal.cancel()
        await self.assert_error('cancelled','not-submitted',client.raw.request(checked,RequestOptions(1000,signal)),'original-close')
        self.assertEqual(self.calls(),[]);self.assertIsNone(client._helper._process)
        await self.assert_error('deadline','uncertain',client.raw.request(checked,RequestOptions(50)),'original-close')
        await self.assert_error('configuration','not-submitted',client.raw.request(checked,OPTIONS),'original-close')
        self.assertEqual(len(self.calls()),1)

    async def test_cancelled_raw_task_preserves_original_key_and_joins_owned_process(self):
        client=self.raw_client({'delayMs':100});checked=client.raw.validate(mutation())
        waiter=asyncio.create_task(client.raw.request(checked,OPTIONS));await self.sent();waiter.cancel('private cancellation')
        with self.assertRaises(SdkCancelledError) as observed:await waiter
        self.assertEqual((observed.exception.kind,observed.exception.certainty,observed.exception.operation.operation_id),('cancelled','uncertain','original-close'))
        self.assertTrue(waiter.cancelled());self.assertEqual(len(self.calls()),1)
        self.assertNotIn('private',str(observed.exception));await client.close(OPTIONS)
        self.assertIsNotNone(client._helper._process.returncode)

    async def test_validator_time_and_output_are_bounded_and_original_child_joined(self):
        for validator in ['oversize','hang']:
            client=self.raw_client({'validator':validator});start=time.monotonic()
            with self.assertRaises(SdkError) as observed:client.raw.validate(get())
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('configuration','not-submitted'))
            self.assertLess(time.monotonic()-start,3);self.assertFalse(client._helper._closed)
            self.assertEqual(self.calls(),[])

    async def test_missing_validator_has_stable_private_configuration_error(self):
        client=Client(self.directory/'private-missing-config',self.directory/'private-missing-executable')
        self.clients.append((client,self.directory/'unused'))
        with self.assertRaises(SdkError) as observed:client.raw.validate(mutation())
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('configuration','not-submitted'))
        self.assertNotIn('private',str(observed.exception));self.assertIsNone(client._helper._process)
