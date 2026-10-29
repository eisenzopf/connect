"""Shared isolated synthetic SDK fixtures; all owned children are joined."""
import asyncio
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from connect_sdk import Client, RequestOptions, SdkError

ROOT = Path(__file__).resolve().parents[2]
OPTIONS = RequestOptions(1500)


class PythonSdkFixture(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        base=ROOT/'.local/gate-6/python-sdk-unit';base.mkdir(parents=True,exist_ok=True,mode=0o700)
        self.directory=Path(tempfile.mkdtemp(prefix='unit-',dir=base))
        self.clients=[];self.loop_errors=[]
        asyncio.get_running_loop().set_exception_handler(lambda loop,context:self.loop_errors.append(context))

    def client(self, config=None):
        directory=self.directory/str(len(self.clients));directory.mkdir(mode=0o700)
        path=directory/'config.json';path.write_text(json.dumps(config or {}));path.chmod(0o600)
        client=Client(path,ROOT/'sdk/tests/fixtures/sdk_helper.mjs');self.clients.append((client,directory))
        return client

    def calls(self, index=-1):
        path=self.clients[index][1]/'requests.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    async def sent(self):
        async with asyncio.timeout(2):
            while not self.calls():await asyncio.sleep(.005)

    async def asyncTearDown(self):
        for client,directory in self.clients:
            try:await client.close(OPTIONS)
            except SdkError as error:self.assertEqual(error.kind,'cleanup-incomplete')
            process=client._helper._process
            if process is not None:
                await asyncio.wait_for(process.wait(),1)
                self.assertIsNotNone(process.returncode)
                with self.assertRaises(ProcessLookupError):os.kill(process.pid,0)
            if client._helper._tasks:
                await asyncio.wait_for(asyncio.gather(*tuple(client._helper._tasks),return_exceptions=True),1)
        shutil.rmtree(self.directory)
        self.assertEqual(self.loop_errors,[])

