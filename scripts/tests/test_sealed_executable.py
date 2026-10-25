import concurrent.futures
import importlib.util
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('sealed_executable', ROOT / 'scripts/lib/sealed_executable.py')
seal = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(seal)


class SealedExecutableTests(unittest.TestCase):
    def setUp(self):
        (ROOT / '.local').mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(dir=ROOT / '.local')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = self.root / 'build-output'
        self.source.write_text('#!/bin/sh\nexit 0\n')
        self.source.chmod(0o755)
        self.sha = seal.digest(self.source)
        self.pool = self.root / 'pool'

    def make(self, name):
        output = self.root / name
        seal.seal_executable(self.source, output, self.sha, pool=self.pool)
        return output

    def test_build_rewrite_cannot_change_shared_sealed_execution(self):
        first, second = self.make('first'), self.make('second')
        self.assertEqual(first.stat().st_ino, second.stat().st_ino)
        self.assertNotEqual(first.stat().st_ino, self.source.stat().st_ino)
        self.source.write_text('#!/bin/sh\nexit 7\n')
        self.assertEqual(subprocess.run([str(self.source)]).returncode, 7)
        self.assertEqual(subprocess.run([str(first)]).returncode, 0)
        self.assertEqual(seal.digest(second), self.sha)
        self.assertEqual(stat.S_IMODE(first.stat().st_mode), 0o755)

    def test_corrupt_pool_refuses_without_creating_evidence(self):
        first = self.make('first')
        first.write_text('corrupt')
        with self.assertRaisesRegex(ValueError, 'corrupt'):
            self.make('refused')
        self.assertFalse((self.root / 'refused').exists())

    def test_concurrent_sealers_share_one_independent_copy(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
            outputs = list(executor.map(self.make, [f'run-{i}' for i in range(8)]))
        self.assertEqual(len({p.stat().st_ino for p in outputs}), 1)
        self.assertEqual(len(list(self.pool.iterdir())), 1)
        self.assertTrue(all(seal.digest(p) == self.sha for p in outputs))

    def test_existing_destination_is_never_overwritten(self):
        first = self.make('first')
        with self.assertRaises(FileExistsError):
            self.make('first')
        self.assertEqual(seal.digest(first), self.sha)


if __name__ == '__main__':
    unittest.main()
