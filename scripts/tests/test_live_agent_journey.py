"""Account-free checks of the actual-model acceptance observer and admission."""
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

LIVE = Path(__file__).resolve().parents[1] / "live"
sys.path.insert(0, str(LIVE))
from agent_journey.common import (ProofFailure, read_json, save, selected_environment,
                                  sha256, verify_runtime)
from agent_journey.cli import main
from agent_journey.model import followup_invocation
from agent_journey.observe import Observer


class JourneyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.runtime = self.root / "runtime"
        self.runtime.mkdir()
        files = {}
        for name in ("bin/connect", "sdk/python/connect_sdk/native.py",
                     "skills/connect/SKILL.md", "skills/connect/references/json-cli.md",
                     "docs/AGENT_ACCESS.md"):
            path = self.runtime / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture, never executed\n")
            files[name] = sha256(path)
        (self.runtime / "bin/connect").chmod(0o700)
        self.manifest = {"target": "aarch64-apple-darwin", "qualification_profile": "gate4-local",
                         "source_revision": "fixture-revision", "files": files}
        self.report = self.root / "package.json"
        self.publish_manifest()

    def publish_manifest(self):
        (self.runtime / "manifest.json").write_text(json.dumps(self.manifest))
        self.report.write_text(json.dumps({"result": "passed", "manifest": self.manifest}))

    def test_matching_qualified_runtime_is_admitted(self):
        self.assertEqual(verify_runtime(self.runtime, self.report), self.manifest)

    def test_changed_executable_cannot_borrow_previous_qualification(self):
        (self.runtime / "bin/connect").write_text("changed\n")
        with self.assertRaisesRegex(ProofFailure, "runtime-file-hash"):
            verify_runtime(self.runtime, self.report)

    def test_failed_or_different_package_report_is_rejected(self):
        for report in ({"result": "failed", "manifest": self.manifest},
                       {"result": "passed", "manifest": {}}):
            with self.subTest(report=report), self.assertRaises(ProofFailure):
                self.report.write_text(json.dumps(report))
                verify_runtime(self.runtime, self.report)

    def test_unrecorded_runtime_asset_is_rejected(self):
        (self.runtime / "extra.py").write_text("unexpected")
        with self.assertRaisesRegex(ProofFailure, "exact-runtime-inventory"):
            verify_runtime(self.runtime, self.report)

    def test_traversal_and_symlink_assets_are_rejected(self):
        for name in ("../outside", "/outside"):
            with self.subTest(name=name):
                manifest = copy.deepcopy(self.manifest)
                self.manifest["files"][name] = "irrelevant"
                self.publish_manifest()
                with self.assertRaisesRegex(ProofFailure, "safe-manifest-path"):
                    verify_runtime(self.runtime, self.report)
                self.manifest = manifest
        self.publish_manifest()
        path = self.runtime / "docs/AGENT_ACCESS.md"
        original = self.root / "outside"
        path.rename(original)
        path.symlink_to(original)
        with self.assertRaisesRegex(ProofFailure, "ordinary-runtime-file"):
            verify_runtime(self.runtime, self.report)

    def test_provider_credentials_never_enter_helper_or_model_environment(self):
        supplied = {"PATH": "/fixture", "HOME": "/fixture", "CODEX_API_KEY": "model-only",
                    "TELNYX_API_KEY": "excluded", "VAPI_PRIVATE_KEY": "excluded",
                    "CONNECT_OPERATOR_CREDENTIAL": "excluded", "UNRELATED_SECRET": "excluded"}
        with patch.dict(os.environ, supplied, clear=True):
            helper, model = selected_environment()
        self.assertNotIn("CODEX_API_KEY", helper)
        self.assertEqual(model["CODEX_API_KEY"], "model-only")
        for name in ("TELNYX_API_KEY", "VAPI_PRIVATE_KEY", "CONNECT_OPERATOR_CREDENTIAL", "UNRELATED_SECRET"):
            self.assertNotIn(name, helper)
            self.assertNotIn(name, model)

    def test_inference_opt_in_is_checked_before_any_runtime_access(self):
        with patch.object(sys, "argv", ["agent-journey.py", "missing-runtime",
                                       "--qualification-report", "missing-report",
                                       "--codex-binary", "missing-host"]):
            with self.assertRaisesRegex(ProofFailure, "explicit-inference-selection-required"):
                main()

    def test_evidence_cannot_overwrite_an_earlier_attempt(self):
        path = self.root / "result.json"
        save(path, {"status": "failed"})
        with self.assertRaises(FileExistsError):
            save(path, {"status": "passed"})
        self.assertEqual(read_json(path), {"status": "failed"})

    def test_bounded_evidence_read_rejects_oversized_file(self):
        path = self.root / "large.json"
        path.write_text('"' + "x" * 100 + '"')
        with self.assertRaisesRegex(ProofFailure, "bounded-json-file"):
            read_json(path, 32)

    def call(self, **changes):
        return {"tool": "connect_task_send", "arguments": {"reply_to": "reply-1"},
                "status": "completed", "result": {"structured_content": {"effect": "actual"}}, **changes}

    def test_followup_requires_exact_observed_call_not_prose_or_other_reply(self):
        expected = self.call()
        self.assertEqual(followup_invocation([expected], "reply-1")["structured"], {"effect": "actual"})
        for calls in ([], [self.call(arguments={"reply_to": "other"})], [expected, expected]):
            with self.subTest(calls=calls), self.assertRaises(ProofFailure):
                followup_invocation(calls, "reply-1")

    def test_failed_actual_tool_call_cannot_be_counted_as_success(self):
        for changes in ({"status": "failed"}, {"error": "transport"},
                        {"result": {"isError": True}}, {"result": {"is_error": True}}):
            with self.subTest(changes=changes), self.assertRaises(ProofFailure):
                followup_invocation([self.call(**changes)], "reply-1")

    def observe_pages(self, pages):
        class Client:
            def __init__(self, *_):
                self.pages = iter(pages)
            def __enter__(self):
                return self
            def __exit__(self, *_):
                pass
            def history(self, *_):
                return next(self.pages)
        observer = Observer()
        observer.human_config = "fixture"
        observer.binary = "fixture"
        observer.directory = self.root
        observer.conversations = {"a": "conversation-a"}
        return observer.histories(Client, "observation")

    def test_history_walks_to_empty_page_instead_of_treating_next_as_more_data(self):
        result = self.observe_pages([{"messages": [{"id": "one"}], "next": "cursor"},
                                     {"messages": [], "next": None}])
        self.assertEqual(result["a"]["messages"], [{"id": "one"}])

    def test_history_rejects_duplicate_messages(self):
        with self.assertRaisesRegex(ProofFailure, "deduplicated-history"):
            self.observe_pages([{"messages": [{"id": "one"}, {"id": "one"}], "next": "cursor"},
                                {"messages": [], "next": None}])

    def test_history_rejects_nonadvancing_or_unbounded_pagination(self):
        pages = [{"messages": [{"id": str(i)}], "next": "cursor"} for i in range(3)]
        with self.assertRaisesRegex(ProofFailure, "advancing-history-cursor"):
            self.observe_pages(pages)
        pages = [{"messages": [{"id": str(i)}], "next": str(i)} for i in range(3)]
        with self.assertRaisesRegex(ProofFailure, "pagination-exhausted"):
            self.observe_pages(pages)


if __name__ == "__main__":
    unittest.main()
