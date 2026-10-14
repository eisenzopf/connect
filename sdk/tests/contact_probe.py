"""Source SDK contact parity against a disposable loopback server."""
import json
import os
import sys
import tempfile
import time
from contextlib import ExitStack, contextmanager
from connect_sdk import NativeClient, HttpClient, ConnectError

owner_path, peer_path, url, helper = sys.argv[1:]
credentials = json.load(sys.stdin)  # Credentials and challenges never enter argv.
secrets = [*credentials.values(), "private-challenge-wrong"]

@contextmanager
def private_diagnostics():
    # Capture the actual helper's inherited stderr, including verbose tracing.
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
            assert not any(secret in text for secret in secrets), "Helper leaked a private value"

with private_diagnostics(), ExitStack() as stack:
    owner_native = stack.enter_context(NativeClient(owner_path, helper))
    peer_native = stack.enter_context(NativeClient(peer_path, helper))
    owner_http = HttpClient(url, credentials["owner"])
    peer_http = HttpClient(url, credentials["peer"])
    peer_id = owner_native.resolve("connect:agent@local")["id"]
    for lane, owner, peer, retry_owner, retry_peer in [
        ("native", owner_native, peer_native, owner_http, peer_http),
        ("http", owner_http, peer_http, owner_native, peer_native),
    ]:
        prefix = "python-contact-" + lane
        expiry = int(time.time()) + 300
        pairing = owner.contact_pairing_start(prefix + "-start", peer_id, expiry)
        secrets.append(pairing["challenge"])
        assert pairing == retry_owner.contact_pairing_start(prefix + "-start", peer_id, expiry)
        assert pairing["state"] == "pending" and pairing.get("challenge")
        for method in [owner.contact_pairing_complete, retry_owner.contact_pairing_complete]:
            try:
                method(prefix + "-wrong-actor", pairing["id"], pairing["challenge"])
                raise AssertionError("Owner presented peer challenge")
            except ConnectError as error:
                assert error.code == "forbidden"
        failed = peer.contact_pairing_complete(prefix + "-bad", pairing["id"], "private-challenge-wrong")
        assert failed == retry_peer.contact_pairing_complete(prefix + "-bad", pairing["id"], "private-challenge-wrong")
        assert failed["state"] == "pending" and "challenge" not in failed
        complete = peer.contact_pairing_complete(prefix + "-complete", pairing["id"], pairing["challenge"])
        assert complete == retry_peer.contact_pairing_complete(prefix + "-complete", pairing["id"], pairing["challenge"])
        assert complete["state"] == "verified_native" and "challenge" not in complete
        contact = owner.contact_state(peer_id)
        assert contact == retry_owner.contact_state(peer_id)
        assert contact["ownership"] == "verified_native" and contact["sms_consent"] is False
        assert contact["verified_at"] is not None
        revoked = owner.contact_pairing_revoke(prefix + "-revoke", pairing["id"])
        assert revoked == retry_owner.contact_pairing_revoke(prefix + "-revoke", pairing["id"])
        assert revoked["state"] == "revoked" and "challenge" not in revoked
        for client in [peer, retry_peer]:
            try:
                client.contact_pairing_complete(prefix + "-complete", pairing["id"], pairing["challenge"])
                raise AssertionError("Revoked completion replay accepted")
            except ConnectError as error:
                assert error.code == "forbidden"
        assert owner.contact_state(peer_id)["ownership"] == "revoked"
        assert retry_owner.contact_state(peer_id)["sms_consent"] is False
print("Python contact native-helper and HTTP source parity passed")
