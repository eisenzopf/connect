"""Synchronous owner exercises actual child lifetime and unchanged typed SDK calls."""
import asyncio
from dataclasses import FrozenInstanceError
import json
import os
from pathlib import Path
import shutil
import socket
import tempfile
import threading
import time
import unittest

from connect_sdk import (AdministrativeClient, Cancellation, CanonicalHttpClient,
                        Client, Conversation, HttpCommandScope, MutationOptions,
                        OperationId, RequestOptions, SdkError, SynchronousSdk)
from _python_http_fixture import batch, reply

ROOT = Path(__file__).resolve().parents[2]
OPTIONS = RequestOptions(1500)


class TypedPythonSync(unittest.TestCase):
    def setUp(self):
        base=ROOT/'.local/gate-6/python-sync-unit';base.mkdir(parents=True,exist_ok=True,mode=0o700)
        self.directory=Path(tempfile.mkdtemp(prefix='unit-',dir=base))
        self.owners=[];self.errors=[];self.processes=[]

    def owner(self,config=None,kind=Client):
        path=self.directory/str(len(self.owners));path.mkdir(mode=0o700)
        file=path/'config.json';file.write_text(json.dumps(config or {}));file.chmod(0o600)
        executable=ROOT/'sdk/tests/fixtures/sdk_helper.mjs'
        if kind is AdministrativeClient:
            executable=path/'connect.mjs'
            executable.write_text('#!/usr/bin/env node\nprocess.env.CONNECT_SDK_FIXTURE_CONFIG='+json.dumps(str(file))+';\nawait import('+json.dumps((ROOT/'sdk/tests/fixtures/sdk_helper.mjs').as_uri())+');\n')
            executable.chmod(0o700)
        client=kind(file,executable)
        owner=SynchronousSdk(client);self.owners.append((owner,path))
        async def configure(sdk):
            asyncio.get_running_loop().set_exception_handler(lambda loop,context:self.errors.append(context))
        owner.run(configure)
        return owner

    def calls(self,index=-1):
        file=self.owners[index][1]/'requests.jsonl'
        return [json.loads(line) for line in file.read_text().splitlines()] if file.exists() else []

    def tearDown(self):
        for owner,path in self.owners:
            for helper in owner._helpers():
                if helper._process is not None:self.processes.append(helper._process)
            try:owner.close(OPTIONS)
            except SdkError as error:self.assertEqual(error.kind,'cleanup-incomplete')
            self.assertTrue(owner._closed)
            self.assertTrue(owner._loop.is_closed())
        for process in self.processes:
            self.assertIsNotNone(process.returncode)
            with self.assertRaises(ProcessLookupError):os.kill(process.pid,0)
        self.assertEqual(self.errors,[])
        shutil.rmtree(self.directory)

    def test_typed_operation_and_handles_keep_one_loop_process_and_original_record_across_calls(self):
        owner=self.owner();caller=threading.get_ident();observed=[]
        async def thread(sdk):observed.append(threading.get_ident())
        owner.run(thread)
        operation=owner.run(lambda sdk:sdk.conversations.create([], 'persistent',MutationOptions(OperationId('original'),OPTIONS)))
        process=owner._client._helper._process
        room=owner.run(lambda sdk:operation.wait(OPTIONS)).value
        self.assertIsInstance(room,Conversation)
        self.assertEqual(room.id,'room')
        self.assertEqual(owner.run(lambda sdk:room.refresh(OPTIONS)).id,'room')
        self.assertEqual(owner.run(lambda sdk:operation.status(OPTIONS)).key.operation_id,'original')
        self.assertIs(owner._client._helper._process,process)
        owner.run(thread);self.assertEqual(observed,[caller,caller])
        self.assertEqual(len(self.calls()),4)
        with self.assertRaises(FrozenInstanceError):owner._client=None

    def test_unused_owner_context_closes_without_process_start_and_cannot_be_reused(self):
        owner=self.owner()
        with owner:self.assertIsNone(owner._client._helper._process)
        self.assertTrue(owner._closed);self.assertEqual(self.calls(),[])
        called=[]
        with self.assertRaises(SdkError):owner.run(lambda sdk:called.append(True))
        self.assertEqual(called,[])
        owner.close(OPTIONS)
        with self.assertRaises(SdkError):SynchronousSdk(owner._client)

    def test_existing_async_owner_and_duplicate_transfer_are_refused_without_new_work(self):
        owner=self.owner()
        with self.assertRaises(SdkError):SynchronousSdk(owner._client)
        path=self.directory/'bound.json';path.write_text('{}')
        client=Client(path,ROOT/'sdk/tests/fixtures/sdk_helper.mjs')
        async def bind():
            await client.capabilities.refresh(OPTIONS)
            with self.assertRaises(SdkError):SynchronousSdk(client)
            await client.close(OPTIONS)
            self.processes.append(client._helper._process)
        asyncio.run(bind())

    def test_foreign_thread_nested_loop_and_reentrancy_refuse_before_callback(self):
        owner=self.owner();called=[];errors=[]
        def foreign():
            try:owner.run(lambda sdk:called.append(True))
            except SdkError as error:errors.append(error.kind)
            try:owner.close(OPTIONS)
            except SdkError as error:errors.append(error.kind)
        thread=threading.Thread(target=foreign);thread.start();thread.join(1)
        self.assertFalse(thread.is_alive());self.assertEqual(errors,['configuration','configuration'])
        async def nested(sdk):
            with self.assertRaises(SdkError):owner.run(lambda _:called.append(True))
            with self.assertRaises(SdkError):owner.close(OPTIONS)
        owner.run(nested)
        async def foreign_loop():
            with self.assertRaises(SdkError):owner.run(lambda sdk:called.append(True))
        asyncio.run(foreign_loop())
        self.assertEqual(called,[]);self.assertEqual(self.calls(),[])

    def test_callback_errors_and_nonawaitable_results_do_not_replace_or_replay_the_owner(self):
        owner=self.owner()
        async def broken(sdk):raise ValueError('user callback')
        with self.assertRaisesRegex(ValueError,'user callback'):owner.run(broken)
        with self.assertRaises(SdkError) as observed:owner.run(lambda sdk:42)
        self.assertEqual(observed.exception.kind,'validation')
        self.assertEqual(owner.run(lambda sdk:sdk.capabilities.refresh(OPTIONS)).profile,'connect/2')
        self.assertEqual(len(self.calls()),1)

    def test_sent_deadline_retains_original_mutation_and_never_retries(self):
        owner=self.owner({'delayMs':500});start=time.monotonic()
        with self.assertRaises(SdkError) as observed:
            owner.run(lambda sdk:sdk.conversations.create([], 'persistent',MutationOptions(OperationId('original'),RequestOptions(200))))
        error=observed.exception
        self.assertEqual((error.kind,error.certainty,error.operation.operation_id),('deadline','uncertain','original'))
        owner.close(OPTIONS)
        self.assertLess(time.monotonic()-start,2)
        self.assertEqual(len(self.calls()),1)

    def test_precancelled_mutation_remains_unsent_without_worker_start(self):
        owner=self.owner();signal=Cancellation();signal.cancel()
        with self.assertRaises(SdkError) as observed:
            owner.run(lambda sdk:sdk.conversations.create([], 'persistent',MutationOptions(OperationId('original'),RequestOptions(1000,signal))))
        self.assertEqual((observed.exception.kind,observed.exception.certainty,observed.exception.operation.operation_id),('cancelled','not-submitted','original'))
        self.assertEqual(self.calls(),[]);self.assertIsNone(owner._client._helper._process)

    def test_thread_safe_signal_cancels_sent_original_mutation_and_joins_helper(self):
        owner=self.owner({'delayMs':1000});signal=Cancellation();stopped=threading.Event()
        def cancel_sent():
            until=time.monotonic()+2
            while not self.calls() and time.monotonic()<until:stopped.wait(.005)
            signal.cancel()
        thread=threading.Thread(target=cancel_sent);thread.start()
        try:
            with self.assertRaises(SdkError) as observed:
                owner.run(lambda sdk:sdk.conversations.create([], 'persistent',MutationOptions(OperationId('original'),RequestOptions(1500,signal))))
            self.assertEqual((observed.exception.kind,observed.exception.certainty,observed.exception.operation.operation_id),('cancelled','uncertain','original'))
            owner.close(OPTIONS)
            self.assertEqual(len(self.calls()),1)
        finally:stopped.set();thread.join(2)
        self.assertFalse(thread.is_alive())

    def test_close_cancels_callback_tasks_and_drains_original_helper_without_background_thread(self):
        owner=self.owner({'delayMs':500});retained=[];cancelled=[]
        async def pending(sdk):
            async def work():
                try:await sdk.capabilities.refresh(OPTIONS)
                finally:cancelled.append(True)
            retained.append(asyncio.create_task(work()))
            while not self.calls():await asyncio.sleep(.005)
        owner.run(pending);process=owner._client._helper._process
        owner.close(OPTIONS)
        self.assertEqual(cancelled,[True]);self.assertTrue(retained[0].done())
        self.assertIsNotNone(process.returncode)

    def test_incomplete_close_retains_loop_and_original_process_until_later_join(self):
        owner=self.owner({'ignoreTermination':True})
        owner.run(lambda sdk:sdk.capabilities.refresh(OPTIONS));process=owner._client._helper._process
        with self.assertRaises(SdkError) as observed:owner.close(RequestOptions(1))
        self.assertEqual(observed.exception.kind,'cleanup-incomplete')
        if not owner._closed:self.assertFalse(owner._loop.is_closed())
        with self.assertRaises(SdkError):owner.run(lambda sdk:sdk.capabilities.refresh(OPTIONS))
        try:owner.close(OPTIONS)
        except SdkError as error:self.assertEqual(error.kind,'cleanup-incomplete')
        self.assertTrue(owner._closed);self.assertIsNotNone(process.returncode)

    def test_administrative_role_uses_separate_profile_and_keeps_typed_recovery(self):
        owner=self.owner(kind=AdministrativeClient)
        async def provision(admin):
            command=admin.validate_provisioning({'type':'identity','operation_id':'original','alias':'local','kind':'human','display_name':'Local'})
            return await admin.provision(command,MutationOptions(OperationId('original'),OPTIONS))
        operation=owner.run(provision)
        self.assertEqual(operation.reference.command,'identity.provision')
        self.assertEqual(owner.run(lambda admin:operation.status(OPTIONS)).key.operation_id,'original')
        self.assertTrue(all(row['profile']=='connect-sdk-administration/2' for row in self.calls()))
        self.assertFalse(hasattr(owner._client,'conversations'))

    def test_http_owner_runs_actual_local_socket_and_joins_per_exchange_worker(self):
        listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen()
        client=CanonicalHttpClient('http://127.0.0.1:'+str(listener.getsockname()[1]),'synthetic')
        owner=SynchronousSdk(client);self.owners.append((owner,self.directory/'http'))
        async def local(http):
            async def respond(reader,writer):
                try:
                    header=await reader.readuntil(b'\r\n\r\n')
                    length=int(next(line.split(b':',1)[1] for line in header.split(b'\r\n') if line.lower().startswith(b'content-length:')))
                    value=json.loads(await reader.readexactly(length));await reply(writer,batch(value))
                finally:writer.close();await writer.wait_closed()
            server=await asyncio.start_server(respond,sock=listener)
            try:
                return await http.request('operation.get',{'operation_id':'original'},HttpCommandScope(),OPTIONS)
            finally:server.close();await server.wait_closed()
        values=owner.run(local)
        self.assertEqual(values[-1].value['type'],'ack')
        self.assertEqual(client._active,set())

    def test_stubborn_callback_cleanup_has_bounded_refusal_and_retains_loop_for_retry(self):
        owner=self.owner();release=Cancellation();tasks=[]
        async def setup(sdk):
            async def stubborn():
                while not release.is_cancelled:
                    try:await asyncio.sleep(.01)
                    except asyncio.CancelledError:pass
            tasks.append(asyncio.create_task(stubborn()));await asyncio.sleep(0)
        owner.run(setup);start=time.monotonic()
        with self.assertRaises(SdkError) as observed:owner.close(RequestOptions(20))
        self.assertEqual(observed.exception.kind,'cleanup-incomplete');self.assertLess(time.monotonic()-start,.3)
        self.assertFalse(owner._closed);self.assertFalse(owner._loop.is_closed())
        release.cancel();owner.close(OPTIONS)
        self.assertTrue(tasks[0].done());self.assertTrue(owner._closed)
