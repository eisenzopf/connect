"""A bounded Responses simulator for concurrent actual Codex host turns."""
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from incoming_runtime_peer import PROGRAM, executable


class Peer:
    """Two scripted turns; a shared gate holds real hosts before their tool work."""

    def __init__(self):
        self.requests, self.errors = [], []
        self.gate = None
        self.tool = None
        peer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    assert self.path == '/responses'
                    size = int(self.headers['Content-Length'])
                    assert 0 < size <= 2 * 1024 * 1024
                    request = json.loads(self.rfile.read(size))
                    index = len(peer.requests)
                    peer.requests.append(request)
                    assert index < 4, 'Unexpected model request or automatic retry'
                    call_id = f'concurrent-source-answer-{index // 2}'
                    if index % 2 == 0:
                        advertised = [executable(item['tools']) for item in request['input']
                                      if item['type'] == 'additional_tools']
                        peer.tool = next((tool for tool in advertised if tool), peer.tool)
                        assert peer.tool is not None, 'Runtime did not advertise code execution'
                        barrier, release = peer.gate
                        barrier.wait(timeout=60)
                        assert release.wait(timeout=15), 'Qualification gate was not released'
                        name, namespace = peer.tool
                        item = {'type': 'custom_tool_call', 'call_id': call_id,
                                'name': name, 'input': PROGRAM}
                        if namespace:
                            item['namespace'] = namespace
                    else:
                        assert any(item.get('call_id') == call_id and item['type'] == 'custom_tool_call_output'
                                   for item in request['input']), 'Missing actual tool result'
                        item = {'type': 'message', 'role': 'assistant', 'id': f'profile-final-{index}',
                                'content': [{'type': 'output_text', 'text': 'Scripted concurrent checkpoint.'}]}
                    identity = f'profile-response-{index}'
                    events = [{'type': 'response.created', 'response': {'id': identity}},
                              {'type': 'response.output_item.done', 'item': item},
                              {'type': 'response.completed', 'response': {'id': identity,
                               'usage': {'input_tokens': 0, 'output_tokens': 0, 'total_tokens': 0}}}]
                    payload = ''.join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in events).encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('Content-Length', str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)
                except BaseException as error:
                    peer.errors.append(repr(error))
                    self.send_error(500)

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.server.server_port}'
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)
        assert not self.thread.is_alive(), 'Responses fixture did not stop'
