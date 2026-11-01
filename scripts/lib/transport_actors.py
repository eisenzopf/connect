"""Public-CLI enrollment for matrix actors; no private Store or media access."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib
import uuid

from .transport_fixture import FixtureError, endpoint, owned_path, private_file, publish, timestamp
from .transport_matrix import digest

OPERATIONS = sorted({
    "resolve", "create", "conversation.list", "conversation.get", "conversation.subscribe",
    "conversation.participant.add", "conversation.participant.update", "conversation.participant.remove",
    "send", "history", "inbox", "operation.get", "operation.cancel", "health", "session_get",
    "session.observation", "session.invite", "session.accept", "session.reject", "session.cancel",
    "session.end", "session.update", "connection.offer", "connection.update", "connection.replace",
    "connection.end", "stream.subscribe", "capability.advertise",
})
ROLES = tuple(f"participant-{n}" for n in range(1, 5))
DEVICES = (*ROLES, "replacement-1")


def encoded(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def exact(path, content):
    if path.exists() or path.is_symlink():
        if private_file(path) != content:
            raise FixtureError("fixture-actor-intent-changed")
    else:
        publish(path, content)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)


def new_plan(binding, expires_at):
    prefix = "g6-058-" + uuid.uuid4().hex
    return dict(format="connect/transport-actor-plan/1", binding=binding, expires_at=expires_at,
        identities=[dict(role=role, alias=prefix + "-" + role, operation_id="identity_" + str(uuid.uuid4())) for role in ROLES],
        devices=[dict(name=name, role="participant-1" if name == "replacement-1" else name,
            operation_id="credential_" + str(uuid.uuid4())) for name in DEVICES])


def validate_plan(plan, binding, ceiling, now):
    if (not isinstance(plan, dict) or set(plan) != {"format", "binding", "expires_at", "identities", "devices"}
            or plan["format"] != "connect/transport-actor-plan/1" or plan["binding"] != binding
            or type(plan["expires_at"]) is not int or not now < plan["expires_at"] <= ceiling
            or not isinstance(plan["identities"], list) or len(plan["identities"]) != 4
            or not isinstance(plan["devices"], list) or len(plan["devices"]) != 5):
        raise FixtureError("fixture-actor-plan")
    seen = set()
    prefix = None
    for role, item in zip(ROLES, plan["identities"]):
        if not isinstance(item, dict) or set(item) != {"role", "alias", "operation_id"} or item["role"] != role:
            raise FixtureError("fixture-actor-identities")
        alias = item["alias"]
        if not isinstance(alias, str) or not alias.endswith("-" + role):
            raise FixtureError("fixture-actor-alias")
        actual = alias.removesuffix("-" + role)
        try:
            assert actual.startswith("g6-058-") and uuid.UUID(hex=actual.removeprefix("g6-058-")).hex == actual.removeprefix("g6-058-")
        except (ValueError, AssertionError):
            raise FixtureError("fixture-actor-alias") from None
        prefix = actual if prefix is None else prefix
        if prefix != actual:
            raise FixtureError("fixture-actor-alias")
    for name, item in zip(DEVICES, plan["devices"]):
        if (not isinstance(item, dict) or set(item) != {"name", "role", "operation_id"}
                or item["name"] != name or item["role"] != ("participant-1" if name == "replacement-1" else name)):
            raise FixtureError("fixture-actor-devices")
    for kind, items in (("identity", plan["identities"]), ("credential", plan["devices"])):
        for item in items:
            value = item["operation_id"]
            try:
                assert isinstance(value, str) and value.startswith(kind + "_")
                assert str(uuid.UUID(value.removeprefix(kind + "_"))) == value.removeprefix(kind + "_")
                assert value not in seen
            except (ValueError, AssertionError):
                raise FixtureError("fixture-actor-operation-id") from None
            seen.add(value)


def installation_files(path):
    result = {}
    for value in sorted(path.iterdir()):
        if value.is_dir() or value.is_symlink():
            raise FixtureError("fixture-actor-installation-shape")
        private_file(value)
        result[value.name] = digest(value)
    return result


def enroll_actors(root, instance, admin_path, binary, output, execute, *, resume=False):
    """Caller owns the running instance. execute invokes only the named public CLI.

    Enrollment itself does not create Conversation participants, sessions, media
    owners or a workload. Those observations belong to the full cell executor.
    """
    instance, admin_path, output = (owned_path(root, p) for p in (instance, admin_path, output))
    binary = Path(binary)
    if binary.is_symlink() or not binary.is_file() or not binary.stat().st_mode & 0o111:
        raise FixtureError("fixture-actor-binary")
    summary = json.loads(private_file(instance / "materialized.json"))
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    if (summary.get("format") != "connect/transport-fixture-materialization/1"
            or summary.get("candidate_source_revision") != revision
            or summary.get("candidate_binary_sha256") != digest(binary)
            or not summary.get("server_configuration_materialized")
            or summary.get("cell_qualified") is not False):
        raise FixtureError("fixture-actor-instance")
    for name, field in (("server.toml", "server_config_sha256"), ("operator.toml", "operator_config_sha256")):
        if digest(instance / name) != summary[field]:
            raise FixtureError("fixture-actor-instance-changed")
    server = tomllib.loads(private_file(instance / "server.toml").decode())
    operator = tomllib.loads(private_file(instance / "operator.toml").decode())
    admin = tomllib.loads(private_file(admin_path).decode())
    endpoint(admin["endpoint"], True)
    if admin["endpoint"] != server["http_bind"] or admin["credential"] != operator["credential"]:
        raise FixtureError("fixture-actor-issuer")
    algorithm = operator["signing_algorithm"]
    if algorithm not in {"EdDSA", "ES256"} or algorithm != server["signing_algorithm"]:
        raise FixtureError("fixture-actor-signing")
    now = int(time.time())
    ceiling = min(int(timestamp(admin["expires_at"]).timestamp()), int(timestamp(operator["server_key_expires_at"]).timestamp()))
    binding = dict(cell_id=summary["cell_id"], source_revision=revision, binary_sha256=digest(binary),
        instance_sha256=digest(instance / "materialized.json"), admin_config_sha256=digest(admin_path),
        operations_sha256=hashlib.sha256(encoded(OPERATIONS)).hexdigest())
    output.mkdir(mode=0o700, parents=True, exist_ok=resume)
    if output.stat().st_mode & 0o077:
        raise FixtureError("fixture-actor-private-directory")
    lock = output / "actors.lock"
    if not resume:
        publish(lock, b"")
    private_file(lock)
    with lock.open("rb") as held:
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
        path = output / "plan.json"
        plan = json.loads(private_file(path)) if resume else new_plan(binding, min(now + 3600, ceiling))
        validate_plan(plan, binding, ceiling, now)
        exact(path, encoded(plan))  # Durable original IDs precede every mutation.
        identities = {}
        for item in plan["identities"]:
            command = dict(type="identity", operation_id=item["operation_id"], alias=item["alias"],
                kind="human", display_name="Synthetic " + item["role"])
            response = json.loads(execute(item["role"] + "-identity", ["provision", "--config", admin_path], command))
            if response.get("profile") != "connect/2" or set(response.get("result", {})) != {"Ok"}:
                raise FixtureError("fixture-actor-provisioning-profile")
            result = response["result"]["Ok"]
            if (result["kind"] != "identity" or result["alias"] != item["alias"]
                    or result["identity_kind"] != "human" or result["revoked"]):
                raise FixtureError("fixture-actor-identity-result")
            identities[item["role"]] = result["identity_id"]
        if len(set(identities.values())) != 4:
            raise FixtureError("fixture-actor-distinct-identities")
        exact(output / "identities.json", encoded(identities))
        installed = {}
        for item in plan["devices"]:
            name, role = item["name"], item["role"]
            request = dict(operation_id=item["operation_id"], identity_id=identities[role], operations=OPERATIONS,
                recipients=sorted(value for key, value in identities.items() if key != role), expires_at=plan["expires_at"])
            intent = output / (name + ".json")
            exact(intent, encoded(request))
            destination = output / name
            if destination.exists() or destination.is_symlink():
                if not resume or destination.is_symlink():
                    raise FixtureError("fixture-actor-existing-device")
                arguments = ["issue-resume", "--config", admin_path, "--installation", destination]
            else:
                arguments = ["issue", "--config", admin_path, "--native-template", instance / "operator.toml",
                    "--request", intent, "--output", destination, "--signing-algorithm", algorithm]
            execute(name + "-enrollment", arguments)
            issued = json.loads(private_file(destination / "issued.json"))
            native = tomllib.loads(private_file(destination / "client.toml").decode())
            if (issued["operation_id"] != request["operation_id"] or issued["identity_id"] != request["identity_id"]
                    or issued["expires_at"] != request["expires_at"] or issued["operations"] != request["operations"]
                    or issued["recipients"] != request["recipients"] or native["identity_id"] != request["identity_id"]
                    or native["credential"] != issued["credential"] or native["signing_algorithm"] != algorithm
                    or native["server_algorithm"] != algorithm or native["server_key_expires_at"] != operator["server_key_expires_at"]):
                raise FixtureError("fixture-actor-enrollment-result")
            assets = installation_files(destination)
            if any(operator["credential"].encode() in private_file(destination / name) for name in assets):
                raise FixtureError("fixture-actor-parent-credential")
            signer = digest(destination / native["signing_private_key"])
            if signer == digest(Path(operator["signing_private_key"])) or signer == digest(Path(server["signing_private_key"])):
                raise FixtureError("fixture-actor-shared-signer")
            installed[name] = dict(role=role, identity_id=native["identity_id"], device_id=native["device_id"],
                credential_id=issued["credential_id"], signing_key_id=native["signing_key_id"], signer_sha256=signer,
                config=str(destination / "client.toml"), assets=assets)
        for field in ("device_id", "credential_id", "signing_key_id", "signer_sha256"):
            if len({item[field] for item in installed.values()}) != 5:
                raise FixtureError("fixture-actor-distinct-devices")
        exact(output / "actors.json", encoded(installed))
        if digest(binary) != binding["binary_sha256"] or digest(admin_path) != binding["admin_config_sha256"]:
            raise FixtureError("fixture-actor-input-changed")
        result = dict(format="connect/transport-actor-enrollment/1", **binding,
            plan_sha256=digest(path), actors_sha256=digest(output / "actors.json"),
            identities_enrolled=4, devices_enrolled=5, independent_replacement_device=True,
            signing_algorithm=algorithm, scope_operations=OPERATIONS, recipient_ceiling=3,
            expires_at=plan["expires_at"], public_cli_enrollment_complete=True,
            conversation_participants_observed=False, concurrent_workload_observed=False,
            replacement_transition_observed=False, actual_topology_verified=False,
            cell_qualified=False, task_complete=False, whole_gate_qualified=False)
        exact(output / "enrollment.json", encoded(result))
        return result
