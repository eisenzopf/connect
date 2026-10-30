"""Public namespace and one-shot worker startup dependency boundaries."""
import os
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]


class PythonImportBoundary(unittest.TestCase):
    def probe(self, source):
        subprocess.run([sys.executable, '-c', source], cwd=ROOT,
            env=dict(os.environ, PYTHONPATH=str(ROOT/'sdk/python'), PYTHONDONTWRITEBYTECODE='1'),
            check=True, timeout=10, capture_output=True)

    def test_import_and_public_attribute_resolution_start_no_connections_or_workers(self):
        self.probe('''import sys
def refuse(event,args):
 if event in ('socket.connect','socket.bind','subprocess.Popen'):raise AssertionError(event)
sys.addaudithook(refuse)
import connect_sdk
assert 'connect_sdk.sdk_sync' not in sys.modules
assert 'connect_sdk.native_audio' not in sys.modules
for name in connect_sdk.__all__:assert getattr(connect_sdk,name) is not None
''')

    def test_http_worker_does_not_import_unrelated_sdk_transports_and_controllers(self):
        self.probe('''import sys
import connect_sdk._sdk_http_worker
assert not {'connect_sdk.sdk_sync','connect_sdk.sdk_sessions','connect_sdk.sdk_connections',
 'connect_sdk.sdk_administration','connect_sdk.sdk_media','connect_sdk.native_audio',
 'connect_sdk.native','connect_sdk.http'}.intersection(sys.modules)
''')

    def test_public_exports_keep_identity_caching_star_import_and_missing_attribute_refusal(self):
        self.probe('''import connect_sdk
from connect_sdk import Client,CanonicalHttpClient,SdkError,NativeClient
from connect_sdk.client import Client as OriginalClient
from connect_sdk.sdk_http import CanonicalHttpClient as OriginalHttp
assert Client is OriginalClient and CanonicalHttpClient is OriginalHttp
assert connect_sdk.Client is Client and connect_sdk.CanonicalHttpClient is CanonicalHttpClient
assert set(connect_sdk.__all__).issubset(dir(connect_sdk))
namespace={};exec('from connect_sdk import *',namespace)
assert all(namespace[name] is getattr(connect_sdk,name) for name in connect_sdk.__all__)
try:connect_sdk.DoesNotExist
except AttributeError:pass
else:raise AssertionError('Unknown public export was fabricated')
''')
