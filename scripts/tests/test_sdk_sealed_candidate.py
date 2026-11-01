"""Inspect real tar streams and refuse tampering or incomplete owner evidence."""
import copy
import hashlib
import io
import json
from pathlib import Path
import runpy
import sys
import tarfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
VERIFIER = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'verify-sdk-candidate.py'))


class SealedCandidate(unittest.TestCase):
    def test_generated_package_readme_is_bound_to_original_producer_without_executing_it(self):
        body = '# Connect SDK candidate\nOriginal generated instructions.\n'
        source = "raise RuntimeError('must never execute')\n(candidate/'README.md').write_text(" + repr(body) + ")\n"
        self.assertEqual(VERIFIER['generated_readme'](source), body.encode())
        self.assertIsNone(VERIFIER['original_path']('README.md'))
        self.assertEqual(VERIFIER['original_path']('docs/developer/rust-sdk.md'), 'docs/developer/rust-sdk.md')
        self.assertEqual(VERIFIER['original_path']('sources/python-source/connect_sdk/operational.py'), 'sdk/python/connect_sdk/operational.py')
        with self.assertRaises(ValueError): VERIFIER['generated_readme']('raise RuntimeError("unrelated source")')

    def components(self):
        revision, target = 'a' * 40, 'fixture'
        runtime = {'passed': True, 'source_inputs_unchanged': True, 'source_revision': revision,
                   'target': target, 'version': '0.1.0', 'archive_name': 'runtime.tar.gz',
                   'archive_sha256': 'runtime', 'binary_sha256': 'binary', 'manifest_sha256': 'runtime-manifest'}
        installed = {'passed': True, 'source_inputs_unchanged': True, 'source_revision': revision,
                     'target': target, 'consumer_outside_repository': True, 'original_server_joined': True,
                     'native_helpers_joined': 2, 'steps': [{'exit_code': 0}],
                     'runtime_archive_sha256': 'runtime', 'node_archive_sha256': 'node',
                     'python_wheel_sha256': 'python', 'rust_source_manifest_sha256': 'rust-manifest',
                     'rust': {'cold_offline_crate_installation': True, 'peer_capacity_reclaimed': True}}
        for role in ('node', 'python'):
            installed[role] = {'peer_capacity_reclaimed': True,
                               'helper': {'native_helpers_joined': 1, 'actual_default_executable': True}}
        offline = {'passed': True, 'source_inputs_unchanged': True, 'source_revision': revision,
                   'consumer_outside_repository': True, 'empty_caches': True, 'steps': [{'exit_code': 0}],
                   'typescript_distribution_byte_identical': True, 'python_wheel_byte_identical': True,
                   'manifest_sha256': 'offline-manifest'}
        node = {'passed': True, 'source_inputs_unchanged': True, 'consumer_outside_repository': True,
                'empty_cache': True, 'external_declarations_checked': True, 'version': '0.1.0',
                'archive_name': 'node.tgz', 'archive_sha256': 'node', 'dependency_archives': []}
        python = {'passed': True, 'source_inputs_unchanged': True, 'consumer_outside_repository': True,
                  'consumer': {'owned_workers_joined': 6}, 'wheel_name': 'python.whl', 'wheel_sha256': 'python'}
        manifest = {'source_revision': revision, 'target': target, 'version': '0.1.0',
                    'candidate_installation_verified': True, 'publication_authorized': False,
                    'release_ready': False, 'task_complete': False, 'whole_gate_qualified': False}
        publication = dict(manifest, registry_account_ownership_verified=False,
                           runtime={'sha256': 'runtime', 'server_and_helper_sha256': 'binary'},
                           npm={'sha256': 'node'}, pypi={'sha256': 'python'})
        proofs = {'runtime': runtime, 'installed': installed, 'offline': offline}
        docs = {'manifest.json': manifest, 'publication.json': publication,
                **{'verification/' + name + '.json': copy.deepcopy(value)
                   for name, value in dict(proofs, node=node, python=python).items()},
                'sdk/rust/manifest.json': {'source_revision': revision, 'files': {}},
                'sources/manifest.json': {'source_revision': revision, 'files': {}}}
        files = {name: {'sha256': value} for name, value in {
            'runtime/runtime.tar.gz': 'runtime', 'sdk/node/node.tgz': 'node', 'sdk/python/python.whl': 'python',
            'sdk/rust/manifest.json': 'rust-manifest', 'sources/manifest.json': 'offline-manifest'}.items()}
        return files, docs, proofs, revision, target, dict(runtime, production_build_verified=True)

    def archive(self, *, altered=None, extra=None):
        payload = {name: json.dumps({}).encode() for name in VERIFIER['DOCUMENTS'] - {'manifest.json'}}
        payload['README.md'] = b'original package description'
        files = {name: {'sha256': hashlib.sha256(body).hexdigest(), 'mode': 0o644} for name, body in payload.items()}
        manifest = json.dumps({'files': files}).encode()
        payload['manifest.json'] = manifest
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode='w:gz') as archive:
            for name, body in payload.items():
                member = tarfile.TarInfo('candidate/' + name)
                member.size = len(body); member.mode = 0o644
                if altered == name: member.mode = 0o600
                archive.addfile(member, io.BytesIO(body))
            if extra is not None: archive.addfile(extra, io.BytesIO())
        stream.seek(0)
        return stream, hashlib.sha256(manifest).hexdigest(), files

    def test_actual_regular_file_bytes_and_portable_modes_match(self):
        stream, manifest_sha, expected = self.archive()
        actual, _ = VERIFIER['inspect_archive'](stream, 'candidate', manifest_sha)
        self.assertEqual(actual, expected)

    def test_private_mode_cannot_be_promoted_from_a_content_only_match(self):
        stream, manifest_sha, _ = self.archive(altered='README.md')
        with self.assertRaisesRegex(ValueError, 'mode'):
            VERIFIER['inspect_archive'](stream, 'candidate', manifest_sha)

    def test_traversal_alias_duplicate_and_links_never_extract(self):
        for name, link in [('candidate/../outside', False), ('candidate//README.md', False),
                           ('candidate/README.md', False), ('candidate/link', True)]:
            with self.subTest(name=name):
                member = tarfile.TarInfo(name); member.mode = 0o644
                if link: member.type = tarfile.SYMTYPE; member.linkname = '/outside'
                stream, manifest_sha, _ = self.archive(extra=member)
                with self.assertRaises(ValueError):
                    VERIFIER['inspect_archive'](stream, 'candidate', manifest_sha)

    def test_manifest_digest_is_anchored_outside_the_payload(self):
        stream, _, _ = self.archive()
        with self.assertRaisesRegex(ValueError, 'independent evidence'):
            VERIFIER['inspect_archive'](stream, 'candidate', '0' * 64)

    def test_packaged_success_flag_does_not_replace_original_ci_record(self):
        original = {'passed': False}
        docs = {'manifest.json': {}, 'publication.json': {},
                'verification/runtime.json': {'passed': True}}
        before = copy.deepcopy(original)
        with self.assertRaisesRegex(ValueError, 'CI evidence'):
            VERIFIER['verify_components']({}, docs, {'runtime': original}, 'a' * 40, 'fixture', {})
        self.assertEqual(original, before)

    def test_component_bindings_refuse_unjoined_helper_missing_reclaim_and_wrong_native_source(self):
        VERIFIER['verify_components'](*self.components())
        for defect in ('helper', 'reclaim', 'source', 'archive', 'publication'):
            with self.subTest(defect=defect):
                values = self.components(); files, docs, proofs, _, _, native = values
                if defect == 'helper':
                    proofs['installed']['node']['helper']['native_helpers_joined'] = 0
                    docs['verification/installed.json'] = copy.deepcopy(proofs['installed'])
                elif defect == 'reclaim':
                    proofs['installed']['python']['peer_capacity_reclaimed'] = False
                    docs['verification/installed.json'] = copy.deepcopy(proofs['installed'])
                elif defect == 'source': native['source_revision'] = 'b' * 40
                elif defect == 'archive': files['sdk/node/node.tgz']['sha256'] = 'changed'
                else: docs['publication.json']['publication_authorized'] = True
                with self.assertRaises(ValueError): VERIFIER['verify_components'](*values)

    def test_repacked_rust_source_cannot_hide_behind_a_consistent_outer_inventory(self):
        package = {'name': 'fixture', 'version': '1.0.0', 'archive': 'fixture.crate', 'sha256': 'archive',
                   'manifest_source': '.deps/fixture/Cargo.toml', 'normalized_manifest_sha256': 'normalized',
                   'source_manifest_file': 'Cargo.toml.connect-source',
                   'source_files': {'Cargo.toml': 'original-manifest', 'src/lib.rs': 'original-source'}}
        files = {name: {'sha256': value} for name, value in {
            'sdk/rust/archives/fixture.crate': 'archive', 'sdk/rust/vendor/fixture-1.0.0/Cargo.toml': 'normalized',
            'sdk/rust/vendor/fixture-1.0.0/Cargo.toml.connect-source': 'original-manifest',
            'sdk/rust/vendor/fixture-1.0.0/src/lib.rs': 'original-source'}.items()}
        sources = {'.deps/fixture/' + name: {'sha256': value} for name, value in package['source_files'].items()}
        manifest = {'registry_packages': [], 'prepared_packages': [package]}
        VERIFIER['verify_rust_sources'](files, manifest, sources, {})
        files['sdk/rust/vendor/fixture-1.0.0/src/lib.rs']['sha256'] = 'changed'
        with self.assertRaisesRegex(ValueError, 'Actual repackaged Rust source'):
            VERIFIER['verify_rust_sources'](files, manifest, sources, {})


if __name__ == '__main__':
    unittest.main()
