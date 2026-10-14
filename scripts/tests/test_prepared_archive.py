"""Archive dependency provenance must fail closed on corruption or local drift."""
import hashlib
import importlib.util
import io
import json
import pathlib
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('prepared_archive', pathlib.Path(__file__).parents[1] / 'lib/prepared_archive.py')
ARCHIVE = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(ARCHIVE)


class PreparedArchiveTests(unittest.TestCase):
    def fixture(self, root, members=None):
        cache = root / 'cache'; cache.mkdir()
        archive = cache / 'fixture-1.crate'
        with tarfile.open(archive, 'w:gz') as bundle:
            for name, data, kind in members or [('fixture-1/source.txt', b'original\n', 'file')]:
                info = tarfile.TarInfo(name); info.mode = 0o644
                if kind == 'link': info.type = tarfile.SYMTYPE; info.linkname = '../private'
                else: info.size = len(data)
                bundle.addfile(info, io.BytesIO(data) if kind == 'file' else None)
        source = {'kind': 'crate', 'version': '1', 'url': 'https://static.crates.io/crates/fixture/fixture-1.crate',
                  'sha256': ARCHIVE.digest(archive), 'license': 'MIT', 'revision': 'fixture-revision'}
        destination = root / 'deps'; destination.mkdir()
        return cache, archive, source, destination

    def test_recorded_patch_preparation_and_exact_reuse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); cache, _, source, destination = self.fixture(root)
            patch = root / 'change.patch'
            patch.write_text('--- a/source.txt\n+++ b/source.txt\n@@ -1 +1 @@\n-original\n+patched\n')
            patches = [{'path': 'change.patch', 'sha256': ARCHIVE.digest(patch), 'target': 'fixture', 'source': 'regression'}]
            ARCHIVE.prepare_archive(root, destination, 'fixture', source, patches, cache)
            prepared = destination / 'fixture'
            self.assertEqual((prepared / 'source.txt').read_text(), 'patched\n')
            marker = (prepared / ARCHIVE.MARKER).read_bytes()
            ARCHIVE.prepare_archive(root, destination, 'fixture', source, patches, cache)
            self.assertEqual((prepared / ARCHIVE.MARKER).read_bytes(), marker)
            self.assertEqual(json.loads(marker)['identity']['patches'], patches)

    def test_archive_checksum_corruption_rejected_before_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); cache, archive, source, destination = self.fixture(root)
            archive.write_bytes(archive.read_bytes() + b'corrupt')
            with self.assertRaisesRegex(ValueError, 'identity mismatch'):
                ARCHIVE.prepare_archive(root, destination, 'fixture', source, [], cache)
            self.assertFalse((destination / 'fixture').exists())

    def test_changed_missing_extra_and_symlink_files_refuse_reuse(self):
        for mutation in ('changed', 'missing', 'extra', 'symlink', 'mode'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory); cache, _, source, destination = self.fixture(root)
                ARCHIVE.prepare_archive(root, destination, 'fixture', source, [], cache)
                prepared = destination / 'fixture'; original = prepared / 'source.txt'
                if mutation == 'changed': original.write_text('changed')
                elif mutation == 'missing': original.unlink()
                elif mutation == 'extra': (prepared / 'extra.txt').write_text('unexpected')
                elif mutation == 'mode': original.chmod(0o755)
                else: (prepared / 'linked').symlink_to(root)
                with self.assertRaises(ValueError):
                    ARCHIVE.prepare_archive(root, destination, 'fixture', source, [], cache)

    def test_traversal_absolute_link_and_duplicate_archive_members_rejected(self):
        scenarios = [[('fixture-1/../outside', b'bad', 'file')], [('/outside', b'bad', 'file')],
                     [('fixture-1/link', b'', 'link')],
                     [('fixture-1/same', b'a', 'file'), ('fixture-1/same', b'b', 'file')]]
        for members in scenarios:
            with self.subTest(members=members), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory); cache, _, source, destination = self.fixture(root, members)
                with self.assertRaises(ValueError):
                    ARCHIVE.prepare_archive(root, destination, 'fixture', source, [], cache)
                self.assertFalse((destination / 'fixture').exists())
                self.assertEqual(list(destination.glob('.fixture-preparing-*')), [])

    def test_archive_entry_and_expanded_byte_bounds_abort_and_clean_stage(self):
        for name, limit in [('MAX_FILES', 1), ('MAX_EXTRACTED_BYTES', 1)]:
            with self.subTest(bound=name), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                cache, _, source, destination = self.fixture(root, [
                    ('fixture-1/one', b'ab', 'file'), ('fixture-1/two', b'cd', 'file')])
                with patch.object(ARCHIVE, name, limit), self.assertRaisesRegex(ValueError, 'extraction bounds'):
                    ARCHIVE.prepare_archive(root, destination, 'fixture', source, [], cache)
                self.assertFalse((destination / 'fixture').exists())
                self.assertEqual(list(destination.glob('.fixture-preparing-*')), [])

    def test_source_metadata_and_patch_digest_changes_refuse_reuse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); cache, _, source, destination = self.fixture(root)
            license_file = root / 'LICENSE'; license_file.write_text('retained license')
            source['license_files'] = [{'path': 'LICENSE', 'sha256': ARCHIVE.digest(license_file)}]
            ARCHIVE.prepare_archive(root, destination, 'fixture', source, [], cache)
            license_file.write_text('changed license')
            with self.assertRaisesRegex(ValueError, 'license digest'):
                ARCHIVE.prepare_archive(root, destination, 'fixture', source, [], cache)
            license_file.write_text('retained license')
            with self.assertRaisesRegex(ValueError, 'identity'):
                ARCHIVE.prepare_archive(root, destination, 'fixture', dict(source, revision='changed'), [], cache)
            patch = root / 'change.patch'; patch.write_text('unrecorded')
            with self.assertRaisesRegex(ValueError, 'patch digest'):
                ARCHIVE.prepare_archive(root, destination, 'fixture', source,
                    [{'path': 'change.patch', 'target': 'fixture', 'sha256': '0' * 64}], cache)


if __name__ == '__main__': unittest.main()
