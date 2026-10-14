"""Actual public audio+video Session methods, native helper and HTTP."""
import json
import os
import sys
import tempfile
from contextlib import ExitStack, contextmanager
from connect_sdk import NativeClient, HttpClient, ConnectError

owner_path, peer_path, url, helper = sys.argv[1:]
inputs = json.load(sys.stdin)

@contextmanager
def private_diagnostics():
    saved = os.dup(2)
    with tempfile.TemporaryFile() as diagnostics:
        os.dup2(diagnostics.fileno(), 2)
        try:
            yield
        finally:
            os.dup2(saved, 2)
            os.close(saved)
            diagnostics.seek(0)
            text = diagnostics.read().decode("utf-8", errors="replace")
            assert all(inputs[actor] not in text for actor in ["owner", "peer"])

def invoke(client, operation):
    args = {key: value for key, value in operation.items() if key != "op"}
    # These are the public typed convenience methods, not the generic request API.
    return getattr(client, operation["op"])(**args)

with private_diagnostics(), ExitStack() as stack:
    native = {"owner": stack.enter_context(NativeClient(owner_path, helper)),
              "peer": stack.enter_context(NativeClient(peer_path, helper))}
    http = {actor: HttpClient(url, inputs[actor]) for actor in ["owner", "peer"]}
    for clients in [native, http]:
        for record in inputs["records"]:
            try:
                actual = invoke(clients[record["actor"]], record["operation"])
                assert "error" not in record, "Expected denied Session operation"
                assert actual == record["outcome"]["data"]
            except ConnectError as error:
                assert error.code == record.get("error")
    generated = []
    cid, participant = inputs["conversation_id"], inputs["participant_id"]
    for lane, clients, retries in ([] if inputs.get("replay_only") else [("native", native, http), ("http", http, native)]):
        prefix = "python-av-" + lane
        def execute(actor, operation):
            actual = invoke(clients[actor], operation)
            assert actual == invoke(retries[actor], operation)
            assert actual["media"] == ["audio", "video"]
            generated.append({"actor": actor, "operation": operation,
                              "outcome": {"kind": "session", "data": actual}})
            return actual
        invite = execute("owner", dict(op="session_invite", request_id=prefix+"-invite",
                         conversation_id=cid, participant_id=participant, media=["audio", "video"]))
        assert invite["state"] == "invited"
        accepted = execute("peer", dict(op="session_accept", request_id=prefix+"-accept",
                           conversation_id=cid, session_id=invite["id"]))
        assert accepted["state"] == "accepted"
        ended = execute("owner", dict(op="session_end", request_id=prefix+"-end",
                        conversation_id=cid, session_id=invite["id"], reason="SDK video complete"))
        assert ended["state"] == "ended"
        assert execute("peer", dict(op="session_get", conversation_id=cid, session_id=invite["id"])) == ended
print(json.dumps(generated))
