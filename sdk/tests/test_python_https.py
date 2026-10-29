"""Actual local CA chain with platform TLS validation and explicit HTTPS profile."""
import json
from pathlib import Path
import shutil
import ssl
import subprocess
import tempfile
from unittest.mock import patch

from connect_sdk import HttpCommandScope,OperationId,ConversationId
from _python_http_fixture import ROOT,OPTIONS,PythonHttpFixture,batch,reply


class TypedPythonHttps(PythonHttpFixture):
    @classmethod
    def setUpClass(cls):
        base=ROOT/'.local/gate-6/python-http-tls-unit';base.mkdir(mode=0o700,parents=True,exist_ok=True)
        cls.tls_directory=Path(tempfile.mkdtemp(prefix='tls-',dir=base))
        def run(*args):subprocess.run(['openssl',*args],cwd=cls.tls_directory,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=5,check=True)
        run('req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:P-256','-nodes','-keyout','root.key','-out','root.pem','-days','1','-subj','/CN=Connect Python HTTP fixture root','-addext','basicConstraints=critical,CA:TRUE','-addext','keyUsage=critical,keyCertSign,cRLSign','-addext','subjectKeyIdentifier=hash')
        run('req','-newkey','ec','-pkeyopt','ec_paramgen_curve:P-256','-nodes','-keyout','leaf.key','-out','leaf.csr','-subj','/CN=Connect Python HTTP fixture server')
        (cls.tls_directory/'leaf.cnf').write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\n')
        run('x509','-req','-in','leaf.csr','-CA','root.pem','-CAkey','root.key','-CAcreateserial','-out','leaf.pem','-days','1','-extfile','leaf.cnf')
        run('verify','-x509_strict','-CAfile','root.pem','leaf.pem')
        for path in cls.tls_directory.iterdir():path.chmod(0o600)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tls_directory)

    async def tls_server(self,action=None):
        context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.tls_directory/'leaf.pem',self.tls_directory/'leaf.key')
        return await self.server(action,tls=context)

    async def test_explicit_https_uses_normal_ca_chain_and_keeps_original_mutation(self):
        endpoint=await self.tls_server()
        with patch.dict('os.environ',{'SSL_CERT_FILE':str(self.tls_directory/'root.pem')}):
            client=self.client(endpoint,transport='remote-https')
            values=await client.request('message.send',{'private':'synthetic'},HttpCommandScope(ConversationId('room'),operation_id=OperationId('original')),OPTIONS)
        self.assertEqual(values[-1].value['operation_id'],'original')
        self.assertEqual(len(self.requests),1);self.assertTrue(self.helpers[0]._joined)

    async def test_untrusted_root_is_not_bypassed_by_legacy_disable_environment(self):
        self.allow_tls_alerts=True;endpoint=await self.tls_server()
        with patch.dict('os.environ',{'PYTHONHTTPSVERIFY':'0','SSL_CERT_FILE':str(self.tls_directory/'missing-root.pem')}):
            client=self.client(endpoint,transport='remote-https')
            await self.assert_error('trust','not-submitted',client.request('message.send',{},HttpCommandScope(ConversationId('room'),operation_id=OperationId('original')),OPTIONS),'original')
        self.assertEqual(self.requests,[]);self.assertTrue(self.helpers[0]._joined)

    async def test_trusted_ca_does_not_disable_hostname_validation(self):
        self.allow_tls_alerts=True;endpoint=(await self.tls_server()).replace('127.0.0.1','localhost')
        with patch.dict('os.environ',{'SSL_CERT_FILE':str(self.tls_directory/'root.pem')}):
            client=self.client(endpoint,transport='remote-https')
            await self.assert_error('trust','read-only',client.request('operation.get',{'operation_id':'original'},HttpCommandScope(ConversationId('room')),OPTIONS))
        self.assertEqual(self.requests,[])
