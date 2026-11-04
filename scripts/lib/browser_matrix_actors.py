"""Export only a selected synthetic CLI installation to the private browser host.

This grants no authority, provisions no actor and never writes public key material
or credentials to an evidence report. Actual browser authentication is required.
"""
import base64
import datetime
import hashlib
import json
from pathlib import Path
import tomllib

from .transport_fixture import FixtureError, private_file


def _base64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def _milliseconds(value):
    when = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
    if when.tzinfo is None:
        raise FixtureError("browser-fixture-server-validity")
    return int(when.timestamp() * 1000)


def browser_actor(installation, *, scope, algorithm):
    installation, scope = Path(installation), Path(scope).resolve(strict=True)
    if installation.is_symlink() or not installation.resolve(strict=True).is_relative_to(scope):
        raise FixtureError("browser-fixture-selected-installation")
    installation = installation.resolve(strict=True)
    client_bytes = private_file(installation / "client.toml")
    intent_bytes = private_file(installation / "intent.json")
    issued_bytes = private_file(installation / "issued.json")
    client, intent, issued = tomllib.loads(client_bytes.decode()), json.loads(intent_bytes), json.loads(issued_bytes)
    if (algorithm not in {"EdDSA", "ES256"} or client["signing_algorithm"] != algorithm
            or client["server_algorithm"] != algorithm
            or client["identity_id"] != intent["request"]["identity_id"]
            or client["identity_id"] != issued["identity_id"]
            or client["credential"] != issued["credential"]):
        raise FixtureError("browser-fixture-issued-actor-binding")

    def asset(name):
        path = Path(client[name])
        path = path if path.is_absolute() else installation / path
        if path.is_symlink() or not path.resolve(strict=True).is_relative_to(installation):
            raise FixtureError("browser-fixture-actor-asset-path")
        return private_file(path)

    public = base64.b64decode(intent["public_key"] + "=" * (-len(intent["public_key"]) % 4), altchars=b"-_", validate=True)
    if len(public) != (32 if algorithm == "EdDSA" else 65) or algorithm == "ES256" and public[0] != 4:
        raise FixtureError("browser-fixture-enrolled-public-key")
    private, server = asset("signing_private_key"), asset("server_public_key")
    result = dict(identityId=client["identity_id"], credential=client["credential"],
        device=dict(id=client["device_id"], kind=client["device_kind"], platform=client["device_platform"]),
        signing=dict(algorithm=algorithm, keyId=client["signing_key_id"], privatePkcs8=_base64(private), publicRaw=_base64(public)),
        server=dict(algorithm=algorithm, keyId=client["server_key_id"], publicRaw=_base64(server),
                    notBefore=_milliseconds(client["server_key_not_before"]), expiresAt=_milliseconds(client["server_key_expires_at"])))
    binding = dict(client_config_sha256=hashlib.sha256(client_bytes).hexdigest(),
                   original_intent_sha256=hashlib.sha256(intent_bytes).hexdigest(),
                   issuance_sha256=hashlib.sha256(issued_bytes).hexdigest(),
                   signing_algorithm=algorithm, server_signing_algorithm=algorithm)
    return result, binding
