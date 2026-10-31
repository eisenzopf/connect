"""Real Cargo external resolution and candidate source/archive boundaries."""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.sdk_rust_package import install_archive, prepare_package, sha256, standalone_manifest, write_toml


class RustCandidate(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='connect-rust-candidate-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / 'source'; self.source.mkdir()
        (self.source / 'LICENSE').write_text('Exact original fixture license\n')
        (self.source / 'Cargo.toml').write_text('''[workspace]
members=["client","types"]
[workspace.package]
version="0.1.0"
edition="2021"
license="MIT"
[workspace.dependencies]
fixture-types={path="types",version="0.1.0",default-features=false,features=["base"]}
[workspace.lints.rust]
unsafe_code="forbid"
''')
        for name in ('client', 'types'):
            path = self.source / name
            (path / 'src').mkdir(parents=True)
            (path / 'Cargo.toml').write_text('[package]\nname="fixture-' + name + '"\nversion.workspace=true\nedition.workspace=true\nlicense.workspace=true\n[lints]\nworkspace=true\n')
            (path / 'src/lib.rs').write_text('pub fn number()->u32{42}\n')
        with (self.source / 'client/Cargo.toml').open('a') as stream:
            stream.write('[dependencies]\nfixture-types={workspace=true,features=["extra"]}\n[dev-dependencies]\nforward={path="../not-published"}\n')
        with (self.source / 'types/Cargo.toml').open('a') as stream:
            stream.write('[features]\nbase=[]\nextra=[]\n')
        self.vendor = self.root / 'vendor'; self.vendor.mkdir()
        self.archives = self.root / 'archives'; self.archives.mkdir()

    def test_real_cargo_resolves_extracted_archives_outside_source_with_empty_home(self):
        for name in ('client', 'types'):
            prepare_package(self.source / name / 'Cargo.toml', self.vendor, self.archives, 1234)
        # Use the actual archives, not the producer's staging directories.
        shutil.rmtree(self.vendor); self.vendor.mkdir()
        for archive in self.archives.glob('*.crate'):
            name = archive.name.removesuffix('-0.1.0.crate')
            install_archive(archive, self.vendor, name, '0.1.0', sha256(archive))
        shutil.rmtree(self.source)
        (self.root / 'src').mkdir(); (self.root / 'src/main.rs').write_text('fn main(){}\n')
        write_toml(self.root / 'Cargo.toml', {'package': {'name': 'outside-consumer', 'version': '0.1.0', 'edition': '2021'}, 'workspace': {}, 'dependencies': {'fixture-client': '=0.1.0'}})
        (self.root / '.cargo').mkdir()
        write_toml(self.root / '.cargo/config.toml', {'source': {'crates-io': {'replace-with': 'fixture'}, 'fixture': {'directory': 'vendor'}}})
        home = self.root / 'empty-cargo-home'
        self.assertFalse(home.exists())
        result = subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=self.root,
                                env=dict(os.environ, CARGO_HOME=str(home)), capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        packages = {p['name']: p for p in json.loads(result.stdout)['packages']}
        self.assertEqual(set(packages), {'outside-consumer', 'fixture-client', 'fixture-types'})
        self.assertEqual(packages['fixture-types']['features'], {'base': [], 'extra': []})
        self.assertTrue(all(not d.get('path') for p in packages.values() for d in p['dependencies']))
        self.assertTrue(packages['fixture-client']['source'].startswith('registry+'))
        self.assertEqual((self.vendor / 'fixture-client-0.1.0/LICENSE').read_text(), 'Exact original fixture license\n')

    def test_inheritance_features_versions_lints_and_forward_development_dependency(self):
        value, paths, _ = standalone_manifest(self.source / 'client/Cargo.toml')
        dependency = value['dependencies']['fixture-types']
        self.assertEqual(dependency, {'version': '=0.1.0', 'default-features': False, 'features': ['base', 'extra']})
        self.assertEqual(paths, [(self.source / 'types/Cargo.toml').resolve()])
        self.assertEqual(value['dev-dependencies'], {})
        self.assertEqual(value['lints'], {'rust': {'unsafe_code': 'forbid'}})
        self.assertFalse(value['package']['publish'])

    def test_archive_determinism_and_corrupt_original_refusal(self):
        first, _ = prepare_package(self.source / 'client/Cargo.toml', self.vendor, self.archives, 1234)
        shutil.rmtree(self.vendor); self.vendor.mkdir()
        archive = self.archives / first['archive']; original = archive.read_bytes(); archive.unlink()
        second, _ = prepare_package(self.source / 'client/Cargo.toml', self.vendor, self.archives, 1234)
        self.assertEqual(first, second)
        self.assertEqual(original, archive.read_bytes())
        archive.write_bytes(original + b'corruption')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            install_archive(archive, self.root, 'fixture-client', '0.1.0', first['sha256'])

    def test_symlink_and_archive_escape_refused(self):
        (self.source / 'client/private-link').symlink_to(self.source / 'LICENSE')
        with self.assertRaisesRegex(ValueError, 'Symlink'):
            prepare_package(self.source / 'client/Cargo.toml', self.vendor, self.archives, 1234)
        archive = self.archives / 'escape.crate'
        with tarfile.open(archive, 'w:gz') as stream:
            entry = tarfile.TarInfo('fixture-client-0.1.0/../../escape'); entry.size = 1
            stream.addfile(entry, io.BytesIO(b'x'))
        with self.assertRaisesRegex(ValueError, 'path'):
            install_archive(archive, self.root, 'fixture-client', '0.1.0', sha256(archive))
        self.assertFalse((self.root.parent / 'escape').exists())


if __name__ == '__main__':
    unittest.main()
