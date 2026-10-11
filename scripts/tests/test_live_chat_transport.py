"""Actual local HTTP and independent worker process; no provider credentials."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "live"))
import chat_transport


class TransportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.calls, self.status, self.body = [], 200, {"id": "chat-id"}
        test = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                test.calls.append((self.command, self.path, self.headers.get("Authorization")))
                data = test.body if isinstance(test.body, bytes) else json.dumps(test.body).encode()
                self.send_response(test.status)
                self.send_header("Content-Type", "application/json")
                if test.status == 302:self.send_header("Location", "/must-not-follow")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                try:self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):pass

            def do_POST(self):
                size = int(self.headers.get("Content-Length", 0))
                test.posted = json.loads(self.rfile.read(size))
                self.do_GET()

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.shutdown)
        self.base = f"http://127.0.0.1:{self.server.server_port}"
        previous = os.umask(0o077)
        self.addCleanup(os.umask, previous)

    def shutdown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.assertFalse(self.thread.is_alive())

    def request(self, method="GET", route="/session/session-id", payload=None):
        with patch.dict(os.environ, {"VAPI_PRIVATE_KEY": "fixture-private-key"}):
            return chat_transport.request(self.base, method, route, payload, self.root, "provider")

    def test_response_is_private_and_exact_request_is_sent_once(self):
        body = {"sessionId": "session-id", "input": "Explain the recorded decision", "name": "logical-turn"}
        self.assertEqual(self.request("POST", "/chat", body), self.body)
        self.assertEqual(self.posted, body)
        self.assertEqual(self.calls, [("POST", "/chat", "Bearer fixture-private-key")])
        for path in self.root.glob("*.json"):
            self.assertEqual(path.stat().st_mode & 0o077, 0)
            self.assertNotIn("fixture-private-key", path.read_text())
        with self.assertRaises(ValueError):self.request("POST", "/chat", body)
        self.assertEqual(len(self.calls), 1)

    def test_redirect_is_not_followed_with_the_key(self):
        self.status = 302
        with self.assertRaises(RuntimeError):self.request()
        self.assertEqual(len(self.calls), 1)
        result = json.loads((self.root / "provider-response.json").read_text())
        self.assertEqual(result["http_status"], 302)

    def test_oversized_response_is_rejected(self):
        self.body = b"a" * (chat_transport.LIMIT + 1)
        with self.assertRaises(RuntimeError):self.request()
        result = json.loads((self.root / "provider-response.json").read_text())
        self.assertEqual(result["error_class"], "ValueError")
        self.assertNotIn("body", result)

    def test_invalid_json_cannot_become_a_receipt(self):
        self.body = b"not JSON"
        with self.assertRaises(RuntimeError):self.request()
        self.assertEqual(len(self.calls), 1)

    def test_absolute_worker_timeout_kills_and_reaps_before_return(self):
        worker = self.root / "stalled.py"
        worker.write_text("import time\ntime.sleep(60)\n")
        actual = subprocess.run
        actual_popen = subprocess.Popen
        children = []

        def observe(*args, **kwargs):
            child = actual_popen(*args, **kwargs)
            children.append(child)
            return child

        def bounded(*args, **kwargs):
            self.assertEqual(kwargs["timeout"], 40)
            kwargs["timeout"] = 0.3  # Execute the same timeout/reaping path quickly.
            return actual(*args, **kwargs)

        with patch.object(chat_transport, "__file__", str(worker)), \
             patch.object(chat_transport.subprocess, "run", side_effect=bounded), \
             patch.object(chat_transport.subprocess, "Popen", side_effect=observe), \
             self.assertRaises(subprocess.TimeoutExpired):
            self.request()
        self.assertEqual(len(children), 1)
        self.assertIsNotNone(children[0].returncode, "Worker was not reaped before timeout returned")
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
