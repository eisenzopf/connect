"""Package integrity and deterministic archive boundaries; no root service effects."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import sys
import time
import zipfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    sys.path.insert(0, str(ROOT / 'scripts'))
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


installer = load('linux_installer', 'deploy/linux/install.py')
packager = load('linux_packager', 'scripts/package-linux.py')
verifier = load('linux_verifier', 'scripts/verify-linux-installation.py')
build_pipeline = load('linux_build_pipeline', 'scripts/check-linux-package-build.py')


class Integrity(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'connect-fixture'
        self.root.mkdir()
        self.files = {}
        for name in ('bin/connect', 'deploy/connect.service', 'deploy/install.py', 'README.md', 'LICENSE', 'dependencies/source-lock.json'):
            path = self.root / name
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(name.encode())
            self.files[name] = {'sha256': installer.digest(path), 'mode': 0o755 if name == 'bin/connect' else 0o644}
        self.manifest = {'format': 'connect/linux-package/1', 'target': 'ubuntu-24.04-x86_64', 'installation_id': 'connect-fixture', 'files': self.files, 'source_revision': 'a' * 40, 'source_epoch': 42, 'inputs': {'fixture.rs': '1' * 64}, 'build_profile': 'release', 'test_support': False, 'publication_authorized': False}
        self.write_manifest()

    def write_manifest(self):
        (self.root / 'manifest.json').write_text(json.dumps(self.manifest))

    def test_archive_repacking_is_byte_identical(self):
        self.assertEqual(installer.verified_files(self.root), self.manifest)
        a, b = self.root.parent / 'a.gz', self.root.parent / 'b.gz'
        packager.archive(self.root, a, 42)
        for path in self.root.rglob('*'):
            os.utime(path, (500, 500))
            if path.is_file():
                path.chmod(0o700)
        packager.archive(self.root, b, 42)
        self.assertEqual(hashlib.sha256(a.read_bytes()).digest(), hashlib.sha256(b.read_bytes()).digest())

    def test_sealed_release_directories_are_traversable_under_private_umask(self):
        prior = os.umask(0o077)
        try:
            stage = self.root / 'private-stage'
            (stage / 'bin').mkdir(parents=True, mode=0o700)
            installer.seal_directories(stage)
            self.assertEqual(stage.stat().st_mode & 0o777, 0o755)
            self.assertEqual((stage / 'bin').stat().st_mode & 0o777, 0o755)
        finally:
            os.umask(prior)

    def test_complete_configuration_grants_service_read_under_private_umask(self):
        config = self.root / 'server.toml'
        prior = os.umask(0o077)
        try:
            # Unit fixtures run without root; only ownership needs a stub. The
            # file creation, umask, publication and final permissions are real.
            with mock.patch.object(installer.os, 'fchown') as ownership:
                installer.publish_configuration(config, 'database = "private.db"\n', 123)
                ownership.assert_called_once()
                self.assertEqual(ownership.call_args.args[1:], (0, 123))
            self.assertEqual(config.stat().st_mode & 0o777, 0o640)
            self.assertEqual(config.read_text(), 'database = "private.db"\n')
            with self.assertRaises(FileExistsError):
                installer.publish_configuration(config, 'replace', 123)
            self.assertEqual(config.read_text(), 'database = "private.db"\n')
        finally:
            os.umask(prior)

    def package_zip(self):
        archive = self.root.parent / 'package.tar.gz'
        packager.archive(self.root, archive, 42)
        wrapped = self.root.parent / 'artifact.zip'
        with zipfile.ZipFile(wrapped, 'w') as output:
            output.write(archive, archive.name)
        return wrapped

    def test_independent_verifier_checks_packaged_bytes_and_source_map(self):
        wrapped = self.package_zip()
        with verifier.artifact(wrapped, installer.digest(wrapped)) as artifact:
            manifest, proof = verifier.verify_package(artifact, 'a' * 40, {'fixture.rs': '1' * 64})
            self.assertEqual(manifest, self.manifest)
            self.assertEqual(proof['binary_sha256'], self.files['bin/connect']['sha256'])
        with zipfile.ZipFile(wrapped) as artifact:
            with self.assertRaises(ValueError):
                verifier.verify_package(artifact, 'b' * 40, {'fixture.rs': '1' * 64})
        with zipfile.ZipFile(wrapped) as artifact:
            with self.assertRaises(ValueError):
                verifier.verify_package(artifact, 'a' * 40, {'fixture.rs': '2' * 64})
        with self.assertRaises(ValueError):
            verifier.artifact(wrapped, '0' * 64)

    def test_independent_verifier_refuses_incomplete_git_source_report(self):
        import subprocess
        revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        with self.assertRaises(ValueError):
            verifier.verify_sources({}, revision, self.root, None)

    def test_rejects_corruption_symlinks_and_unlisted_material(self):
        path = self.root / 'LICENSE'
        path.write_bytes(b'bad')
        with self.assertRaises(ValueError): installer.verified_files(self.root)
        path.unlink()
        path.symlink_to(self.root / 'README.md')
        with self.assertRaises(ValueError): installer.verified_files(self.root)
        path.unlink(); path.write_bytes(b'LICENSE')
        (self.root / 'unlisted').write_bytes(b'extra')
        with self.assertRaises(ValueError): installer.verified_files(self.root)

    def test_rejects_path_traversal_and_installation_id_escape(self):
        self.manifest['files']['../escape'] = {'sha256': '0' * 64, 'mode': 0o644}
        self.write_manifest()
        with self.assertRaises(ValueError): installer.verified_files(self.root)
        del self.manifest['files']['../escape']
        self.manifest['installation_id'] = '../arbitrary'
        self.write_manifest()
        with self.assertRaises(ValueError): installer.verified_files(self.root)


@unittest.skipUnless(os.name == 'posix', 'Linux build process groups')
class BoundedBuild(unittest.TestCase):
    def test_timeout_retires_compiler_descendants_before_returning(self):
        import fcntl
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lock = root / 'compiler.lock'
            # A child holds a real OS lock while alive. Its release proves that
            # terminating only the Cargo-shaped parent cannot satisfy this case.
            compiler = "import fcntl,sys,time; f=open(sys.argv[1],'w'); fcntl.flock(f,fcntl.LOCK_EX); f.write('ready'); f.flush(); time.sleep(30)"
            parent = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',sys.argv[1],sys.argv[2]]); time.sleep(30)"
            result = build_pipeline.run_bounded(
                [sys.executable, '-c', parent, compiler, str(lock)],
                root / 'build.log', os.environ.copy(), 2)
            self.assertEqual(result, 'timeout')
            self.assertEqual(lock.read_text(), 'ready')
            with lock.open() as stream:
                deadline = time.monotonic() + 2
                while True:
                    try:
                        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        break
                    except BlockingIOError:
                        if time.monotonic() >= deadline:
                            self.fail('Compiler descendant retained its OS lock')
                        time.sleep(.01)

    def test_completed_build_preserves_its_exit_status(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(build_pipeline.run_bounded(
                [sys.executable, '-c', 'raise SystemExit(7)'],
                Path(directory) / 'build.log', os.environ.copy(), 2), 7)


class BuildPhaseGuards(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.evidence = self.root / 'evidence'
        self.evidence.mkdir()
        self.inputs = {'crates/example/src/lib.rs': {'sha256': '1' * 64, 'mode': 0o644}}
        source = self.evidence / 'source-inputs.json'
        source.write_text(json.dumps(self.inputs) + '\n')
        self.report = {'source_revision': 'a' * 40, 'requested_source_revision': 'a' * 40,
                       'controller_source_revision': 'b' * 40,
                       'source_inputs_sha256': installer.digest(source), 'source_count': 1,
                       'steps': [{'name': 'recovery-tests', 'exit_code': 0}],
                       'compilation_completed': True, 'passed': False}
        self.report_path = self.evidence / 'build.json'
        self.report_path.write_text(json.dumps(self.report))
        self.patchers = [
            mock.patch.multiple(build_pipeline, ROOT=self.root, EVIDENCE=self.evidence, PRIVATE=self.root / 'private'),
            mock.patch('sys.argv', ['check-linux-package-build.py', '--phase', 'package']),
            mock.patch.dict(os.environ, {'CONNECT_INSTALLATION_SOURCE_REVISION': 'a' * 40,
                                         'GITHUB_SHA': 'b' * 40, 'GITHUB_OUTPUT': str(self.root / 'outputs')}),
            mock.patch.object(build_pipeline.os, 'umask', return_value=0o022),
            mock.patch.object(build_pipeline.subprocess, 'check_output', return_value='a' * 40 + '\n'),
        ]
        for patcher in self.patchers:
            patcher.start()
            self.addCleanup(patcher.stop)

    def test_package_phase_refuses_a_changed_source_manifest_without_building(self):
        self.report['source_inputs_sha256'] = '0' * 64
        self.report_path.write_text(json.dumps(self.report))
        with mock.patch.object(build_pipeline.runpy, 'run_path', return_value={'sources': lambda: self.inputs}), mock.patch.object(build_pipeline, 'run_bounded') as build:
            with self.assertRaisesRegex(AssertionError, 'Recovery source manifest changed'):
                build_pipeline.main()
            build.assert_not_called()

    def test_changes_between_phases_are_not_freshened_and_cannot_save_a_cache(self):
        changed = {'crates/example/src/lib.rs': {'sha256': '2' * 64, 'mode': 0o644}}
        with mock.patch.object(build_pipeline.runpy, 'run_path', return_value={'sources': lambda: changed}), mock.patch.object(build_pipeline, 'run_bounded') as build:
            with self.assertRaisesRegex(AssertionError, 'Source changed between build phases'):
                build_pipeline.main()
            build.assert_not_called()
        report = json.loads(self.report_path.read_text())
        self.assertFalse(report['source_inputs_unchanged'])
        self.assertFalse(report['cacheable_build'])
        self.assertEqual(json.loads((self.evidence / 'source-inputs.json').read_text()), self.inputs)


if __name__ == '__main__':
    unittest.main()
