"""Materialize protected server/operator configuration from a matching stopped backup.

Only the production CLI restores database state. This stage neither starts a
listener nor grants a matrix cell a pass. Browser enrollment, workload, capture,
media and actual topology still require the cell's runtime producer.
"""
import datetime
import ipaddress
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import stat
import subprocess
import tomllib
from urllib.parse import urlsplit

from .transport_matrix import digest, inventory

MAX_CONFIG = 1024 * 1024
MAX_SEED_BYTES = 512 * 1024 * 1024  # Synthetic fixture bound, not a product quota.
RELIABLE_PAYLOAD_BYTES = 64 * 1024  # The declared native workload's decoded payload.
ASSET = re.compile(r"(?:server\.toml|operator\.toml|state\.sqlite3|asset-\d{4}\.bin|peer-\d{4}\.toml)")
PROVIDER_FIELDS = ("telephone", "public_vapi", "provider_voice", "provider_messages", "mcp")


class FixtureError(ValueError):
    """Fixed codes only; private configuration must not enter error messages."""


def private_file(path, limit=MAX_CONFIG):
    value = path.lstat()
    if not stat.S_ISREG(value.st_mode) or value.st_mode & 0o077 or value.st_size > limit:
        raise FixtureError("fixture-private-file")
    return path.read_bytes()


def owned_path(root, path):
    path = Path(path)
    if path.is_symlink() or not path.resolve().is_relative_to((root / ".local").resolve()):
        raise FixtureError("fixture-owned-path")
    return path.resolve()


def endpoint(value, local):
    try:
        parsed = urlsplit("quic://" + value)
        address, port = ipaddress.ip_address(parsed.hostname), parsed.port
        if parsed.username or parsed.password or parsed.path or parsed.query or parsed.fragment:
            raise ValueError()
        if port is None or not 1 <= port <= 65535 or address.is_unspecified or address.is_multicast:
            raise ValueError()
        if local and not address.is_loopback:
            raise ValueError()
        if not local and (not address.is_private or address.is_link_local or (address.is_reserved and not address.is_loopback)):
            raise ValueError()
    except (ValueError, TypeError):
        raise FixtureError("fixture-endpoint") from None


def timestamp(value):
    try:
        date = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
        if date.tzinfo is None:
            raise ValueError()
        return date
    except (TypeError, ValueError, AttributeError):
        raise FixtureError("fixture-key-validity") from None


def workload_capacity(root, server, workload):
    limits = server.get("limits")
    if not isinstance(limits, dict):
        raise FixtureError("fixture-workload-capacity")
    effective = dict(limits)
    if "sessions" not in effective:
        # Current Limits uses serde(default=default_sessions) and omits that
        # value on serialization. Resolve its pinned source definition without
        # rewriting the seed or inventing a second default in fixture tooling.
        source = (root / "crates/connect-protocol/src/lib.rs").read_text()
        if not re.search(r'#\[serde\(\s*default = "default_sessions",\s*skip_serializing_if = "is_default_sessions"\s*\)\]\s*pub sessions: u64,', source):
            raise FixtureError("fixture-session-default-contract")
        definition = re.search(r"fn default_sessions\(\) -> u64\s*\{\s*([0-9_]+)\s*\}", source)
        if definition is None:
            raise FixtureError("fixture-session-default-contract")
        effective["sessions"] = int(definition[1].replace("_", ""))
    for key, count in (("participants", workload["participants"]),
            ("connections", workload["connections"]), ("sessions", workload["concurrent_sessions"])):
        if type(effective.get(key)) is not int or effective[key] < count:
            raise FixtureError("fixture-workload-capacity")
    if type(effective.get("message_bytes")) is not int or effective["message_bytes"] < RELIABLE_PAYLOAD_BYTES:
        raise FixtureError("fixture-workload-message-limit")


def inspect_seed(root, seed, parameters):
    """Validate private preparation inputs; no runtime authority is inferred."""
    fields = {"cell_id", "candidate_source_revision", "candidate_binary_sha256", "seed_manifest_sha256", "topology", "browser_origin"}
    if (not isinstance(parameters, dict) or set(parameters) != fields
            or not isinstance(parameters["candidate_source_revision"], str)
            or not re.fullmatch(r"[0-9a-f]{40}", parameters["candidate_source_revision"])):
        raise FixtureError("fixture-parameters")
    for name in ("candidate_binary_sha256", "seed_manifest_sha256"):
        if not isinstance(parameters[name], str) or not re.fullmatch(r"[0-9a-f]{64}", parameters[name]):
            raise FixtureError("fixture-digest")
    matrix = inventory(root, parameters["candidate_source_revision"])
    matches = [c for c in matrix["cells"] if c["id"] == parameters["cell_id"]]
    if len(matches) != 1:
        raise FixtureError("fixture-cell")
    cell = matches[0]
    seed = owned_path(root, seed)
    if not seed.is_dir() or seed.stat().st_mode & 0o077:
        raise FixtureError("fixture-seed-directory")
    try:
        manifest = json.loads(private_file(seed / "manifest.json"))
        server = tomllib.loads(private_file(seed / "server.toml").decode())
        operator = tomllib.loads(private_file(seed / "operator.toml").decode())
    except (ValueError, UnicodeError, OSError):
        raise FixtureError("fixture-seed-config") from None
    if digest(seed / "manifest.json") != parameters["seed_manifest_sha256"] or manifest.get("format") != "connect/canonical-backup/1":
        raise FixtureError("fixture-seed-manifest")
    schema = int(re.search(r"pub const SCHEMA_VERSION: u32 = (\d+);", (root / "crates/connect-store/src/lib.rs").read_text())[1])
    if manifest.get("snapshot", {}).get("schema") != schema:
        raise FixtureError("fixture-seed-schema")
    files, blobs = manifest.get("files"), manifest.get("blobs")
    if not isinstance(files, dict) or not isinstance(blobs, dict) or len(files) + len(blobs) > 4096:
        raise FixtureError("fixture-seed-inventory")
    if not {"server.toml", "operator.toml", "state.sqlite3"} <= files.keys():
        raise FixtureError("fixture-seed-inventory")
    bound = {"manifest.json": parameters["seed_manifest_sha256"]}
    total = 0
    for name, expected in files.items():
        if not ASSET.fullmatch(name) or not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
            raise FixtureError("fixture-seed-asset")
        path = seed / name
        value = path.lstat()
        if not stat.S_ISREG(value.st_mode) or value.st_mode & 0o077:
            raise FixtureError("fixture-private-file")
        total += value.st_size
        bound[name] = expected
    if blobs:
        directory = seed / "blobs"
        if directory.is_symlink() or not directory.is_dir() or directory.stat().st_mode & 0o077:
            raise FixtureError("fixture-seed-blobs")
    for name, expected in blobs.items():
        if not re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\.blob", name):
            raise FixtureError("fixture-seed-blob")
        if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
            raise FixtureError("fixture-seed-blob")
        path = seed / "blobs" / name
        value = path.lstat()
        if not stat.S_ISREG(value.st_mode) or value.st_mode & 0o077:
            raise FixtureError("fixture-private-file")
        total += value.st_size
        bound["blobs/" + name] = expected
    if total > MAX_SEED_BYTES or any(digest(seed / name) != expected for name, expected in bound.items()):
        raise FixtureError("fixture-seed-integrity")
    for name in PROVIDER_FIELDS:
        if server.get(name) is not None:
            raise FixtureError("fixture-external-component")
    profile = "local" if cell["network"] == "local" else "remote"
    if server.get("network_profile", "local") != profile or operator.get("network_profile", "local") != profile:
        raise FixtureError("fixture-network-profile")
    if (server.get("signing_algorithm") != cell["signing"]
            or operator.get("server_algorithm") != cell["signing"]
            or operator.get("signing_algorithm") != cell["signing"]):
        raise FixtureError("fixture-signing-profile")
    if server.get("signing_key_id") != operator.get("server_key_id"):
        raise FixtureError("fixture-server-pin")
    for server_key, client_key in (("signing_key_not_before", "server_key_not_before"), ("signing_key_expires_at", "server_key_expires_at")):
        if timestamp(server[server_key]) != timestamp(operator[client_key]):
            raise FixtureError("fixture-server-pin")
    now = datetime.datetime.now(datetime.timezone.utc)
    if not timestamp(server["signing_key_not_before"]) <= now < timestamp(server["signing_key_expires_at"]):
        raise FixtureError("fixture-expired-server-pin")
    if server.get("database") != "state.sqlite3":
        raise FixtureError("fixture-portable-database")
    for config, names in ((server, ("certificate", "private_key", "signing_private_key")),
            (operator, ("trust_certificate", "signing_private_key", "server_public_key"))):
        for name in names:
            if config.get(name) not in files:
                raise FixtureError("fixture-portable-asset")
    if files[server["signing_private_key"]] in {files[server["private_key"]], files[operator["signing_private_key"]]}:
        raise FixtureError("fixture-distinct-signing-domains")
    topology = parameters["topology"]
    topology_fields = {"kind", "server_platform", "client_platform", "client_emulated", "native_endpoint", "server_name"}
    if not isinstance(topology, dict) or set(topology) != topology_fields:
        raise FixtureError("fixture-topology")
    kind = "local-loopback" if profile == "local" else ("macos-linux-guest" if cell["server"] == "macos-arm64" else "ubuntu-netns")
    if (topology["kind"] != kind or topology["server_platform"] != cell["server"]
            or topology["client_platform"] != cell["expected_client_platform"]
            or type(topology["client_emulated"]) is not bool
            or topology["client_emulated"] != (kind == "macos-linux-guest")):
        raise FixtureError("fixture-topology")
    if topology["native_endpoint"] != operator.get("endpoint") or topology["server_name"] != operator.get("server_name"):
        raise FixtureError("fixture-provisioned-contact")
    endpoint(server["bind"], profile == "local")
    endpoint(topology["native_endpoint"], profile == "local")
    workload = json.loads((root / "contracts/gate6-transport-matrix.json").read_text())["workload"]
    workload_capacity(root, server, workload)
    if cell["transport"] != "native-quic":
        origin = parameters["browser_origin"]
        if not isinstance(origin, str) or any(c.isspace() for c in origin):
            raise FixtureError("fixture-browser-origin")
        try:
            url = urlsplit(origin)
            if url.port is not None and not 1 <= url.port <= 65535:
                raise ValueError()
        except ValueError:
            raise FixtureError("fixture-browser-origin") from None
        if (url.scheme != "https" or not url.hostname or url.username or url.password
                or url.path or url.query or url.fragment or origin != "https://" + url.netloc):
            raise FixtureError("fixture-browser-origin")
        for name in ("webtransport", "websocket"):
            config = server.get(name)
            if not isinstance(config, dict) or config.get("mount_path") != "/connect" or config.get("allowed_origins") != [origin]:
                raise FixtureError("fixture-browser-transports")
        if not isinstance(server.get("webrtc"), dict) or not server["webrtc"]:
            raise FixtureError("fixture-browser-media-required")
        applications = [c for c in server.get("https", []) if c.get("surface") == "application"]
        if len(applications) != 1 or applications[0].get("allowed_origins") != [origin]:
            raise FixtureError("fixture-browser-enrollment-https-required")
    elif parameters["browser_origin"] is not None:
        raise FixtureError("fixture-native-origin")
    return dict(cell=cell, seed=seed, seed_files=bound, server=server, operator=operator,
        service_contract_sha256=matrix["service_contract_sha256"])


def toml_bytes(value):
    """Closed ordinary TOML data; no string substitution into executable syntax."""
    lines = []

    def scalar(item):
        if isinstance(item, str):
            return json.dumps(item, ensure_ascii=False)
        if type(item) is bool:
            return "true" if item else "false"
        if type(item) is int:
            return str(item)
        if type(item) is float and math.isfinite(item):
            return repr(item)
        if isinstance(item, list) and not any(isinstance(v, dict) for v in item):
            return "[" + ", ".join(scalar(v) for v in item) + "]"
        raise FixtureError("fixture-toml-value")

    def table(data, prefix=(), array=False, depth=0):
        if depth > 16 or not isinstance(data, dict) or len(data) > 4096:
            raise FixtureError("fixture-toml-shape")
        if prefix:
            name = ".".join(json.dumps(p) for p in prefix)
            lines.append(("[[" + name + "]]" if array else "[" + name + "]"))
        for key, item in data.items():
            if not isinstance(key, str) or not 1 <= len(key) <= 128:
                raise FixtureError("fixture-toml-key")
            if item is None or isinstance(item, dict) or (isinstance(item, list) and item and all(isinstance(v, dict) for v in item)):
                continue
            lines.append(json.dumps(key) + " = " + scalar(item))
        for key, item in data.items():
            if isinstance(item, dict):
                lines.append("")
                table(item, (*prefix, key), depth=depth + 1)
            elif isinstance(item, list) and item and all(isinstance(v, dict) for v in item):
                for entry in item:
                    lines.append("")
                    table(entry, (*prefix, key), array=True, depth=depth + 1)

    table(value)
    encoded = ("\n".join(lines) + "\n").encode()
    if len(encoded) > MAX_CONFIG:
        raise FixtureError("fixture-config-capacity")
    tomllib.loads(encoded.decode())
    return encoded


def publish(path, content):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        os.fchmod(stream.fileno(), 0o600)
        stream.write(content)
        stream.flush()
        os.fsync(stream.fileno())


def materialize(root, seed, output, parameters, binary, environment):
    plan = inspect_seed(root, seed, parameters)
    host = (platform.system(), platform.machine())
    expected = ("Darwin", "arm64") if plan["cell"]["server"] == "macos-arm64" else ("Linux", "x86_64")
    if host != expected:
        raise FixtureError("fixture-server-platform")
    if host[0] == "Linux":
        release = platform.freedesktop_os_release()
        if release.get("ID") != "ubuntu" or release.get("VERSION_ID") != "24.04":
            raise FixtureError("fixture-server-platform")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    if revision != parameters["candidate_source_revision"]:
        raise FixtureError("fixture-candidate-source")
    binary = Path(binary)
    if binary.is_symlink() or not binary.is_file() or not binary.stat().st_mode & 0o111 or digest(binary) != parameters["candidate_binary_sha256"]:
        raise FixtureError("fixture-candidate-binary")
    output = owned_path(root, output)
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    output.chmod(0o700)
    state = output / "state"
    with (output / "restore.log").open("xb") as log:
        os.fchmod(log.fileno(), 0o600)
        child = subprocess.Popen([str(binary), "restore", "--backup", str(plan["seed"]), "--directory", str(state)],
            cwd=root, env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        joined = False
        try:
            code = child.wait(timeout=30)
            joined = True
        finally:
            if not joined:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
    if code != 0:
        raise FixtureError("fixture-public-restore-refused")
    for name, expected in plan["seed_files"].items():
        if name == "manifest.json":
            continue
        # Public restore relocates backup blobs to Store::blob_directory_for
        # (database.with_extension("blobs")); portable backup layout differs.
        path = state / ("state.blobs/" + name.removeprefix("blobs/") if name.startswith("blobs/") else name)
        value = path.lstat()
        if not stat.S_ISREG(value.st_mode) or value.st_mode & 0o077 or value.st_size == 0:
            raise FixtureError("fixture-restored-asset")
        if name != "state.sqlite3" and digest(path) != expected:
            raise FixtureError("fixture-restored-asset")
    if any(digest(plan["seed"] / name) != expected for name, expected in plan["seed_files"].items()):
        raise FixtureError("fixture-seed-changed")
    if digest(binary) != parameters["candidate_binary_sha256"]:
        raise FixtureError("fixture-candidate-binary-changed")
    server, operator = dict(plan["server"]), dict(plan["operator"])
    for key in ("database", "certificate", "private_key", "signing_private_key"):
        server[key] = str(state / server[key])
    for key in ("trust_certificate", "signing_private_key", "server_public_key"):
        operator[key] = str(state / operator[key])
    publish(output / "server.toml", toml_bytes(server))
    publish(output / "operator.toml", toml_bytes(operator))
    summary = dict(format="connect/transport-fixture-materialization/1", cell_id=parameters["cell_id"],
        candidate_source_revision=revision, candidate_binary_sha256=parameters["candidate_binary_sha256"],
        seed_manifest_sha256=parameters["seed_manifest_sha256"], service_contract_sha256=plan["service_contract_sha256"],
        server_config_sha256=digest(output / "server.toml"), operator_config_sha256=digest(output / "operator.toml"),
        production_restore_joined=True, server_configuration_materialized=True,
        participant_setup_complete=False, actual_topology_verified=False, runtime_authority_verified=False,
        packet_capture_observed=False, decoded_media_observed=False,
        cell_qualified=False, task_complete=False, whole_gate_qualified=False)
    publish(output / "materialized.json", (json.dumps(summary, indent=2) + "\n").encode())
    directory = os.open(output, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)
    return summary
