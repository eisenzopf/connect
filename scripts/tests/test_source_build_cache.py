import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/lib"))
from source_build_cache import restore, snapshot


class SourceBuildCache(unittest.TestCase):
    def setUp(self):
        private = ROOT / ".local/gate6-linux-ci/cache-tests"
        private.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=private)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True)
        self.source = self.root / "source.rs"
        self.source.write_text("allow = false\n")
        subprocess.run(["git", "add", "source.rs"], cwd=self.root, check=True)
        os.utime(self.source, ns=(1_000_000_000, 1_000_000_000))

    def test_unchanged_content_and_mode_restore_the_original_source_time(self):
        saved = snapshot(self.root)
        os.utime(self.source, ns=(3_000_000_000, 3_000_000_000))
        self.assertEqual(restore(self.root, saved), 1)
        self.assertEqual(self.source.stat().st_mtime_ns, 1_000_000_000)

    def test_same_size_changed_source_keeps_its_new_time(self):
        saved = snapshot(self.root)
        self.source.write_text("allow = true \n")
        os.utime(self.source, ns=(3_000_000_000, 3_000_000_000))
        self.assertEqual(restore(self.root, saved), 0)
        self.assertEqual(self.source.stat().st_mtime_ns, 3_000_000_000)
        self.assertEqual(self.source.read_text(), "allow = true \n")

    def test_mode_changes_and_untracked_private_paths_are_never_restored(self):
        saved = snapshot(self.root)
        private = self.root / "private.txt"
        private.write_text(self.source.read_text())
        saved["private.txt"] = saved["source.rs"]
        self.source.chmod(0o755)
        os.utime(self.source, ns=(3_000_000_000, 3_000_000_000))
        os.utime(private, ns=(3_000_000_000, 3_000_000_000))
        self.assertEqual(restore(self.root, saved), 0)
        self.assertEqual(private.stat().st_mtime_ns, 3_000_000_000)
        self.assertEqual(self.source.stat().st_mtime_ns, 3_000_000_000)


if __name__ == "__main__":
    unittest.main()
