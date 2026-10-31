"""Actual native archive installation, tamper refusal and extraction boundaries."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.sdk_candidate import digest, install_runtime


class NativeCandidate(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / 'candidate.tar.gz'
        self.proof = {'passed': True, 'source_inputs_unchanged': True, 'production_release': True,
                      'test_support': False, 'source_revision': 'a' * 40, 'version': '0.1.0',
                      'target': 'x86_64-unknown-linux-gnu', 'binary_sha256': hashlib.sha256(b'owned executable').hexdigest()}
        self.identity = 'connect-runtime-0.1.0-x86_64-unknown-linux-gnu-' + 'a' * 12
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

    def test_self_consistently_repacked_corrupt_binary_still_refused(self):
        self.pack(binary=b'altered executable')
        with self.assertRaisesRegex(ValueError, 'payload differs'):
            install_runtime(self.archive, self.proof, self.root / 'installed')

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
