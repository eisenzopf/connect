"""Actual isolated local HTTP server and owned Python workers; no remote accounts."""
import asyncio
import json
import os
from pathlib import Path
import shutil
import ssl
import tempfile
import unittest
from unittest.mock import patch

from connect_sdk import CanonicalHttpClient, RequestOptions
from connect_sdk._sdk_helper import _SdkHelper

ROOT=Path(__file__).resolve().parents[2]
OPTIONS=RequestOptions(2000)


def frame(request,kind='ack',payload=None,**extra):
    return {"v":2,"id":"reply","ts":"2026-10-28T00:00:00Z","type":kind,"payload":{} if payload is None else payload,
            **{k:request[k] for k in ('cid','sid','connid','operation_id') if k in request},"in_reply_to":request['id'],**extra}


def batch(request,frames=None):
    return {"profile":"connect/2","request_id":request['id'],"envelopes":[frame(request)] if frames is None else frames}


async def reply(writer,value,status=200,headers=None):
    raw=json.dumps(value,ensure_ascii=False,separators=(',',':')).encode() if not isinstance(value,bytes) else value
    head={"content-type":"application/json","content-length":str(len(raw)),"connection":"close",**(headers or {})}
    writer.write((f"HTTP/1.1 {status} Fixture\r\n"+''.join(k+': '+str(v)+'\r\n' for k,v in head.items())+'\r\n').encode()+raw)
    await writer.drain()


class PythonHttpFixture(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        base=ROOT/'.local/gate-6/python-http-unit';base.mkdir(mode=0o700,parents=True,exist_ok=True)
        self.directory=Path(tempfile.mkdtemp(prefix='unit-',dir=base))
        self.clients=[];self.helpers=[];self.servers=[];self.writers=set();self.handlers=set();self.requests=[];self.loop_errors=[]
        self.allow_tls_alerts=False
        def errors(loop,context):
            if self.allow_tls_alerts and isinstance(context.get('exception'),(ssl.SSLError,ConnectionResetError)):
                return
            self.loop_errors.append(context)
        asyncio.get_running_loop().set_exception_handler(errors)
        def owned(*args,**kwargs):
            helper=_SdkHelper(*args,**kwargs);self.helpers.append(helper);return helper
        self.capture=patch('connect_sdk.sdk_http._SdkHelper',side_effect=owned);self.capture.start()

    async def server(self,action=None,*,tls=None,host='127.0.0.1'):
        async def handle(reader,writer):
            task=asyncio.current_task();self.handlers.add(task);self.writers.add(writer)
            try:
                headers=await reader.readuntil(b'\r\n\r\n')
                lines=headers.decode('ascii').split('\r\n');fields={k.lower():v.strip() for k,v in (line.split(':',1) for line in lines[1:] if line)}
                raw=await reader.readexactly(int(fields.get('content-length','0')))
                value=json.loads(raw)
                self.requests.append({'line':lines[0],'headers':fields,'value':value,'raw':raw})
                if action is None:await reply(writer,batch(value))
                else:await action(value,reader,writer)
            except (asyncio.IncompleteReadError,ConnectionResetError,BrokenPipeError):
                pass
            finally:
                writer.close()
                try:await writer.wait_closed()
                except (ConnectionResetError,BrokenPipeError):pass
                self.writers.discard(writer);self.handlers.discard(task)
        server=await asyncio.start_server(handle,host,0,ssl=tls)
        self.servers.append(server)
        return ('https' if tls is not None else 'http')+'://127.0.0.1:'+str(server.sockets[0].getsockname()[1])

    def client(self,endpoint,credential='synthetic-private-http-credential',transport='loopback-http'):
        client=CanonicalHttpClient(endpoint,credential,transport);self.clients.append(client);return client

    async def sent(self,count=1):
        async with asyncio.timeout(4):
            while len(self.requests)<count:await asyncio.sleep(.005)

    async def assert_error(self,kind,certainty,action,id=None):
        from connect_sdk import SdkError
        with self.assertRaises(SdkError) as error:await action
        self.assertEqual((error.exception.kind,error.exception.certainty),(kind,certainty))
        if id:self.assertEqual(error.exception.operation.operation_id,id)
        self.assertNotIn('private',str(error.exception));return error.exception

    async def asyncTearDown(self):
        try:
            for client in self.clients:await client.close(RequestOptions(3000))
            for helper in self.helpers:
                await helper.close(RequestOptions(2000))
                if helper._process is not None:
                    await asyncio.wait_for(helper._process.wait(),1)
                    with self.assertRaises(ProcessLookupError):os.kill(helper._process.pid,0)
                    self.assertTrue(helper._joined)
                if helper._tasks:await asyncio.wait_for(asyncio.gather(*tuple(helper._tasks),return_exceptions=True),1)
            for writer in self.writers:writer.close()
            for server in self.servers:server.close();await server.wait_closed()
            if self.handlers:await asyncio.wait_for(asyncio.gather(*tuple(self.handlers)),1)
            self.assertEqual(self.loop_errors,[])
        finally:
            self.capture.stop();shutil.rmtree(self.directory)
