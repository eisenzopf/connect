"""Private browser fixture exports must retain issuer and actor-file boundaries."""
import base64
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.browser_matrix_actors import browser_actor
from lib.transport_fixture import FixtureError, toml_bytes


class BrowserActorExport(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.scope = Path(directory.name)
        self.installation = self.scope / "selected"
        self.installation.mkdir(mode=0o700)
        self.client = dict(signing_algorithm="EdDSA", server_algorithm="EdDSA",
            identity_id="synthetic-identity", credential="synthetic-issued-credential",
            device_id="synthetic-device", device_kind="desktop", device_platform="fixture",
            signing_key_id="synthetic-key", server_key_id="synthetic-server-key",
            signing_private_key="device-signing.der", server_public_key="server-signing.pub",
            server_key_not_before="2026-11-04T00:00:00Z", server_key_expires_at="2026-11-05T00:00:00Z")
        self.write("intent.json", json.dumps(dict(request=dict(identity_id=self.client["identity_id"]),
            public_key=base64.urlsafe_b64encode(bytes(32)).decode().rstrip("="))).encode())
        self.write("issued.json", json.dumps(dict(identity_id=self.client["identity_id"],
            credential=self.client["credential"])).encode())
        self.write("device-signing.der", b"synthetic-private-key-not-used-for-authentication")
        self.write("server-signing.pub", bytes(32))
        self.save_client()

    def write(self, name, body):
        path = self.installation / name
        path.write_bytes(body)
        path.chmod(0o600)

    def save_client(self):
        self.write("client.toml", toml_bytes(self.client))

    def export(self, **extra):
        return browser_actor(self.installation, scope=self.scope, algorithm=extra.get("algorithm", "EdDSA"))

    def test_private_key_from_another_actor_cannot_enter_selected_actor_export(self):
        other = self.scope / "another-actors-private-key.der"
        other.write_bytes(b"must-not-be-exported")
        other.chmod(0o600)
        self.client["signing_private_key"] = str(other)
        self.save_client()
        with self.assertRaisesRegex(FixtureError, "actor-asset-path"):
            self.export()
        self.assertEqual(other.read_bytes(), b"must-not-be-exported")

    def test_changed_credential_or_signing_cohort_cannot_borrow_original_issuance(self):
        with self.assertRaisesRegex(FixtureError, "issued-actor-binding"):
            self.export(algorithm="ES256")
        self.client["credential"] = "different-synthetic-credential"
        self.save_client()
        with self.assertRaisesRegex(FixtureError, "issued-actor-binding"):
            self.export()

    def test_publicly_readable_or_symlinked_private_inputs_refuse_export(self):
        path = self.installation / "client.toml"
        path.chmod(0o644)
        with self.assertRaisesRegex(FixtureError, "fixture-private-file"):
            self.export()
        self.save_client()
        key = self.installation / "device-signing.der"
        key.unlink()
        key.symlink_to(self.installation / "server-signing.pub")
        with self.assertRaisesRegex(FixtureError, "actor-asset-path"):
            self.export()


if __name__ == "__main__":
    unittest.main()
