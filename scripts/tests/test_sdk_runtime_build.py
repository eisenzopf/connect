import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.sdk_runtime_build import bundled_opus, require_bundled_opus
from lib.sdk_candidate import install_runtime


class NativeOpusBuild(unittest.TestCase):
    def test_macos_consumer_refuses_missing_or_malformed_native_proof_before_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / 'installation'
            for native in [None, {}, {'mode': 'system-static', 'native_archive_sha256': 'a'*64},
                           {'mode': 'bundled-static', 'native_archive_sha256': 'not-a-digest'},
                           {'mode': 'bundled-static', 'native_archive_sha256': 1}]:
                proof = {'target': 'aarch64-apple-darwin', 'opus_native_build': native}
                with self.assertRaisesRegex(ValueError, 'bundled Opus provenance'):
                    install_runtime(Path(directory) / 'absent.tar.gz', proof, destination)
                self.assertFalse(destination.exists())
            require_bundled_opus({'target': 'aarch64-apple-darwin',
                'opus_native_build': {'mode': 'bundled-static', 'native_archive_sha256': 'a'*64}})
            require_bundled_opus({'target': 'aarch64-unknown-linux-gnu'})

    def test_actual_selected_bundled_archive_is_hashed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'out'
            library = root / 'lib/libopus.a'
            library.parent.mkdir(parents=True)
            library.write_bytes(b'controlled static native archive')
            log = Path(directory) / 'build.log'
            record = {'reason': 'build-script-executed', 'package_id': 'registry+source#audiopus_sys@0.2.2',
                      'out_dir': str(root), 'linked_paths': ['native=' + str(library.parent)],
                      'linked_libs': ['static=opus']}
            log.write_text('native compiler diagnostic\n' + json.dumps(record) + '\n')
            self.assertEqual(bundled_opus(log), {'mode': 'bundled-static',
                'native_archive_sha256': hashlib.sha256(library.read_bytes()).hexdigest()})
            for mutate in [lambda r: r.update(linked_paths=['native=/opt/homebrew/Cellar/opus/1.6.1/lib']),
                           lambda r: r.update(linked_libs=['opus']),
                           lambda r: r.update(linked_paths=['native=' + str(root) + '-sibling/lib']),
                           lambda r: r.update(linked_paths=['framework=' + str(root)])]:
                changed = json.loads(json.dumps(record)); mutate(changed)
                log.write_text(json.dumps(changed) + '\n')
                with self.assertRaises(ValueError): bundled_opus(log)

    def test_missing_duplicate_or_foreign_build_records_cannot_prove_native_source(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'build.log'
            for rows in [[], [{'reason': 'compiler-artifact', 'package_id': 'registry+source#audiopus_sys@0.2.2'}],
                         [{'reason': 'build-script-executed', 'package_id': 'registry+source#foreign_audiopus_sys@0.2.2'}],
                         [{'reason': 'build-script-executed', 'package_id': 'registry+source#audiopus_sys@0.2.2'}] * 2]:
                log.write_text('\n'.join(json.dumps(row) for row in rows))
                with self.assertRaises(ValueError): bundled_opus(log)


if __name__ == '__main__': unittest.main()
