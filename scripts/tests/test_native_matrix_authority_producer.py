"""Inspect real producer files at its owner boundary without opening a transport."""
import datetime
import json
from pathlib import Path
import tempfile
import tomllib
from types import SimpleNamespace
import unittest

from lib.native_matrix_authority import run_native_authority
from lib.transport_fixture import publish, toml_bytes


class NativeAuthorityConfigurationTests(unittest.TestCase):
    def test_expiry_and_revocation_use_native_only_projection_without_changing_issuance(self):
        for signing in ("EdDSA", "ES256"):
            for network in ("local", "remote"):
                with self.subTest(signing=signing, network=network), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary).resolve()
                    binary = root / "connect"
                    binary.write_bytes(b"synthetic executable identity; never executed")
                    revision, opened, installations, native_configs = "a" * 40, [], {}, {}
                    platform = "ubuntu24-arm64" if network == "remote" else "macos-arm64"

                    def execute(label, command, intent=None):
                        if command[0] == "provision":
                            return json.dumps({"result": {"Ok": {"identity_id": "synthetic-identity"}}})
                        self.assertEqual(command[0], "issue", "No revocation or workload is simulated")
                        issued_intent = json.loads(Path(command[command.index("--request") + 1]).read_text())
                        installation = Path(command[command.index("--output") + 1])
                        installation.mkdir(mode=0o700)
                        native = dict(endpoint="127.0.0.1:5555", credential="synthetic-original-credential",
                            signing_algorithm=signing, server_algorithm=signing,
                            trust_certificate="trust.pem", signing_private_key="signing.key", server_public_key="server.key")
                        application = dict(endpoint="127.0.0.1:5556", credential=native["credential"], timeout_ms=1000,
                            expires_at=datetime.datetime.fromtimestamp(issued_intent["expires_at"], datetime.timezone.utc).isoformat())
                        publish(installation / "client.toml", toml_bytes(native))
                        publish(installation / "application.toml", toml_bytes(application))
                        publish(installation / "issued.json", json.dumps(dict(expires_at=issued_intent["expires_at"],
                            operation_id=issued_intent["operation_id"], credential_id="synthetic-credential-id")).encode())
                        for key in ("trust_certificate", "signing_private_key", "server_public_key"):
                            publish(installation / native[key], b"synthetic original key custody\n")
                        installations[installation] = {p.name: p.read_bytes() for p in installation.iterdir()}
                        native_configs[installation] = native
                        return ""

                    def owner_factory(command, directory, log, *, cwd, env, wrap):
                        value = json.loads(Path(command[2]).read_text())
                        configuration = Path(value["config"])
                        opened.append((value, configuration, configuration.read_bytes()))
                        # This test inspects preparation only. Never manufacture
                        # a successful execution or an authentication verdict.
                        observation = dict(passed=False, executed=False, kind=value["kind"],
                            source_revision=revision, binary_sha256=value["binary_sha256"],
                            signing_algorithm=signing, network_profile=network, host={"platform": platform})
                        Path(command[3]).write_text(json.dumps(observation))
                        return SimpleNamespace(poll=lambda: 1, close=lambda: dict(original_supervisor_joined=True,
                            original_workers_joined=True, forced=False, exit_code=1))

                    arguments = dict(root=root, directory=root / "authority", instance=root / "instance",
                        admin=root / "admin.toml", binary=binary, env={}, execute=execute, source_revision=revision,
                        signing_algorithm=signing, network_profile=network, client_platform=platform, owner_factory=owner_factory)
                    if network == "remote":
                        arguments.update(client_executable="/guest/bin/connect", client_binary_sha256="b" * 64)
                    results = run_native_authority(**arguments)
                    self.assertEqual([v[0]["kind"] for v in opened], ["expiry", "revocation"])
                    for value, configuration, before in opened:
                        config = tomllib.loads(before.decode())
                        self.assertEqual(set(config), {"tenant_id", "control"})
                        self.assertEqual(config["tenant_id"], "tnt_default")
                        installation = configuration.parent / "installation"
                        expected = dict(native_configs[installation])
                        for key in ("trust_certificate", "signing_private_key", "server_public_key"):
                            expected[key] = str(installation / expected[key])
                        self.assertEqual(config["control"], expected, "Credential, signer and native route stay original")
                        self.assertEqual(configuration.read_bytes(), before, "No config rewrite at the authority boundary")
                        self.assertEqual({p.name: p.read_bytes() for p in installation.iterdir()}, installations[installation])
                        application = tomllib.loads((installation / "application.toml").read_text())
                        self.assertEqual(datetime.datetime.fromisoformat(application["expires_at"]).timestamp(), value["expires_at"])
                        self.assertEqual(application["credential"], config["control"]["credential"])
                        self.assertTrue(results[value["kind"]]["original_issuance_unchanged"])
                        self.assertFalse(results[value["kind"]]["executed"])
                        self.assertFalse(results[value["kind"]]["passed"])


if __name__ == "__main__":
    unittest.main()
