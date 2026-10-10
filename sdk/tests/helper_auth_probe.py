"""Real Native SDK authentication classification; no parent credential supplied."""
import json
import sys
import time
from connect_sdk import ConnectError, NativeClient

valid_config, bad_config, absent_config, executable = sys.argv[1:]
data = json.load(sys.stdin)


def reject(fn, expected):
    try:
        fn()
    except ConnectError as error:
        assert error.code == expected, (expected, error.code)
    else:
        raise AssertionError("Expected typed rejection")


for path, expected in [(bad_config, "unauthorized"), (absent_config, "configuration")]:
    with NativeClient(path, executable=executable) as client:
        reject(lambda: client.negotiate_task(data["task"]), expected)
        reject(lambda: client.negotiate_task(data["task"]), expected)

with NativeClient(valid_config, executable=executable) as client:
    assert client.negotiate_task(data["task"])["id"] == data["task"]
    # Exercise expiry on a live peer, rather than QUIC's earlier idle timeout.
    while data["expires_at"] - time.time() > 5:
        time.sleep(5)
        if data["expires_at"] - time.time() > 5:
            result = client.task({"action": "get", "task_id": data["task"]})
            assert result["kind"] == "record" and result["data"]["id"] == data["task"]
    time.sleep(max(0, data["expires_at"] - time.time()) + 1)
    reject(lambda: client.task({"action": "get", "task_id": data["task"]}), "unauthorized")

with NativeClient(valid_config, executable=executable) as client:
    reject(lambda: client.negotiate_task(data["task"]), "unauthorized")
print(json.dumps({"status": "ok", "runtime": "python"}))
