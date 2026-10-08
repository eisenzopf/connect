import sys
import tomllib
from connect_sdk import NativeClient, HttpClient, ConnectError

path, url, executable = sys.argv[1:]
with open(path, "rb") as file:
    config = tomllib.load(file)
http = HttpClient(url, config["credential"])
with NativeClient(path, executable) as native:
    assert native.health() == http.health()
    peer = native.resolve("connect:agent@local")
    conversation = native.create("python-create", [peer["id"]])
    assert conversation == http.create("python-create", [peer["id"]])
    receipt = native.send("python-send", conversation["id"], "Python over QUIC")
    assert receipt == http.send("python-send", conversation["id"], "Python over QUIC")
    assert native.history(conversation["id"]) == http.history(conversation["id"])
    assert len(http.history(conversation["id"])["messages"]) == 1
    assert http.request_status("python-send")["state"] == "accepted"
    assert native.provider_status(conversation["id"])["deliveries"] == []
    try:
        http.send("python-send", conversation["id"], "changed")
        raise AssertionError("Conflict was not rejected")
    except ConnectError as error:
        assert error.code == "conflict"
    try:
        HttpClient(url, "invalid").health()
        raise AssertionError("Bad auth was accepted")
    except ConnectError as error:
        assert error.code == "unauthorized"
print("Python native QUIC and HTTP SDK parity passed")
