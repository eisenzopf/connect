"""Actual native archive installation, tamper refusal and extraction boundaries."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
import runpy
import tarfile
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.sdk_candidate import digest, install_runtime, runtime_inventory


class NativeCandidate(unittest.TestCase):
    def test_full_artifact_binds_runtime_to_its_own_inventory_among_three_components(self):
        verifier = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'verify-sdk-runtime.py'))
        for with_runtime_source in (True, False):
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, 'w') as archive:
                archive.writestr('target/runtime.json', '{"source_revision":"original"}')
                archive.writestr('target/installed/source-inputs.json', b'installed')
                archive.writestr('target/offline/source-inputs.json', b'offline')
                if with_runtime_source: archive.writestr('target/source-inputs.json', b'original-native-inputs')
            stream.seek(0)
            with zipfile.ZipFile(stream) as archive:
                if with_runtime_source:
                    proof, original = verifier['runtime_evidence'](archive)
                    self.assertEqual(proof, {'source_revision': 'original'})
                    self.assertEqual(original, b'original-native-inputs')
                else:
                    with self.assertRaisesRegex(ValueError, 'own unique sibling'):
                        verifier['runtime_evidence'](archive)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / 'candidate.tar.gz'
        self.proof = {'passed': True, 'source_inputs_unchanged': True, 'production_release': True,
                      'test_support': False, 'source_revision': 'a' * 40, 'version': '0.1.0',
                      'target': 'aarch64-unknown-linux-gnu', 'binary_sha256': hashlib.sha256(b'owned executable').hexdigest()}
        self.identity = 'connect-runtime-0.1.0-aarch64-unknown-linux-gnu-' + 'a' * 12
        self.files = {'bin/connect': {'sha256': self.proof['binary_sha256'], 'mode': 0o755},
                      'LICENSE': {'sha256': hashlib.sha256(b'original license').hexdigest(), 'mode': 0o644}}
        self.proof['files'] = self.files

    def pack(self, extra=None, binary=b'owned executable'):
        manifest = json.dumps(dict(self.proof, files=self.files)).encode()
        self.proof['manifest_sha256'] = hashlib.sha256(manifest).hexdigest()
        with tarfile.open(self.archive, 'w:gz') as stream:
            for name, body in [('bin/connect', binary), ('LICENSE', b'original license'), ('manifest.json', manifest)]:
                entry = tarfile.TarInfo(self.identity + '/' + name); entry.size = len(body)
                entry.mode = 0o755 if name == 'bin/connect' else 0o644
                stream.addfile(entry, io.BytesIO(body))
            if extra:
                stream.addfile(extra, io.BytesIO(b''))
        self.proof['archive_sha256'] = digest(self.archive)

    def test_installs_exact_files_modes_and_source_bound_manifest(self):
        self.pack(); destination = self.root / 'installed'
        observed = install_runtime(self.archive, self.proof, destination)
        self.assertEqual(observed['source_revision'], 'a' * 40)
        self.assertEqual((destination / 'bin/connect').read_bytes(), b'owned executable')
        self.assertEqual((destination / 'bin/connect').stat().st_mode & 0o777, 0o755)
        self.assertEqual((destination / 'LICENSE').read_bytes(), b'original license')

    def test_macos_native_archive_provenance_must_match_the_archived_manifest(self):
        self.proof.update(target='aarch64-apple-darwin',
            opus_native_build={'mode': 'bundled-static', 'native_archive_sha256': 'b'*64})
        self.identity = 'connect-runtime-0.1.0-aarch64-apple-darwin-' + 'a'*12
        self.pack()
        installed = install_runtime(self.archive, self.proof, self.root / 'matching')
        self.assertEqual(installed['opus_native_build'], self.proof['opus_native_build'])
        self.proof['opus_native_build']['native_archive_sha256'] = 'c'*64
        with self.assertRaisesRegex(ValueError, 'payload differs'):
            install_runtime(self.archive, self.proof, self.root / 'mismatching')

    def test_self_consistently_repacked_corrupt_binary_still_refused(self):
        self.pack(binary=b'altered executable')
        with self.assertRaisesRegex(ValueError, 'payload differs'):
            install_runtime(self.archive, self.proof, self.root / 'installed')

    def test_actual_producer_archive_modes_match_manifest_under_private_umask(self):
        runtime = self.root / self.identity
        (runtime / 'bin').mkdir(parents=True)
        (runtime / 'bin/connect').write_bytes(b'owned executable')
        (runtime / 'bin/connect').chmod(0o755)
        (runtime / 'LICENSE').write_bytes(b'original license')
        (runtime / 'LICENSE').chmod(0o600)
        self.proof['files'] = runtime_inventory(runtime)
        body = json.dumps(self.proof).encode()
        (runtime / 'manifest.json').write_bytes(body)
        (runtime / 'manifest.json').chmod(0o600)
        self.proof['manifest_sha256'] = hashlib.sha256(body).hexdigest()
        root = Path(__file__).resolve().parents[2]
        producer = runpy.run_path(str(root / 'scripts/package-linux.py'))
        producer['archive'](runtime, self.archive, 1234)
        self.proof['archive_sha256'] = digest(self.archive)
        destination = self.root / 'installed'
        install_runtime(self.archive, self.proof, destination)
        self.assertEqual((runtime / 'LICENSE').stat().st_mode & 0o777, 0o600)
        self.assertEqual((destination / 'LICENSE').stat().st_mode & 0o777, 0o644)

    def test_escape_symlink_and_canonical_duplicate_are_refused(self):
        for index, name in enumerate([self.identity + '/../escape', self.identity + '/bin//connect', self.identity + '/link']):
            entry = tarfile.TarInfo(name)
            if name.endswith('/link'):
                entry.type = tarfile.SYMTYPE; entry.linkname = '/outside'
            self.pack(extra=entry)
            with self.assertRaises(ValueError):
                install_runtime(self.archive, self.proof, self.root / str(index))
        self.assertFalse((self.root / 'escape').exists())


if __name__ == '__main__':
    unittest.main()
