"""Pinned git preparation and standalone Cargo-root override guards."""
import hashlib
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prepare = load('prepare_dependency_roots_test', 'scripts/prepare-dependencies.py')
checker = load('check_parley_roots_test', 'scripts/check-parley.py')


class GitPreparation(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        (self.root / 'dependencies').mkdir()
        self.lock = {'format': 1, 'sources': {}, 'patches': []}
        for name in ('rvoip', 'parley'):
            dest = self.root / '.deps' / name
            dest.mkdir(parents=True)
            self.git(dest, 'init', '--quiet')
            self.git(dest, 'config', 'user.name', 'Fixture')
            self.git(dest, 'config', 'user.email', 'fixture@example.invalid')
            self.git(dest, 'remote', 'add', 'origin', f'https://example.invalid/{name}.git')
            (dest / 'source.txt').write_text('original\n')
            self.git(dest, 'add', 'source.txt')
            self.git(dest, 'commit', '--quiet', '-m', 'fixture')
            self.lock['sources'][name] = {
                'url': f'https://example.invalid/{name}.git',
                'revision': self.git(dest, 'rev-parse', 'HEAD')}
            body = ('diff --git a/source.txt b/source.txt\n--- a/source.txt\n'
                    '+++ b/source.txt\n@@ -1 +1 @@\n-original\n+' + name + '\n')
            p = self.root / f'{name}.patch'
            p.write_text(body)
            item = {'path': p.name, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
            if name == 'parley':
                item['target'] = name
            self.lock['patches'].append(item)
        self.save_lock()

    def git(self, path, *args):
        return subprocess.check_output(['git', '-C', str(path), *args], text=True).strip()

    def save_lock(self):
        (self.root / 'dependencies/source-lock.json').write_text(json.dumps(self.lock))

    def run_prepare(self):
        with patch.object(prepare, 'ROOT', self.root), patch.object(sys, 'argv', ['prepare']):
            prepare.main()

    def test_default_and_named_targets_apply_once_and_reuse(self):
        self.run_prepare()
        self.run_prepare()
        for name in ('rvoip', 'parley'):
            dest = self.root / '.deps' / name
            self.assertEqual((dest / 'source.txt').read_text(), name + '\n')
            self.assertTrue((dest / '.git/connect-prepared.json').is_file())

    def test_modified_named_source_is_rejected(self):
        self.run_prepare()
        (self.root / '.deps/parley/source.txt').write_text('changed\n')
        with self.assertRaisesRegex(SystemExit, 'Prepared source changed'):
            self.run_prepare()

    def test_unexpected_named_source_is_rejected(self):
        self.run_prepare()
        (self.root / '.deps/parley/extra.txt').write_text('extra\n')
        with self.assertRaisesRegex(SystemExit, 'Unexpected prepared source'):
            self.run_prepare()

    def test_changed_lock_does_not_reset_prepared_sources(self):
        self.run_prepare()
        self.lock['format'] = 2
        self.save_lock()
        with self.assertRaisesRegex(SystemExit, 'Dependency lock changed'):
            self.run_prepare()
        self.assertEqual((self.root / '.deps/parley/source.txt').read_text(), 'parley\n')

    def test_initial_dirty_named_source_is_not_overwritten(self):
        (self.root / '.deps/parley/source.txt').write_text('private edit\n')
        with self.assertRaisesRegex(SystemExit, 'unrecorded changes'):
            self.run_prepare()
        self.assertEqual((self.root / '.deps/parley/source.txt').read_text(), 'private edit\n')


class CargoRootOverrides(unittest.TestCase):
    def setUp(self):
        self.root = pathlib.Path('/fixture')
        self.lock = {'sources': {name: {'version': '1.2.3'} for name in checker.OVERRIDES}}
        self.metadata = {'packages': [{'name': name, 'version': '1.2.3', 'source': None,
                                      'manifest_path': f'/fixture/.deps/{name}/Cargo.toml'}
                                     for name in checker.OVERRIDES]}

    def test_exact_prepared_paths_are_accepted(self):
        checker.verify_metadata(self.metadata, self.root, self.lock)

    def test_registry_source_is_rejected(self):
        self.metadata['packages'][0]['source'] = 'registry+https://example.invalid/index'
        with self.assertRaises(ValueError):
            checker.verify_metadata(self.metadata, self.root, self.lock)

    def test_wrong_prepared_directory_is_rejected(self):
        self.metadata['packages'][0]['manifest_path'] = '/other/rtc-dtls/Cargo.toml'
        with self.assertRaises(ValueError):
            checker.verify_metadata(self.metadata, self.root, self.lock)

    def test_wrong_pinned_version_is_rejected(self):
        self.metadata['packages'][0]['version'] = '1.2.4'
        with self.assertRaises(ValueError):
            checker.verify_metadata(self.metadata, self.root, self.lock)

    def test_missing_or_duplicate_package_is_rejected(self):
        missing = {'packages': self.metadata['packages'][1:]}
        duplicate = {'packages': self.metadata['packages'] + self.metadata['packages'][:1]}
        for metadata in (missing, duplicate):
            with self.assertRaises(ValueError):
                checker.verify_metadata(metadata, self.root, self.lock)


if __name__ == '__main__':
    unittest.main()
