"""Component provenance refusal, before extraction or npm execution."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
spec = importlib.util.spec_from_file_location('reference_components', ROOT / 'scripts/install-reference-app-components.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ComponentRefusals(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.dist = self.root / 'dist'
        self.private = self.root / '.local'
        self.dist.mkdir(); self.private.mkdir()
        self.runtime = self.dist / 'runtime.tar.gz'; self.runtime.write_bytes(b'not extracted')
        self.node = self.dist / 'node.tgz'; self.node.write_bytes(b'not installed')
        self.runtime_proof = self.private / 'runtime.json'
        self.node_proof = self.private / 'node.json'
        self.output = self.private / 'output'
        self.native = dict(format='connect/sdk-native-runtime-candidate/1', passed=True,
            production_release=True, test_support=False, source_inputs_unchanged=True,
            target='aarch64-apple-darwin', source_revision='a'*40, version='0.1.0',
            opus_native_build=dict(mode='bundled-static', native_archive_sha256='b'*64),
            archive_name=self.runtime.name, archive_sha256=module.digest(self.runtime))
        self.package = dict(format='connect/installed-node-sdk-candidate/1', passed=True,
            source_inputs_unchanged=True, consumer_outside_repository=True, empty_cache=True,
            install_scripts_disabled=True, external_declarations_checked=True,
            package_name='@rudeless/connect', version='0.1.0',
            archive_name=self.node.name, archive_sha256=module.digest(self.node), dependency_archives=[])
        for target, value in [('ROOT', self.root), ('platform.system', lambda: 'Darwin'), ('platform.machine', lambda: 'arm64')]:
            context = (patch.object(module, target, value) if '.' not in target
                       else patch.object(module.platform, target.split('.')[1], value))
            context.start(); self.addCleanup(context.stop)
        self.extract = patch.object(module, 'install_runtime', side_effect=AssertionError('Extraction reached'))
        self.extract.start(); self.addCleanup(self.extract.stop)

    def attempt(self, native=None, package=None):
        self.runtime_proof.write_text(json.dumps(self.native if native is None else native))
        self.node_proof.write_text(json.dumps(self.package if package is None else package))
        return module.install(self.runtime, self.runtime_proof, self.node, self.node_proof, self.output)

    def test_invalid_runtime_proof_refuses_before_output_or_extraction(self):
        for key, value in [('passed', False), ('production_release', False), ('test_support', True),
                ('source_inputs_unchanged', False), ('source_revision', 'not-a-revision'),
                ('archive_sha256', '0'*64), ('archive_name', 'other.tar.gz')]:
            with self.subTest(key=key):
                native = dict(self.native, **{key:value})
                with self.assertRaises(ValueError): self.attempt(native=native)
                self.assertFalse(self.output.exists())

    def test_invalid_node_proof_refuses_before_output_or_extraction(self):
        for key, value in [('passed', False), ('empty_cache', False), ('consumer_outside_repository', False),
                ('source_inputs_unchanged', False), ('install_scripts_disabled', False),
                ('external_declarations_checked', False), ('package_name', 'foreign'),
                ('version', 'other'), ('archive_sha256', '0'*64)]:
            with self.subTest(key=key):
                package = dict(self.package, **{key:value})
                with self.assertRaises(ValueError): self.attempt(package=package)
                self.assertFalse(self.output.exists())

    def test_requested_flags_without_actual_bundled_library_proof_refuse_before_output(self):
        native = dict(self.native, build_settings={'OPUS_STATIC': '1', 'LIBOPUS_NO_PKG': '1'})
        del native['opus_native_build']
        with self.assertRaisesRegex(ValueError, 'bundled Opus provenance'):
            self.attempt(native=native)
        self.assertFalse(self.output.exists())

    def test_arm_component_requires_explicit_producer_scope_and_actual_arm_host(self):
        native = dict(self.native, target='aarch64-unknown-linux-gnu')
        with patch.object(module.platform, 'system', return_value='Linux'), \
                patch.object(module.platform, 'machine', return_value='aarch64'), \
                patch.object(module.platform, 'freedesktop_os_release', return_value={'ID':'ubuntu','VERSION_ID':'24.04'}):
            with self.assertRaisesRegex(ValueError, 'Explicit ARM64 component provenance'):
                self.attempt(native=native)
            native['arm64_component_only'] = True
            with patch.object(module.platform, 'machine', return_value='x86_64'):
                with self.assertRaisesRegex(ValueError, 'Native producer, host'):
                    self.attempt(native=native)
        self.assertFalse(self.output.exists())

    def test_actual_arm_component_keeps_full_source_validation_before_extraction(self):
        native = dict(self.native, target='aarch64-unknown-linux-gnu', arm64_component_only=True)
        body, inputs = self.inputs()
        with patch.object(module.platform, 'system', return_value='Linux'), \
                patch.object(module.platform, 'machine', return_value='aarch64'), \
                patch.object(module.platform, 'freedesktop_os_release', return_value={'ID':'ubuntu','VERSION_ID':'24.04'}), \
                patch.object(module.runpy, 'run_path', return_value={'inputs':lambda:inputs}), \
                patch.object(module.subprocess, 'check_output', return_value=b'different source'):
            with self.assertRaisesRegex(ValueError, 'different source inputs'):
                self.attempt(native=native)
        self.assertFalse(self.output.exists())

    def test_linked_component_or_private_proof_refuses_without_overwriting_original(self):
        for original, parent in [(self.runtime,self.dist),(self.node_proof,self.private)]:
            if not original.exists(): original.write_bytes(b'original')
            linked = parent / ('linked-'+original.name); linked.symlink_to(original)
            before = original.read_bytes()
            with self.assertRaises(ValueError): module.checked_file(linked, parent)
            self.assertEqual(original.read_bytes(), before)

    def inputs(self):
        body = b'original package input'
        inputs = {'sdk/typescript/package.json':dict(sha256=hashlib.sha256(body).hexdigest(),mode=0o644)}
        path = self.private / 'source-inputs.json'; path.write_text(json.dumps(inputs))
        self.package['source_inputs_sha256'] = module.digest(path)
        return body, inputs

    def test_component_source_mismatch_refuses_even_when_version_and_archive_checks_match(self):
        body, inputs = self.inputs()
        with patch.object(module.runpy, 'run_path', return_value={'inputs':lambda:inputs}), \
                patch.object(module.subprocess, 'check_output', return_value=b'different source'):
            with self.assertRaisesRegex(ValueError, 'different source inputs'): self.attempt()
        self.assertFalse(self.output.exists())

    def test_occupied_output_is_preserved_before_extraction(self):
        body, inputs = self.inputs()
        self.output.mkdir(); marker = self.output / 'preserve'; marker.write_bytes(b'original')
        with patch.object(module.runpy, 'run_path', return_value={'inputs':lambda:inputs}), \
                patch.object(module.subprocess, 'check_output', return_value=body):
            with self.assertRaisesRegex(ValueError, 'new ordinary private'): self.attempt()
        self.assertEqual(marker.read_bytes(), b'original')


if __name__ == '__main__': unittest.main()
