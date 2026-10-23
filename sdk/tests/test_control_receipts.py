"""Public HTTP send receipts carry an exact, queryable local mutation reference."""
import copy
import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from connect_sdk import HttpClient, ConnectError


class ControlReceipts(unittest.TestCase):
    def test_exact_reference_is_required_and_bad_reference_never_causes_a_retry(self):
        valid={"request_id":"logical-message","message_id":"message-one","conversation_id":"room-one","receipt":{"operation_id":"logical-message","conversation_id":"room-one","state":"completed","completion_boundary":"local-mutation","status_reference":{"action":"get","operation_id":"logical-message"}}}
        values=[copy.deepcopy(valid) for _ in range(5)]
        del values[1]["receipt"]
        values[2]["receipt"]["status_reference"]["operation_id"]="other-message"
        values[3]["receipt"]["status_reference"]["action"]="cancel"
        values[4]["receipt"]["completion_boundary"]="provider-delivered"
        requests=[]
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args): pass
            def do_POST(self):
                requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                raw=json.dumps({"profile":"connect-control/1","result":{"Ok":{"kind":"accepted","data":values[len(requests)-1]}}}).encode()
                self.send_response(200);self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            client=HttpClient(f'http://127.0.0.1:{server.server_port}','isolated-fixture')
            self.assertEqual(client.send('logical-message','room-one','message'),valid)
            for _ in values[1:]:
                with self.assertRaises(ConnectError) as error:client.send('logical-message','room-one','message')
                self.assertEqual(error.exception.code,'protocol')
            self.assertEqual(len(requests),5)
            self.assertEqual({r['request_id'] for r in requests},{'logical-message'})
        finally:server.shutdown();server.server_close();thread.join()
