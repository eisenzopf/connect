"""Pinned Firefox trust importer bootstrap; no browser or native library execution."""
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
