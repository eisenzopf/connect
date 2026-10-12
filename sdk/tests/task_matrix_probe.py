"""Run the shared task matrix through the real Python public client."""
import json
import pathlib
import re
import sys
import tomllib
from connect_sdk import ConnectError, HttpClient, NativeClient


_MISSING = object()

def path(value, dotted, *, strict=True):
    for key in dotted.split("."):
        if isinstance(value, dict) and key not in value:
            if strict:
                raise KeyError(key)
            return _MISSING
        value = value[int(key)] if isinstance(value, list) else value[key]
    return value


def expand(value, bindings):
    if isinstance(value, dict):
        return {key: expand(item, bindings) for key, item in value.items()}
    if isinstance(value, list):
        return [expand(item, bindings) for item in value]
    if not isinstance(value, str):
        return value
    if re.fullmatch(r"\$\{[^}]+\}", value):
        return bindings[value[2:-1]]
    return re.sub(r"\$\{([^}]+)\}", lambda match: str(bindings[match[1]]), value)


def verify(result, expected):
    for field, value in expected.get("paths", {}).items():
        assert path(result, field) == value, field
    for field, value in expected.get("greater", {}).items():
        assert path(result, field) > value, field
    for check in expected.get("contains", []):
        matches = [item for item in path(result, check["path"])
                   if path(item, check["field"], strict=False) == check["value"]]
        if "count" in check:
            assert len(matches) == check["count"], check["path"]
        else:
            assert matches, check["path"]


def main():
    owner_path, peer_path, url, executable, mode = sys.argv[1:]
    bindings = json.load(sys.stdin)
    matrix = json.loads(pathlib.Path(__file__).with_name("task_matrix.json").read_text())
    configs = {"owner": owner_path, "peer": peer_path}

    def open_client(actor):
        if mode == "native":
            client = NativeClient(configs[actor], executable=executable)
        else:
            with open(configs[actor], "rb") as stream:
                client = HttpClient(url, tomllib.load(stream)["credential"])
        client.negotiate_tasks(
            "connect:agent@local" if actor == "owner" else "connect:reviews@local",
            bindings["cid"],
        )
        return client

    clients = {actor: open_client(actor) for actor in configs}
    try:
        for template in matrix["steps"]:
            step = expand(template, bindings)
            actor = step["actor"]
            if step.get("reconnect"):
                if mode == "native":
                    clients[actor].close()
                clients[actor] = open_client(actor)
            try:
                result = (clients[actor].task(step["command"]) if "command" in step
                          else clients[actor].request(step["operation"]))
            except ConnectError as error:
                assert error.code == step.get("error"), (step["id"], error.code)
                continue
            assert "error" not in step, step["id"]
            verify(result, step.get("expect", {}))
            for name, field in step.get("capture", {}).items():
                bindings[name] = path(result, field)
        print(f"Python {mode}: {len(matrix['steps'])} real-server task steps passed")
    finally:
        if mode == "native":
            for client in clients.values():
                client.close()


if __name__ == "__main__":
    main()
