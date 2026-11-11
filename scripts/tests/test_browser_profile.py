"""Exact private-profile runtime/schema and bundled NSS; no browser/library execution."""
import importlib.util
from contextlib import closing
import hashlib
import json
import sqlite3
import tempfile
from pathlib import Path
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).resolve().parents[1] / "video/browser_profile.py"
SPEC = importlib.util.spec_from_file_location("browser_profile", MODULE_PATH)
PROFILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILE)


class BundledLoader:
    """Model the retained Linux ELF dependencies and absent mozglue library."""

    DEPENDENCIES = {
        "libnspr4.so": (),
        "libplc4.so": ("libnspr4.so",),
        "libplds4.so": ("libnspr4.so",),
        "libnssutil3.so": ("libnspr4.so", "libplc4.so", "libplds4.so"),
        "libmozsqlite3.so": (),
        "libfreeblpriv3.so": ("libnspr4.so", "libplc4.so", "libplds4.so"),
        "libsoftokn3.so": ("libnspr4.so", "libplc4.so", "libplds4.so", "libnssutil3.so", "libmozsqlite3.so"),
        "libnss3.so": ("libnspr4.so", "libplc4.so", "libplds4.so", "libnssutil3.so"),
    }

    def __init__(self, root, absent=()):
        self.root = root
        self.absent = absent
        self.handles = {}
        self.calls = []

    def __call__(self, filename, **options):
        path = Path(filename)
        self.calls.append(path)
        if path.parent != self.root or path.name in self.absent or path.name not in self.DEPENDENCIES:
            raise OSError("Bundled library is absent")
        for dependency in self.DEPENDENCIES[path.name]:
            if dependency not in self.handles:
                raise OSError("Dependency would resolve outside the designated bundle")
        if path.name != "libnss3.so" and options.get("mode") != PROFILE.ctypes.RTLD_GLOBAL:
            raise OSError("Bundled dependency unavailable to NSS")
        handle = self.handles[path.name] = object()
        return handle


class FirefoxProfileBootstrap(unittest.TestCase):
    def test_linux_archive_without_mozglue_uses_its_own_nss_and_software_token(self):
        runtime = Path("/original/firefox")
        loader = BundledLoader(runtime)
        with mock.patch.object(PROFILE.sys, "platform", "linux"), mock.patch.object(PROFILE.ctypes, "CDLL", loader):
            nss, owned = PROFILE.load_firefox_nss(runtime)
        self.assertIs(nss, loader.handles["libnss3.so"])
        self.assertIn(loader.handles["libsoftokn3.so"], owned)
        self.assertIn(loader.handles["libfreeblpriv3.so"], owned)
        self.assertNotIn(runtime / "libmozglue.so", loader.calls)
        self.assertEqual(len(owned), len(loader.handles) - 1)

    def test_missing_bundled_dependency_refuses_without_system_library_search(self):
        runtime = Path("/original/firefox")
        loader = BundledLoader(runtime, absent=("libnssutil3.so",))
        with mock.patch.object(PROFILE.sys, "platform", "linux"), mock.patch.object(PROFILE.ctypes, "CDLL", loader):
            with self.assertRaises(OSError):
                PROFILE.load_firefox_nss(runtime)
        self.assertNotIn("libnss3.so", loader.handles)
        self.assertTrue(all(path.parent == runtime for path in loader.calls))

    def test_mac_retains_required_mozglue_before_its_bundled_nss(self):
        runtime = Path("/original/Firefox.app/Contents/MacOS")
        glue, nss = object(), object()
        with mock.patch.object(PROFILE.sys, "platform", "darwin"), mock.patch.object(PROFILE.ctypes, "CDLL", side_effect=[glue, nss]) as load:
            actual, owned = PROFILE.load_firefox_nss(runtime)
        self.assertIs(actual, nss)
        self.assertEqual(owned, [glue])
        self.assertEqual(load.call_args_list, [
            mock.call(str(runtime / "libmozglue.dylib"), mode=PROFILE.ctypes.RTLD_GLOBAL),
            mock.call(str(runtime / "libnss3.dylib")),
        ])

    def test_unsupported_platform_never_loads_a_library(self):
        with mock.patch.object(PROFILE.sys, "platform", "win32"), mock.patch.object(PROFILE.ctypes, "CDLL") as load:
            with self.assertRaises(ValueError):
                PROFILE.load_firefox_nss(Path("/original/firefox"))
        load.assert_not_called()


class ChromeProfileContract(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.base = Path(self.scratch.name)
        self.root_der = self.base / 'root.der'
        self.root_der.write_bytes(b'\x30\x00')
        self.binary = self.base / 'original-chrome'
        self.contract = json.loads((PROFILE.ROOT / 'contracts/gate6-transport-matrix.json').read_text())

    def create(self, version, system='darwin', destination=None):
        destination = destination or self.base / 'profile'
        with mock.patch.object(PROFILE.sys, 'platform', system), \
                mock.patch.object(PROFILE.platform, 'machine', return_value='aarch64'), \
                mock.patch.object(PROFILE.subprocess, 'check_output', return_value=version + '\n'):
            return PROFILE.create_profile('chrome', self.binary, destination, self.root_der)

    def test_approved_mac157_seeds_actual_chrome157_schema_and_preserves_trust_bytes(self):
        fixture_path = Path(__file__).resolve().parents[1] / 'fixtures/browser-profile/chrome157-server-certificate-schema.json'
        original = json.loads(fixture_path.read_text())
        version = 'Google Chrome ' + self.contract['expected_browsers_by_client_platform']['macos-arm64']['chrome']
        self.assertEqual(version, original['browser_version'], 'New approved runtime needs an actual schema observation')
        result = self.create(version)
        self.assertEqual(result['version'], version)
        self.assertEqual(result['root_sha256'], hashlib.sha256(self.root_der.read_bytes()).hexdigest())
        with closing(sqlite3.connect(self.base / 'profile/Default/ServerCertificate')) as db:
            for name, columns in original['tables'].items():
                self.assertEqual([list(row) for row in db.execute('PRAGMA table_info(' + name + ')')], columns)
            self.assertEqual([list(row) for row in db.execute('SELECT key,value FROM meta ORDER BY key')], original['meta'])
            self.assertEqual(db.execute('SELECT sha256hash_hex,der_cert,trust_settings FROM certificates').fetchall(),
                             [(result['root_sha256'], self.root_der.read_bytes(), bytes.fromhex('0a020803'))])
            self.assertEqual(db.execute('PRAGMA integrity_check').fetchone(), ('ok',))
        self.assertEqual((self.base / 'profile').stat().st_mode & 0o777, 0o700)

    def test_linux_retains_exact_approved_testing_runtime(self):
        version = 'Google Chrome for Testing ' + self.contract['expected_browsers_by_client_platform']['ubuntu24-arm64']['chrome']
        self.assertEqual(self.create(version, 'linux')['version'], version)

    def test_profile_import_reads_actual_contract_not_a_second_version_literal(self):
        contract = json.loads(json.dumps(self.contract))
        contract['expected_browsers_by_client_platform']['macos-arm64']['chrome'] = '157.0.8103.40'
        source = self.base / 'contract-root'
        (source / 'contracts').mkdir(parents=True)
        (source / 'contracts/gate6-transport-matrix.json').write_text(json.dumps(contract))
        with mock.patch.object(PROFILE, 'ROOT', source):
            self.assertEqual(self.create('Google Chrome 157.0.8103.40')['version'], 'Google Chrome 157.0.8103.40')

    def test_stale_foreign_product_or_platform_runtime_refuses_before_profile_creation(self):
        for index, (system, version) in enumerate([
                ('darwin', 'Google Chrome 156.0.8078.80'),
                ('darwin', 'Google Chrome for Testing 157.0.8103.39'),
                ('linux', 'Google Chrome 157.0.8103.39'),
                ('linux', 'Google Chrome for Testing 156.0.8078.5')]):
            with self.subTest(system=system, version=version):
                profile = self.base / ('refused-' + str(index))
                with self.assertRaises(ValueError):
                    self.create(version, system, profile)
                self.assertFalse(profile.exists())

    def test_existing_or_symlink_profile_refuses_without_running_version_or_changing_target(self):
        existing = self.base / 'existing'
        existing.mkdir()
        marker = existing / 'original'
        marker.write_bytes(b'keep')
        link = self.base / 'linked'
        link.symlink_to(existing, target_is_directory=True)
        with mock.patch.object(PROFILE.subprocess, 'check_output') as check:
            for profile in [existing, link]:
                with self.assertRaises(FileExistsError):
                    PROFILE.create_profile('chrome', self.binary, profile, self.root_der)
        check.assert_not_called()
        self.assertEqual(marker.read_bytes(), b'keep')
        self.assertEqual(sorted(p.name for p in existing.iterdir()), ['original'])

    def test_empty_or_oversized_root_refuses_without_profile_or_version_execution(self):
        for size in [0, 65537]:
            self.root_der.write_bytes(b'x' * size)
            with mock.patch.object(PROFILE.subprocess, 'check_output') as check:
                with self.assertRaises(ValueError):
                    PROFILE.create_profile('chrome', self.binary, self.base / 'refused', self.root_der)
            check.assert_not_called()
            self.assertFalse((self.base / 'refused').exists())

    def test_unsupported_platform_refuses_without_creating_profile(self):
        with self.assertRaises(ValueError):
            self.create('Google Chrome 157.0.8103.39', 'win32')
        self.assertFalse((self.base / 'profile').exists())


if __name__ == "__main__":
    unittest.main()
