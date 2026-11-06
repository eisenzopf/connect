"""Original issued authority metadata remains exact; no transport execution."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from lib.matrix_authority import MatrixAuthority


class MatrixAuthorityMetadataTests(unittest.TestCase):
    def issue(self, changed_expiry=False):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory, actors = root / "bridge", root / "actors"
            directory.mkdir(); actors.mkdir()
            (directory / "authority-1-request.json").write_text(json.dumps(dict(sequence=1, kind="expiry")))
            calls = []

            def execute(label, arguments):
                calls.append(label)
                intent = json.loads(Path(arguments[arguments.index("--request") + 1]).read_text())
                installation = Path(arguments[arguments.index("--output") + 1])
                installation.mkdir()
                (installation / "issued.json").write_text(json.dumps(dict(credential_id="original",
                    issued_at=100, expires_at=intent["expires_at"] + int(changed_expiry))))

            authority = MatrixAuthority(directory, actors, {"participant-1": {"identity_id": "identity"}},
                root / "instance", root / "admin", execute, "EdDSA")
            with patch("lib.matrix_authority.time.time", return_value=100), \
                    patch("lib.matrix_authority.browser_actor", return_value=({"synthetic": True}, {})):
                authority.poll()
            return json.loads((directory / "authority-1-response.json").read_text()), calls

    def test_exact_original_issue_and_expiry_are_forwarded_without_reissuance(self):
        response, calls = self.issue()
        self.assertEqual(response, dict(actor={"synthetic": True}, issued_at=100, expires_at=112))
        self.assertEqual(calls, ["authority-issue-expiry"])

    def test_changed_issued_expiry_refuses_metadata_without_second_issue(self):
        response, calls = self.issue(changed_expiry=True)
        self.assertEqual(response, dict(failure="ValueError"))
        self.assertEqual(calls, ["authority-issue-expiry"])


if __name__ == "__main__":
    unittest.main()
