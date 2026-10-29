"""Synthetic original SDK process lifecycle only; no native/server qualification."""
import asyncio
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time
from collections.abc import Mapping
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'sdk/python'))
from connect_sdk._sdk_helper import _SdkHelper, _encode, _parse
from connect_sdk.sdk_types import Cancellation, OperationKey, RequestOptions, Scope, SdkError

OPTIONS = RequestOptions(1500)
KEY = OperationKey('original', 'message.send', Scope('room'))


def intent():
    return {'type':'conversation_send','conversation_id':'room','message_id':'message',
            'to':['participant'],'text':'original','mutation':{'operation_id':'original','extensions':None}}


class OwnedSdkHelper(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        base=ROOT/'.local/gate-6/python-helper-unit'
        base.mkdir(parents=True,exist_ok=True,mode=0o700)
        self.directory=Path(tempfile.mkdtemp(prefix='unit-',dir=base))
        self.config=self.directory/'config.json'
        self.helpers=[]
        self.loop_errors=[]
        asyncio.get_running_loop().set_exception_handler(lambda loop,context:self.loop_errors.append(context))

    async def helper(self, config=None, executable=None, *, administrative=False):
        self.config.write_text(json.dumps(config or {}))
        self.config.chmod(0o600)
        helper=_SdkHelper(self.config, executable or ROOT/'sdk/tests/fixtures/sdk_helper.mjs', administrative=administrative)
        self.helpers.append(helper)
        return helper

    async def calls(self):
        path=self.directory/'requests.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    async def sent(self, count=1):
        async with asyncio.timeout(2):
            while len(await self.calls()) < count:
                await asyncio.sleep(.005)

    async def asyncTearDown(self):
        for helper in self.helpers:
            try:
                await helper.close(OPTIONS)
            except SdkError as error:
                self.assertEqual(error.kind,'cleanup-incomplete')
            if helper._process is not None:
                await asyncio.wait_for(helper._process.wait(),1)
                self.assertIsNotNone(helper._process.returncode)
            if helper._tasks:
                await asyncio.wait_for(asyncio.gather(*tuple(helper._tasks),return_exceptions=True),1)
        shutil.rmtree(self.directory)
        self.assertEqual(self.loop_errors, [], "Owned cleanup must not leak shield/future/process errors")

    async def test_original_process_captures_queued_intent_and_keeps_profile_and_logical_identity(self):
        helper=await self.helper({'delayMs':80})
        first=asyncio.create_task(helper.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v))
        await self.sent()
        action=intent()
        queued=asyncio.create_task(helper.request(action,OPTIONS,lambda v:v,KEY,True))
        await asyncio.sleep(0)
        action['text']='changed';action['mutation']['operation_id']='replacement'
        original=await queued;await first
        self.assertEqual(original['operation_id'],'original')
        calls=await self.calls()
        self.assertEqual(calls[1]['request']['action']['text'],'original')
        self.assertEqual(calls[1]['request']['action']['mutation']['operation_id'],'original')
        self.assertEqual({v['profile'] for v in calls},{'connect-sdk-helper/4'})
        self.assertEqual(json.loads((self.directory/'argv.json').read_text()),['sdk-stdio','--config',str(self.config)])
        self.assertNotEqual(calls[0]['id'],calls[1]['id'])

    async def test_unsent_deadline_removes_only_that_job_without_fencing_or_restarting(self):
        helper=await self.helper({'delayMs':150})
        first=asyncio.create_task(helper.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v))
        await self.sent()
        with self.assertRaises(SdkError) as observed:
            await helper.request(intent(),RequestOptions(30),lambda v:v,KEY,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('deadline','not-submitted'))
        await first
        await helper.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v)
        self.assertEqual(len(await self.calls()),2)
        self.assertFalse(helper._closed)

    async def test_queued_explicit_cancellation_is_not_submitted_and_preserves_control(self):
        helper=await self.helper({'delayMs':150})
        first=asyncio.create_task(helper.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v))
        await self.sent();signal=Cancellation()
        queued=asyncio.create_task(helper.request(intent(),RequestOptions(1000,signal),lambda v:v,KEY,True))
        await asyncio.sleep(0);signal.cancel()
        with self.assertRaises(SdkError) as observed:await queued
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('cancelled','not-submitted'))
        await first;await helper.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v)
        self.assertEqual(len(await self.calls()),2)

    async def test_sent_deadline_fences_original_owner_and_preserves_mutation_uncertainty(self):
        helper=await self.helper({'delayMs':200})
        with self.assertRaises(SdkError) as observed:
            await helper.request(intent(),RequestOptions(80),lambda v:v,KEY,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('deadline','uncertain'))
        self.assertEqual(observed.exception.operation,KEY)
        with self.assertRaises(SdkError) as later:
            await helper.request(intent(),OPTIONS,lambda v:v,KEY,True)
        self.assertEqual((later.exception.kind,later.exception.certainty),('configuration','not-submitted'))
        self.assertLessEqual(len(await self.calls()),1)

    async def test_python_task_cancellation_stays_cancelled_and_retains_original_intent_metadata(self):
        helper=await self.helper({'delayMs':200})
        pending=asyncio.create_task(helper.request(intent(),OPTIONS,lambda v:v,KEY,True))
        await self.sent();pending.cancel()
        with self.assertRaises(asyncio.CancelledError) as observed:await pending
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('cancelled','uncertain'))
        self.assertEqual(observed.exception.operation,KEY)
        self.assertTrue(pending.cancelled());self.assertTrue(helper._closed)

    async def test_corrupt_foreign_duplicate_oversized_and_coercible_replies_fail_closed(self):
        for mode in ['profile','profile-old','wrong-id','utf8','duplicate','extra','oversize','phase-array','phase-missing','startup-success','startup-uncertain','error-kind-array','error-certainty-array']:
            with self.subTest(mode=mode):
                helper=await self.helper({'mode':mode})
                with self.assertRaises(SdkError) as observed:
                    await helper.request(intent(),OPTIONS,lambda v:v,KEY,True)
                self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','uncertain'))
                self.assertEqual(observed.exception.operation,KEY)
                await helper.close(OPTIONS)

    async def test_decode_scope_failure_is_protocol_not_an_unrelated_transport_failure(self):
        helper=await self.helper()
        def refuse(_):raise SdkError('protocol','read-only')
        with self.assertRaises(SdkError) as observed:
            await helper.request(intent(),OPTIONS,refuse,KEY,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','uncertain'))

    async def test_structured_error_retains_recorded_observation_without_diagnostics_or_helper_loss(self):
        helper=await self.helper({'mode':'refuse','observation':{'state':'completed','private':['private-raw-body']}})
        with self.assertRaises(SdkError) as observed:
            await helper.request(intent(),OPTIONS,lambda v:v,KEY,True)
        error=observed.exception
        self.assertEqual((error.kind,error.certainty),('receipt-unavailable','recorded'))
        self.assertEqual(error.operation,KEY);self.assertEqual(error.observation['state'],'completed')
        self.assertNotIn('private-raw-body',str(error)+repr(error)+repr(error.problem))
        with self.assertRaises(TypeError):error.observation['state']='foreign'
        with self.assertRaises(AttributeError):error.certainty='uncertain'
        await helper.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v)
        self.assertFalse(helper._closed)

    async def test_error_correlation_cannot_substitute_another_logical_id_command_or_scope(self):
        exact=KEY._wire()
        helper=await self.helper({'mode':'refuse','errorOperation':exact})
        with self.assertRaises(SdkError) as observed:
            await helper.request(intent(),OPTIONS,lambda v:v,KEY,True)
        self.assertEqual(observed.exception.operation,KEY)
        self.assertEqual(observed.exception.certainty,'recorded')
        await helper.close(OPTIONS)
        for changed in [{**exact,'operationId':'foreign'},{**exact,'command':'session.end'},
                        {**exact,'scope':{**exact['scope'],'conversationId':'foreign'}}]:
            helper=await self.helper({'mode':'refuse','errorOperation':changed})
            with self.assertRaises(SdkError) as observed:
                await helper.request(intent(),OPTIONS,lambda v:v,KEY,True)
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','uncertain'))
            self.assertEqual(observed.exception.operation,KEY)
            await helper.close(OPTIONS)

    async def test_trusted_backend_has_its_own_exact_profile_and_never_downgrades(self):
        key=OperationKey('original','identity.provision')
        action={'type':'administrative_provision','command':{'type':'identity','operation_id':'original','alias':'synthetic'},'mutation':{'operation_id':'original'}}
        helper=await self.helper(administrative=True)
        value=await helper.request(action,OPTIONS,lambda v:v,key,True)
        self.assertEqual(value['command'],'identity.provision')
        self.assertEqual((await self.calls())[0]['profile'],'connect-sdk-administration/2')
        self.assertEqual(json.loads((self.directory/'argv.json').read_text())[0],'sdk-admin-stdio')
        await helper.close(OPTIONS)
        old=await self.helper({'mode':'profile-old'},administrative=True)
        with self.assertRaises(SdkError) as observed:
            await old.request(action,OPTIONS,lambda v:v,key,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('protocol','uncertain'))
        self.assertEqual(observed.exception.operation,key)

    async def test_only_valid_startup_phase_can_prove_no_mutation_was_submitted(self):
        helper=await self.helper({'mode':'startup'})
        with self.assertRaises(SdkError) as observed:
            await helper.request(intent(),OPTIONS,lambda v:v,KEY,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('authentication','not-submitted'))
        self.assertEqual(observed.exception.operation,KEY)

    async def test_local_validation_pre_cancellation_and_missing_executable_do_not_submit(self):
        helper=await self.helper();signal=Cancellation();signal.cancel()
        with self.assertRaises(SdkError) as observed:
            await helper.request(intent(),RequestOptions(100,signal),lambda v:v,KEY,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('cancelled','not-submitted'))
        for value in [0,True,1.5,60001]:
            with self.assertRaises(SdkError):RequestOptions(value)
        for action in [{'type':'capabilities_refresh','bad':float('nan')},{'type':'capabilities_refresh','bad':'\ud800'},{'type':'capabilities_refresh','bad':'x'*65000}]:
            with self.assertRaises(SdkError):await helper.request(action,OPTIONS,lambda v:v)
        self.assertIsNone(helper._process);self.assertFalse((self.directory/'pid').exists())
        missing=await self.helper(executable=self.directory/'missing')
        with self.assertRaises(SdkError) as observed:
            await missing.request(intent(),OPTIONS,lambda v:v,KEY,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('configuration','not-submitted'))

    async def test_queue_budget_and_close_settle_original_waits_without_replay(self):
        helper=await self.helper({'delayMs':300})
        first=asyncio.create_task(helper.request(intent(),OPTIONS,lambda v:v,KEY,True));await self.sent()
        pending=[asyncio.create_task(helper.request(intent(),OPTIONS,lambda v:v,KEY,True)) for _ in range(15)]
        await asyncio.sleep(0)
        with self.assertRaises(SdkError) as overflow:await helper.request(intent(),OPTIONS,lambda v:v,KEY,True)
        self.assertEqual((overflow.exception.kind,overflow.exception.certainty),('application','not-submitted'))
        await helper.close(OPTIONS)
        results=await asyncio.gather(first,*pending,return_exceptions=True)
        self.assertEqual([e.certainty for e in results],['uncertain']+['not-submitted']*15)
        self.assertTrue(all(e.kind=='cancelled' and e.operation==KEY for e in results))
        self.assertEqual(len(await self.calls()),1)

    async def test_close_during_process_creation_retains_one_owned_start_and_never_submits_the_cancelled_job(self):
        helper=await self.helper();original=asyncio.create_subprocess_exec
        started=asyncio.Event();release=asyncio.Event();spawns=0
        async def delayed(*args,**kwargs):
            nonlocal spawns
            spawns+=1;started.set();await release.wait()
            return await original(*args,**kwargs)
        with mock.patch('asyncio.create_subprocess_exec',new=delayed):
            pending=asyncio.create_task(helper.request(intent(),OPTIONS,lambda v:v,KEY,True))
            await asyncio.wait_for(started.wait(),1)
            with self.assertRaises(SdkError) as cleanup:await helper.close(RequestOptions(10))
            self.assertEqual(cleanup.exception.kind,'cleanup-incomplete')
            with self.assertRaises(SdkError) as observed:await pending
            self.assertEqual((observed.exception.kind,observed.exception.certainty),('cancelled','not-submitted'))
            self.assertEqual(observed.exception.operation,KEY)
            release.set();await helper.close(OPTIONS)
        self.assertEqual(spawns,1);self.assertTrue(helper._joined)
        self.assertEqual(await self.calls(),[])

    async def test_graceful_original_process_close_joins_and_forced_close_remains_incomplete(self):
        helper=await self.helper({'signalDrainMs':40})
        await helper.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v)
        await helper.close(OPTIONS)
        self.assertEqual(helper._process.returncode,0);self.assertTrue(helper._joined)
        self.assertEqual(json.loads((self.directory/'drained.json').read_text()),{'drained':True})
        self.assertEqual((self.directory/'signals.jsonl').read_text().splitlines(),['"SIGTERM"'])
        # Use a separate original process after the first has actually joined.
        other=await self.helper({'ignoreTermination':True})
        await other.request({'type':'capabilities_refresh'},OPTIONS,lambda v:v)
        with self.assertRaises(SdkError) as observed:await other.close(RequestOptions(50))
        self.assertEqual(observed.exception.kind,'cleanup-incomplete')
        await asyncio.wait_for(other._process.wait(),1)
        self.assertTrue(other._force_killed)

    async def test_absolute_deadline_includes_intent_capture_before_process_or_queue_admission(self):
        class SlowMapping(Mapping):
            def __len__(self):return 1
            def __iter__(self):time.sleep(.03);return iter(['marker'])
            def __getitem__(self,key):return True
        helper=await self.helper();action=intent();action['mutation']['extensions']={'vendor':SlowMapping()}
        with self.assertRaises(SdkError) as observed:
            await helper.request(action,RequestOptions(10),lambda v:v,KEY,True)
        self.assertEqual((observed.exception.kind,observed.exception.certainty),('deadline','not-submitted'))
        self.assertIsNone(helper._process);self.assertEqual(await self.calls(),[])

    async def test_json_budgets_preserve_dense_large_replies_without_accepting_duplicate_or_nonfinite_values(self):
        value={'values':[0]*200000,'vendor':'x'*600000}
        parsed=_parse(json.dumps(value).encode(),large=True)
        self.assertEqual(parsed,value)
        for raw in [b'{"id":1,"id":2}',b'{"bad":NaN}',b'{"bad":"\\ud800"}',b'{"big":18446744073709551616}']:
            with self.assertRaises((ValueError,UnicodeError)):_parse(raw)
        with self.assertRaises(ValueError):_encode({'bad':2**64},65536)
