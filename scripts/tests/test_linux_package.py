"""Package integrity and deterministic archive boundaries; no root service effects."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import zipfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    import sys
    sys.path.insert(0, str(ROOT / 'scripts'))
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


installer = load('linux_installer', 'deploy/linux/install.py')
packager = load('linux_packager', 'scripts/package-linux.py')
verifier = load('linux_verifier', 'scripts/verify-linux-installation.py')


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


if __name__ == '__main__':
    unittest.main()
