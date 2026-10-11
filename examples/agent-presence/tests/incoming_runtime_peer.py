"""Scripted local Responses peer; drives real installed tools without inference."""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import threading

TOOLS = (Path(__file__).resolve().parents[3] / "crates/connect-server/tests/fixtures/codex-workflow-tools.js").read_text()
PROGRAM = TOOLS + '''
const task = await invoke('connect_task_status', {});
const recall = await invoke('connect_recall', {limit: 100});
const provenance = 'conversation-journal:' + task.data.initiating_event_id;
const source = recall.data.sources.find(s => s.provenance.split(';')[0] === provenance);
if (!source || source.state !== 'available') throw new Error('Incoming source unavailable');
const contact = JSON.parse(source.body);
const result = await invoke('connect_send', {
  request_id: 'incoming-answer', step_id: 'answer', wait_for_reply: false,
  reply_to: task.data.initiating_event_id,
  body: 'Received from ' + source.source_id + ': ' + contact.body,
});
text(JSON.stringify({task, source, result}));
'''


def executable(tools, namespace=None):
    for tool in tools:
        if tool.get("name") == "exec" and tool.get("type") == "custom":
            return tool["name"], namespace
        if tool.get("type") == "namespace":
            found = executable(tool["tools"], tool["name"])
            if found:
                return found
    return None


class Peer:
    def __init__(self):
        self.requests, self.errors = [], []
        peer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    assert self.path == "/responses"
                    size = int(self.headers["Content-Length"])
                    assert size <= 2 * 1024 * 1024
                    request = json.loads(self.rfile.read(size))
                    index = len(peer.requests)
                    peer.requests.append(request)
                    assert index < 2, "Unexpected model request or retry"
                    if index == 0:
                        name, namespace = next(executable(item["tools"]) for item in request["input"]
                                               if item["type"] == "additional_tools" and executable(item["tools"]))
                        item = {"type": "custom_tool_call", "call_id": "incoming-source-answer", "name": name, "input": PROGRAM}
                        if namespace:
                            item["namespace"] = namespace
                    else:
                        assert any(item.get("call_id") == "incoming-source-answer" and item["type"] == "custom_tool_call_output"
                                   for item in request["input"])
                        item = {"type": "message", "role": "assistant", "id": "incoming-final",
                                "content": [{"type": "output_text", "text": "Scripted local incoming-contact checkpoint."}]}
                    response_id = "incoming-" + str(index)
                    events = [{"type": "response.created", "response": {"id": response_id}},
                              {"type": "response.output_item.done", "item": item},
                              {"type": "response.completed", "response": {"id": response_id,
                               "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}}}]
                    payload = "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in events).encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)
                except BaseException as error:
                    peer.errors.append(repr(error))
                    self.send_error(500)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)
        assert not self.thread.is_alive()
