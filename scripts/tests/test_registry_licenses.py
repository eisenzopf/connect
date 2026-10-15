"""Exact notice discovery/provenance and selected-graph coverage guards."""
import hashlib
import json
import pathlib
import sys
import tempfile
import tarfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib.registry_licenses import (copy_registry_notices, digest, notice_paths,
                                   selected_dependency_ids, REGISTRY)


class RegistryLicenseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.crate = self.root / 'registry/src/example/sample-1.0.0'
        self.crate.mkdir(parents=True)
        self.archive = self.root / 'registry/cache/example/sample-1.0.0.crate'
        self.archive.parent.mkdir(parents=True)
        self.runtime = self.root / 'runtime'
        self.package = {'id': 'registry#sample@1.0.0', 'name': 'sample', 'version': '1.0.0',
                        'manifest_path': str(self.crate / 'Cargo.toml'), 'source': REGISTRY,
                        'license': 'MIT', 'repository': 'https://github.com/example/sample', 'license_file': None}
        (self.crate / 'Cargo.toml').write_text('[package]\nname="sample"\nversion="1.0.0"\n')
        (self.root / 'Cargo.lock').write_text('[[package]]\nname="sample"\nversion="1.0.0"\nsource="'+REGISTRY+'"\nchecksum="'+'a'*64+'"\n')
        (self.root / 'dependencies').mkdir()
        self.lock_path = self.root / 'dependencies/registry-license-lock.json'
        self.lock = {'version': 1, 'packages': {}}
        self.write_lock()
        self.checksums()

    def write_lock(self):
        self.lock_path.write_text(json.dumps(self.lock))

    def checksums(self):
        with tarfile.open(self.archive, 'w:gz') as tar:
            for path in sorted(self.crate.rglob('*')):
                if path.is_file():
                    tar.add(path, arcname='sample-1.0.0/' + path.relative_to(self.crate).as_posix())
        self.checksum = digest(self.archive)
        (self.root / 'Cargo.lock').write_text('[[package]]\nname="sample"\nversion="1.0.0"\nsource="'+REGISTRY+'"\nchecksum="'+self.checksum+'"\n')

    def run_collection(self, selected=True):
        return copy_registry_notices(self.root, self.runtime, {'packages': [self.package]},
                                     {self.package['id']} if selected else set())

    def fallback(self):
        (self.crate / '.cargo_vcs_info.json').write_text(json.dumps({'git': {'sha1': 'b'*40}, 'path_in_vcs': 'crate'}))
        self.checksums()
        license_path = self.root / 'dependencies/LICENSE'
        license_path.write_text('Exact upstream transition, copyright and complete license text.\n')
        self.entry = {k:self.package[k] for k in ['name','version','source','license','repository']}
        self.entry.update(checksum=self.checksum, github_repository='example/sample', revision='b'*40, path_in_vcs='crate',
                          license_files=[{'path':'dependencies/LICENSE','sha256':digest(license_path),
                                          'upstream_path':'LICENSE','url':'https://raw.githubusercontent.com/example/sample/'+'b'*40+'/LICENSE'}])
        self.lock['packages']['sample-1.0.0'] = self.entry
        self.write_lock()

    def test_nested_license_directory_and_explicit_nonstandard_file_preserve_bytes(self):
        (self.crate/'LICENSES').mkdir()
        (self.crate/'LICENSES/MIT.txt').write_bytes(b'Copyright precise owner\nMIT text\n')
        (self.crate/'legal').mkdir()
        (self.crate/'legal/grant.txt').write_bytes(b'Exact special grant\n')
        self.package['license_file'] = str(self.crate/'legal/grant.txt')
        self.checksums()
        result = self.run_collection()[0]
        self.assertEqual(len(result['texts']), 2)
        for name in ['LICENSES/MIT.txt','legal/grant.txt']:
            self.assertEqual((self.runtime/'licenses/registry/sample-1.0.0'/name).read_bytes(), (self.crate/name).read_bytes())

    def test_exact_fallback_keeps_complete_notice_and_provenance(self):
        self.fallback()
        result = self.run_collection()[0]
        self.assertEqual((self.runtime/result['texts'][0]).read_bytes(), (self.root/'dependencies/LICENSE').read_bytes())
        self.assertEqual(result['repository_license_provenance'], self.entry)
        self.assertEqual((self.runtime/'licenses/provenance/dependencies/registry-license-lock.json').read_bytes(), self.lock_path.read_bytes())

    def test_missing_selected_coverage_fails_unselected_is_explicit(self):
        (self.crate/'src').mkdir()
        (self.crate/'src/license.rs').write_text('fn unrelated() {}')
        self.checksums()
        with self.assertRaisesRegex(ValueError, 'no license text'):
            self.run_collection()
        self.package['license'] = None
        result = self.run_collection(False)[0]
        self.assertEqual(result['coverage'], 'outside-selected-graph-declaration-only')
        self.assertFalse(result['selected_normal_build_dependency'])
        self.assertEqual(result['texts'], [])

    def test_version_source_checksum_and_repository_mismatches_fail(self):
        self.fallback()
        for key in ['version','source','checksum','repository','name','license']:
            with self.subTest(key=key):
                value = self.entry[key];self.entry[key] = 'wrong';self.write_lock()
                with self.assertRaisesRegex(ValueError, 'identity mismatch'):
                    self.run_collection()
                self.entry[key] = value
        self.write_lock()

    def test_vcs_revision_path_and_exact_url_mismatches_fail(self):
        self.fallback()
        for key in ['revision','path_in_vcs']:
            with self.subTest(key=key):
                value = self.entry[key];self.entry[key] = 'wrong';self.write_lock()
                with self.assertRaisesRegex(ValueError, 'VCS'):
                    self.run_collection()
                self.entry[key] = value
        self.entry['license_files'][0]['url'] = 'https://example.com/LICENSE';self.write_lock()
        with self.assertRaisesRegex(ValueError, 'URL'):
            self.run_collection()

    def test_modified_crate_or_pinned_license_bytes_fail(self):
        (self.crate/'LICENSE').write_text('original');self.checksums()
        (self.crate/'LICENSE').write_text('tampered')
        with self.assertRaisesRegex(ValueError, 'file checksum'):
            self.run_collection()
        (self.crate/'LICENSE').unlink();self.fallback()
        (self.root/'dependencies/LICENSE').write_text('tampered')
        with self.assertRaisesRegex(ValueError, 'digest mismatch'):
            self.run_collection()
        self.entry['license_files'][0]['sha256'] = digest(self.root/'dependencies/LICENSE');self.write_lock()
        with self.archive.open('ab') as stream: stream.write(b'tampered archive')
        with self.assertRaisesRegex(ValueError, 'archive checksum'):
            self.run_collection()

    def test_path_traversal_external_explicit_and_symlink_fail(self):
        external=self.root/'external';external.write_text('outside')
        for path in ['../external',str(external)]:
            with self.subTest(path=path):
                self.package['license_file']=path
                with self.assertRaises(ValueError): self.run_collection()
        self.package['license_file']=None
        (self.crate/'LICENSE').symlink_to(external)
        with self.assertRaisesRegex(ValueError,'Symlink'):self.run_collection()
        (self.crate/'LICENSE').unlink();self.fallback()
        self.entry['license_files'][0]['path']='../external';self.write_lock()
        with self.assertRaisesRegex(ValueError,'Unsafe'):self.run_collection()

    def test_stale_mapping_and_unbound_selected_graph_fail(self):
        self.lock['packages']['stale-1.0.0']={};self.write_lock()
        with self.assertRaisesRegex(ValueError,'stale'):self.run_collection(False)
        self.lock['packages']={};self.write_lock()
        with self.assertRaisesRegex(ValueError,'Selected graph'):
            copy_registry_notices(self.root,self.runtime,{'packages':[self.package]},{'unknown'})

    def test_target_filtered_graph_includes_normal_build_macros_not_dev_or_disconnected_nonhost(self):
        packages=[{'id':n,'name':n} for n in ['server','example','video','normal','macro','dev','windows']]
        dep=lambda p,k:{'pkg':p,'dep_kinds':[{'kind':k,'target':None}]}
        nodes=[{'id':p['id'],'deps':[]} for p in packages]
        nodes[0]['deps']=[dep('normal',None),dep('macro','build'),dep('dev','dev')]
        # Nonhost package can remain in packages but has no target-filtered edge.
        metadata={'packages':packages,'resolve':{'nodes':nodes}}
        self.assertEqual(selected_dependency_ids(metadata,['server','example','video']),{'server','example','video','normal','macro'})
        self.assertNotIn('video',selected_dependency_ids(metadata,['server','example']))
        with self.assertRaisesRegex(ValueError,'runtime root'):selected_dependency_ids(metadata,['absent'])


if __name__ == '__main__':
    unittest.main()
