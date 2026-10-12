"""CI target coverage, real Cargo retirement and retained failure diagnostics."""
import contextlib
import io
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from lib.ci_diagnostics import bundle, redact, redact_value
from lib.ci_cache import retire_baseline_cache
from lib.ci_workspace import retire, run, targets
from lib.local_process import ROOT


class WorkspaceCoverage(unittest.TestCase):
    def test_workspace_target_selection_accounts_for_docs_and_compile_only_examples(self):
        def target(name, kind, test=True, doctest=False):
            return dict(name=name, kind=[kind], test=test, doctest=doctest)
        metadata = {"workspace_members": ["member"], "packages": [
            {"id": "member", "name": "fixture", "targets": [
                target("library", "lib", doctest=True), target("binary", "bin"),
                target("integration", "test"), target("example", "example", test=False),
                target("build-script", "custom-build", test=False)]},
            {"id": "dependency", "name": "ignored", "targets": [target("other", "lib")]}]}
        self.assertEqual([(s[2]["name"], s[3]) for group in targets(metadata) for s in group], [
            ("library", "test"), ("library", "doc"), ("binary", "test"),
            ("integration", "test"), ("example", "build")])
        metadata["packages"][0]["targets"].append(target("future", "unknown"))
        with self.assertRaises(ValueError):
            targets(metadata)

    def test_retirement_never_removes_application_binaries_shared_libraries_or_foreign_paths(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); target = root / "target"
            deps = target / "debug/deps"; deps.mkdir(parents=True)
            binary = deps / "fixture-hash"; binary.write_bytes(b"test executable")
            symbols = deps / "fixture-hash.dSYM"; symbols.mkdir(); (symbols / "symbol").write_bytes(b"symbols")
            library = deps / "libfixture.rlib"; library.write_bytes(b"shared")
            application = target / "debug/connect"; application.write_bytes(b"application")
            result = retire(binary, target, "test")
            self.assertEqual(len(result["paths"]), 2)
            self.assertFalse(binary.exists()); self.assertFalse(symbols.exists())
            self.assertEqual(library.read_bytes(), b"shared")
            with self.assertRaises(ValueError):
                retire(application, target, "test")
            link = deps / "link"; link.symlink_to(application)
            with self.assertRaises(ValueError):
                retire(link, target, "test")
            self.assertEqual(application.read_bytes(), b"application")

    def fixture(self, root, failure=False, example_test=False):
        (root / "src").mkdir(); (root / "tests").mkdir(); (root / "examples").mkdir()
        manifest = '''[package]
name = "ci-fixture"
version = "0.1.0"
edition = "2021"
[features]
extra = []
[dependencies]
ci-dependency = { path = "dependency" }
[workspace]
members = ["dependency"]
'''
        if example_test:
            manifest += '\n[[example]]\nname = "example"\ntest = true\n'
        (root / "Cargo.toml").write_text(manifest)
        (root / "dependency/src").mkdir(parents=True)
        (root / "dependency/Cargo.toml").write_text('''[package]
name = "ci-dependency"
version = "0.1.0"
edition = "2021"
[features]
enabled = []
''')
        (root / "dependency/src/lib.rs").write_text('pub fn feature_enabled() -> bool { cfg!(feature="enabled") }\n')
        (root / "src/lib.rs").write_text('''/// ```
/// assert_eq!(ci_fixture::value(), 7);
/// ```
pub fn value() -> u8 { 7 }
#[test] fn unit() { assert_eq!(value(), 7); }
#[cfg(feature="extra")] #[test] fn feature() { assert_eq!(value(), 7); }
#[test] fn workspace_feature_union() { assert!(ci_dependency::feature_enabled()); }
''')
        (root / "src/main.rs").write_text('fn main() {}\n#[test] fn binary() { assert_eq!(2+2,4); }\n')
        (root / "examples/example.rs").write_text('fn main() {}\n#[test] fn example_test() { assert_eq!(3+4,7); }\n')
        (root / "tests/integration.rs").write_text(
            '#[test] fn integration() { assert!(false, "owned fixture failure"); }\n' if failure
            else '#[test] fn integration() { assert_eq!(ci_fixture::value(),7); }\n')
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)

    def environment(self, root):
        return patch.dict(os.environ, {"CARGO_TARGET_DIR": str(root / "target"),
                                      "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0",
                                      "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "1"})

    def test_actual_cargo_all_features_tests_docs_examples_and_retires_only_completed_targets(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); self.fixture(root)
            output = io.StringIO()
            with self.environment(root), contextlib.redirect_stdout(output):
                result = run(root, root / "report", all_features=True, minimum_free_bytes=0)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(len(result["steps"]), 5)
            self.assertIn("3 passed; 0 failed", output.getvalue())
            self.assertIn("value (line 1)", output.getvalue())
            self.assertTrue((root / "target/debug/ci-fixture").exists())
            self.assertTrue(list((root / "target/debug/deps").glob("*.rlib")))
            for step in result["steps"]:
                for retired in step["retired"]:
                    for path in retired["paths"]:
                        self.assertFalse(pathlib.Path(path).exists())

    def test_actual_cargo_example_harness_uses_and_retires_the_examples_directory(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); self.fixture(root, example_test=True)
            with self.environment(root), contextlib.redirect_stdout(io.StringIO()):
                result = run(root, root / "report", all_features=True, minimum_free_bytes=0)
            example = next(s for s in result["steps"] if s["targets"][0]["target"] == "example")
            self.assertEqual(example["mode"], "test")
            self.assertEqual(example["status"], "passed")
            self.assertIn("/debug/examples/", example["retired"][0]["paths"][0])

    def test_real_compiler_diagnostic_remains_visible_and_failure_stops_coverage(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); self.fixture(root)
            (root / "src/lib.rs").write_text("pub fn value() -> u8 { unknown_symbol }\n")
            output = io.StringIO()
            with self.environment(root), contextlib.redirect_stdout(output), self.assertRaises(RuntimeError):
                run(root, root / "report", all_features=True, minimum_free_bytes=0)
            self.assertIn("unknown_symbol", output.getvalue())
            self.assertIn("error", output.getvalue())
            result = json.loads((root / "report/result.json").read_text())
            self.assertEqual(len(result["steps"]), 1)
            self.assertEqual(result["status"], "failed")

    def test_failed_real_cargo_target_is_not_retired_or_promoted(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); self.fixture(root, failure=True)
            with self.environment(root), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(RuntimeError):
                    run(root, root / "report", all_features=True, minimum_free_bytes=0)
            result = json.loads((root / "report/result.json").read_text())
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["steps"][-1]["status"], "failed")
            self.assertNotIn("retired", result["steps"][-1])
            self.assertTrue(list((root / "target/debug/deps").glob("integration-*")))

    def test_disk_headroom_failure_is_recorded_before_a_test_build(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); self.fixture(root)
            with self.environment(root), patch("lib.ci_workspace.snapshot", return_value={"free_bytes": 0}):
                with self.assertRaisesRegex(RuntimeError, "disk headroom"):
                    run(root, root / "report")
            result = json.loads((root / "report/result.json").read_text())
            self.assertEqual(result["steps"], [])
            self.assertFalse((root / "target/debug").exists())


class FailureEvidence(unittest.TestCase):
    def test_failed_phase_is_retained_and_private_state_and_success_logs_are_excluded(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); phase = root / ".local/checks/run"; phase.mkdir(parents=True)
            secret = "a" * 64
            (phase / "result.json").write_text(json.dumps({"result": "failed", "steps": [
                {"name": "lint", "result": "passed", "log": "lint.log"},
                {"name": "workspace", "result": "failed", "log": "workspace.log"}]}))
            (phase / "lint.log").write_text("successful lint")
            (phase / "workspace.log").write_text(f'actual workspace failure\ntoken = "{secret}"\nBearer bearer-value\n')
            (phase / "private.toml").write_text("private provider configuration")
            with patch("lib.ci_diagnostics.disk", return_value={"free_bytes": 1024}), contextlib.redirect_stdout(io.StringIO()):
                result = bundle(root, root / "upload")
            self.assertEqual(result["reports"][0]["result"], "failed")
            files = [p.name for p in (root / "upload").iterdir()]
            self.assertIn("checks-workspace.log", files)
            self.assertNotIn("checks-lint.log", files); self.assertNotIn("private.toml", files)
            self.assertEqual(json.loads((root / "upload/checks-result.json").read_text())["result"], "failed")
            text = (root / "upload/checks-workspace.log").read_text()
            self.assertIn("actual workspace failure", text)
            self.assertNotIn(secret, text); self.assertNotIn("bearer-value", text)

    def test_active_failed_report_traversal_and_grant_secrets_are_not_uploaded(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp); phase = root / ".local/gate-4-checks/run"; phase.mkdir(parents=True)
            (phase / "result.json").write_text(json.dumps({"status": "failed", "active_phase": "workspace",
                "steps": [{"status": "failed", "log": "../private.log"}]}))
            (phase / "workspace.log").write_text("disk failure ctg1.fixture.secret\n")
            with patch("lib.ci_diagnostics.disk", return_value={}), contextlib.redirect_stdout(io.StringIO()):
                bundle(root, root / "upload")
            self.assertIn("[REDACTED-GRANT]", (root / "upload/gate-4-checks-workspace.log").read_text())
            self.assertFalse((root / "upload/private.log").exists())

    def test_redaction_handles_json_toml_bearer_and_grants(self):
        for text in ['{"credential":"secret-value"}', "api_key = 'secret-value'", "Bearer secret-value", "ctg1.secret-value"]:
            self.assertNotIn("secret-value", redact(text))

    def test_structured_secrets_are_redacted_recursively_without_losing_types(self):
        value = {"error": {"credential": "dummy-nonhex-secret", "api_key": "other-private-value"},
                 "steps": [{"Authorization": "private", "token": {"nested": "private"}}],
                 "environment": {"TELNYX_API_KEY": "private", "access_token": "private",
                                 "credential_id": "public-identifier"},
                 "count": 2, "ready": False, "detail": "Bearer opaque-private-value"}
        clean = redact_value(value)
        self.assertEqual(clean["error"], {"credential": "[REDACTED]", "api_key": "[REDACTED]"})
        self.assertEqual(clean["steps"], [{"Authorization": "[REDACTED]", "token": "[REDACTED]"}])
        self.assertEqual(clean["environment"], {"TELNYX_API_KEY": "[REDACTED]",
                         "access_token": "[REDACTED]", "credential_id": "public-identifier"})
        self.assertEqual(clean["count"], 2)
        self.assertIs(clean["ready"], False)
        self.assertEqual(clean["detail"], "Bearer [REDACTED]")
        self.assertNotIn("private", json.dumps(clean))
        self.assertEqual(value["error"]["credential"], "dummy-nonhex-secret")


class BaselineCache(unittest.TestCase):
    def sealed(self, output):
        (output / "target/debug").mkdir(parents=True)
        (output / "target/debug/cache").write_bytes(b"generated")
        binary = output / "connect"; binary.write_bytes(b"sealed executable")
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        (output / "manifest.json").write_text(json.dumps({"source_revision": output.name, "binary_sha256": digest}))
        return digest

    def test_only_sealed_baseline_cache_is_retired_and_inputs_survive(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp).resolve()
            output = root / ".local/recovery-baselines" / ("a" * 40)
            digest = self.sealed(output)
            binary = output / "connect"
            manifest = output / "manifest.json"
            saved = manifest.read_bytes()
            binary.write_bytes(b"changed")
            with patch("lib.ci_cache.ROOT", root), self.assertRaises(ValueError):
                retire_baseline_cache(output, output.name)
            self.assertTrue((output / "target/debug/cache").exists())
            binary.write_bytes(b"sealed executable")
            with patch("lib.ci_cache.ROOT", root):
                result = retire_baseline_cache(output, output.name)
            self.assertTrue(result["cache_removed"])
            self.assertFalse((output / "target").exists())
            self.assertEqual(manifest.read_bytes(), saved)
            self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), digest)

    def test_output_directory_symlink_cannot_retire_foreign_cache(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp).resolve(); revision = "a" * 40
            foreign = root / "foreign" / revision; self.sealed(foreign)
            parent = root / ".local/recovery-baselines"; parent.mkdir(parents=True)
            output = parent / revision; output.symlink_to(foreign, target_is_directory=True)
            with patch("lib.ci_cache.ROOT", root), patch("lib.ci_cache.shutil.rmtree") as remove:
                with self.assertRaises(ValueError):
                    retire_baseline_cache(output, revision)
                remove.assert_not_called()
            self.assertEqual((foreign / "target/debug/cache").read_bytes(), b"generated")

    def test_parent_directory_symlink_cannot_retire_foreign_cache(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp).resolve(); revision = "a" * 40
            foreign = root / "foreign"; self.sealed(foreign / revision)
            (root / ".local").mkdir()
            (root / ".local/recovery-baselines").symlink_to(foreign, target_is_directory=True)
            with patch("lib.ci_cache.ROOT", root), patch("lib.ci_cache.shutil.rmtree") as remove:
                with self.assertRaises(ValueError):
                    retire_baseline_cache(root / ".local/recovery-baselines" / revision, revision)
                remove.assert_not_called()
            self.assertEqual((foreign / revision / "target/debug/cache").read_bytes(), b"generated")

    def test_intermediate_directory_symlink_is_denied_by_resolved_owner_validation(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp).resolve(); revision = "a" * 40
            foreign = root / "foreign"; self.sealed(foreign / "recovery-baselines" / revision)
            (root / ".local").symlink_to(foreign, target_is_directory=True)
            with patch("lib.ci_cache.ROOT", root), patch("lib.ci_cache.shutil.rmtree") as remove:
                with self.assertRaises(ValueError):
                    retire_baseline_cache(root / ".local/recovery-baselines" / revision, revision)
                remove.assert_not_called()
            self.assertEqual((foreign / "recovery-baselines" / revision / "target/debug/cache").read_bytes(), b"generated")

    def test_matching_basename_outside_builder_owner_is_not_authority(self):
        with tempfile.TemporaryDirectory(dir=ROOT / ".local") as tmp:
            root = pathlib.Path(tmp).resolve(); revision = "a" * 40
            foreign = root / "foreign/recovery-baselines" / revision; self.sealed(foreign)
            with patch("lib.ci_cache.ROOT", root), patch("lib.ci_cache.shutil.rmtree") as remove:
                with self.assertRaises(ValueError):
                    retire_baseline_cache(foreign, revision)
                remove.assert_not_called()
            self.assertEqual((foreign / "target/debug/cache").read_bytes(), b"generated")


class PackageSelectors(unittest.TestCase):
    def test_missing_or_unusable_explicit_host_fails_before_creating_check_state(self):
        directory = ROOT / ".local/checks"
        before = sorted(directory.iterdir()) if directory.exists() else []
        for arguments in [["--package", "/not/a/candidate"], ["--codex-binary", "/not/a/codex"],
                          ["--package", "/not/a/candidate", "--codex-binary", "/not/a/codex"]]:
            result = subprocess.run([sys.executable, str(ROOT / "scripts/check-local.py"), *arguments],
                                    cwd=ROOT, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn("RUN dependencies", result.stdout)
        self.assertEqual(before, sorted(directory.iterdir()) if directory.exists() else [])


if __name__ == "__main__":
    unittest.main()
