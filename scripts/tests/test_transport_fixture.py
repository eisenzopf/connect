"""Fixture configuration cannot broaden scope, renew trust or replace existing state."""
import copy
import datetime
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.transport_fixture import FixtureError, inspect_seed, publish, toml_bytes


class FixtureBoundary(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        matrix = json.loads((ROOT / "contracts/gate6-transport-matrix.json").read_text())
        for name in ["contracts/gate6-transport-matrix.json", matrix["profile"], *matrix["service_contract"], *matrix["fixture_templates"]]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(matrix) if name == "contracts/gate6-transport-matrix.json" else "fixture input")
        path = self.root / "crates/connect-store/src/lib.rs"
        path.parent.mkdir(parents=True)
        path.write_text("pub const SCHEMA_VERSION: u32 = 116;")
        path = self.root / "crates/connect-protocol/src/lib.rs"
        path.parent.mkdir(parents=True)
        path.write_bytes((ROOT / "crates/connect-protocol/src/lib.rs").read_bytes())
        self.seed = self.root / ".local/seed"
        self.seed.mkdir(parents=True, mode=0o700)
        now = datetime.datetime.now(datetime.timezone.utc)
        self.start = (now - datetime.timedelta(seconds=1)).isoformat()
        self.end = (now + datetime.timedelta(hours=1)).isoformat()
        self.server = dict(network_profile="local", bind="127.0.0.1:4433", database="state.sqlite3",
            certificate="asset-0000.bin", private_key="asset-0001.bin", signing_private_key="asset-0002.bin",
            signing_key_id="fixture-server", signing_algorithm="EdDSA", signing_key_not_before=self.start,
            signing_key_expires_at=self.end, http_bind="127.0.0.1:8080", https=[],
            limits=dict(participants=4, connections=6, sessions=2, message_bytes=65536))
        self.operator = dict(network_profile="local", endpoint="127.0.0.1:4433", server_name="localhost",
            credential="synthetic-not-a-real-bearer", identity_id="fixture-operator", device_id="fixture-device", signing_algorithm="EdDSA",
            server_key_id="fixture-server", server_algorithm="EdDSA", server_key_not_before=self.start,
            server_key_expires_at=self.end, trust_certificate="asset-0000.bin", signing_private_key="asset-0003.bin",
            server_public_key="asset-0004.bin")
        for name in ["state.sqlite3", *(f"asset-{n:04}.bin" for n in range(5))]:
            publish(self.seed / name, ("synthetic-unit-data-not-a-valid-backup:" + name).encode())
        self.parameters = dict(cell_id="G6-058/ubuntu24-arm64/local/native-quic/EdDSA",
            candidate_source_revision="a" * 40, candidate_binary_sha256="b" * 64, seed_manifest_sha256="c" * 64,
            topology=dict(kind="local-loopback", server_platform="ubuntu24-arm64", client_platform="ubuntu24-arm64",
                client_emulated=False, native_endpoint="127.0.0.1:4433", server_name="localhost"), browser_origin=None)
        self.save()

    def save(self):
        for name, content in [("server.toml", toml_bytes(self.server)), ("operator.toml", toml_bytes(self.operator))]:
            path = self.seed / name
            path.write_bytes(content)
            path.chmod(0o600)
        files = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in self.seed.iterdir() if p.name != "manifest.json"}
        path = self.seed / "manifest.json"
        path.write_text(json.dumps(dict(format="connect/canonical-backup/1", snapshot=dict(schema=116), files=files, blobs={})))
        path.chmod(0o600)
        self.parameters["seed_manifest_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()

    def test_private_seed_inspection_preserves_trust_and_does_not_admit_runtime(self):
        plan = inspect_seed(self.root, self.seed, self.parameters)
        self.assertEqual(plan["server"]["signing_key_expires_at"], self.end)
        self.assertEqual(plan["operator"]["credential"], "synthetic-not-a-real-bearer")
        self.assertEqual(plan["cell"]["status"], "not-run")
        self.assertIsNone(plan["cell"]["decoded_media"])

    def test_browser_cell_refuses_missing_media_or_enrollment_https_instead_of_degrading(self):
        self.parameters["cell_id"] = "G6-058/ubuntu24-arm64/local/chrome-wss-webrtc/EdDSA"
        self.parameters["browser_origin"] = "https://browser.fixture.invalid"
        for name in ("webtransport", "websocket"):
            self.server[name] = dict(mount_path="/connect", allowed_origins=[self.parameters["browser_origin"]])
        self.save()
        with self.assertRaisesRegex(FixtureError, "browser-media-required"):
            inspect_seed(self.root, self.seed, self.parameters)
        self.server["webrtc"] = dict(opaque_prevalidated_seed_field=True)
        self.save()
        with self.assertRaisesRegex(FixtureError, "enrollment-https-required"):
            inspect_seed(self.root, self.seed, self.parameters)

    def test_arm_component_requires_explicit_mode_and_preserves_matrix_inventory(self):
        from lib.transport_matrix import inventory
        before = inventory(self.root, "a" * 40)
        self.parameters["cell_id"] = "G6-058-component/ubuntu24-arm64/local/native-quic/EdDSA"
        self.parameters["topology"].update(server_platform="ubuntu24-arm64", client_platform="ubuntu24-arm64")
        with self.assertRaisesRegex(FixtureError, "fixture-cell"):
            inspect_seed(self.root, self.seed, self.parameters)
        plan = inspect_seed(self.root, self.seed, self.parameters, arm64_component=True)
        self.assertTrue(plan["cell"]["component_only"])
        self.assertEqual(plan["cell"]["status"], "not-run")
        self.assertEqual(inventory(self.root, "a" * 40), before)
        self.assertEqual(before["required_cells"], 32)
        self.parameters["topology"]["server_platform"] = "ubuntu24-x86_64"
        with self.assertRaisesRegex(FixtureError, "fixture-topology"):
            inspect_seed(self.root, self.seed, self.parameters, arm64_component=True)

    def test_arm_component_cannot_impersonate_a_required_cell_or_browser_or_remote(self):
        for identity in [self.parameters["cell_id"],
                "G6-058-component/ubuntu24-arm64/remote-profile-fixture/native-quic/EdDSA",
                "G6-058-component/ubuntu24-arm64/local/chrome-wss-webrtc/EdDSA"]:
            with self.subTest(identity=identity):
                self.parameters["cell_id"] = identity
                with self.assertRaisesRegex(FixtureError, "fixture-arm64-component"):
                    inspect_seed(self.root, self.seed, self.parameters, arm64_component=True)

    def test_wrong_signing_network_or_expiry_cannot_be_rewritten_as_a_valid_cohort(self):
        base = copy.deepcopy(self.server)
        for key, value in [("signing_algorithm", "ES256"), ("network_profile", "remote"),
                ("signing_key_expires_at", (datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(seconds=2)).isoformat())]:
            with self.subTest(field=key):
                self.server = copy.deepcopy(base)
                self.server[key] = value
                self.save()
                with self.assertRaises(FixtureError):
                    inspect_seed(self.root, self.seed, self.parameters)

    def test_external_components_or_changed_contact_refuse_before_restoration(self):
        self.server["provider_voice"] = dict(endpoint="https://never-contact.invalid")
        self.save()
        with self.assertRaisesRegex(FixtureError, "external-component"):
            inspect_seed(self.root, self.seed, self.parameters)
        del self.server["provider_voice"]
        self.save()
        self.parameters["topology"]["native_endpoint"] = "127.0.0.1:9999"
        with self.assertRaisesRegex(FixtureError, "provisioned-contact"):
            inspect_seed(self.root, self.seed, self.parameters)

    def test_server_signer_cannot_reuse_operator_or_tls_private_material(self):
        for name in ("asset-0001.bin", "asset-0003.bin"):
            with self.subTest(shared_with=name):
                (self.seed / "asset-0002.bin").write_bytes((self.seed / name).read_bytes())
                self.save()
                with self.assertRaisesRegex(FixtureError, "distinct-signing-domains"):
                    inspect_seed(self.root, self.seed, self.parameters)

    def test_operator_algorithm_must_match_the_actual_client_cohort(self):
        self.operator["signing_algorithm"] = "ES256"
        self.save()
        with self.assertRaisesRegex(FixtureError, "signing-profile"):
            inspect_seed(self.root, self.seed, self.parameters)

    def test_current_serde_omitted_session_default_is_resolved_without_rewriting_seed(self):
        del self.server["limits"]["sessions"]
        self.save()
        original = (self.seed / "server.toml").read_bytes()
        plan = inspect_seed(self.root, self.seed, self.parameters)
        self.assertNotIn("sessions", plan["server"]["limits"])
        self.assertEqual((self.seed / "server.toml").read_bytes(), original)
        self.server["limits"]["sessions"] = 1
        self.save()
        with self.assertRaisesRegex(FixtureError, "workload-capacity"):
            inspect_seed(self.root, self.seed, self.parameters)

    def test_omitted_session_default_must_be_bound_to_the_current_source_contract(self):
        del self.server["limits"]["sessions"]
        self.save()
        path = self.root / "crates/connect-protocol/src/lib.rs"
        path.write_text(path.read_text().replace('default = "default_sessions"', 'default = "other_sessions"'))
        with self.assertRaisesRegex(FixtureError, "session-default-contract"):
            inspect_seed(self.root, self.seed, self.parameters)

    def test_full_size_native_payload_requires_explicit_seed_capacity_without_rewriting_it(self):
        for limit in (None, True, 16384, 65535):
            with self.subTest(limit=limit):
                if limit is None:
                    self.server["limits"].pop("message_bytes", None)
                else:
                    self.server["limits"]["message_bytes"] = limit
                self.save()
                before = {p.name: p.read_bytes() for p in self.seed.iterdir()}
                with self.assertRaisesRegex(FixtureError, "workload-message-limit"):
                    inspect_seed(self.root, self.seed, self.parameters)
                self.assertEqual({p.name: p.read_bytes() for p in self.seed.iterdir()}, before)
        for limit in (65536, 131072):
            with self.subTest(limit=limit):
                self.server["limits"]["message_bytes"] = limit
                self.save()
                self.assertEqual(inspect_seed(self.root, self.seed, self.parameters)["server"]["limits"]["message_bytes"], limit)

    def test_symlink_or_public_secret_asset_refuses(self):
        path = self.seed / "asset-0000.bin"
        original = path.read_bytes()
        path.chmod(0o644)
        with self.assertRaises(FixtureError):
            inspect_seed(self.root, self.seed, self.parameters)
        path.unlink()
        elsewhere = self.root / "elsewhere"
        elsewhere.write_bytes(original)
        path.symlink_to(elsewhere)
        with self.assertRaises(FixtureError):
            inspect_seed(self.root, self.seed, self.parameters)

    def test_toml_string_injection_stays_data_and_exclusive_publish_preserves_original(self):
        content = 'private\n[provider_voice]\nendpoint="https://never-contact.invalid"'
        data = dict(name=content, https=[dict(surface="application", allowed_origins=["https://browser.fixture.invalid"])], nested=dict(enabled=True))
        self.assertEqual(tomllib.loads(toml_bytes(data).decode()), data)
        path = self.root / ".local/existing.toml"
        publish(path, b"original")
        with self.assertRaises(FileExistsError):
            publish(path, b"replacement")
        self.assertEqual(path.read_bytes(), b"original")
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
