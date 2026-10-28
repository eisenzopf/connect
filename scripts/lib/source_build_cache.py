"""Preserve source times only when content and mode match a completed build.

Fresh checkouts otherwise invalidate Cargo's mtime-based dependency cache. This
never restores source contents, permissions, symlinks or untracked/private files.
Call save only after successful compilation with unchanged guarded sources.
"""
import argparse
import hashlib
import json
from pathlib import Path
import os
import stat
import subprocess

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / ".local/gate6-linux-ci/source-times.json"


def snapshot(root):
    names = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
    files = {}
    for name in names:
        if not name:
            continue
        path = root / name
        if path.is_symlink() or not path.is_file():
            continue
        info = path.stat()
        if info.st_nlink != 1:
            continue
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        files[name] = {"sha256": digest, "mode": stat.S_IMODE(info.st_mode), "mtime_ns": info.st_mtime_ns}
    return files


def restore(root, saved):
    restored = 0
    # Iterate current tracked files, never paths supplied by cache metadata.
    for name, current in snapshot(root).items():
        old = saved.get(name, {})
        if old.get("sha256") != current["sha256"] or old.get("mode") != current["mode"]:
            continue
        modified = old.get("mtime_ns")
        if not isinstance(modified, int) or not 0 < modified < 2**63:
            continue
        path = root / name
        info = path.stat()
        os.utime(path, ns=(info.st_atime_ns, modified), follow_symlinks=False)
        restored += 1
    return restored


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("restore", "save"))
    args = parser.parse_args()
    if args.action == "restore":
        if not MANIFEST.exists():
            print(json.dumps({"restored": 0, "cache_present": False}))
            return
        assert MANIFEST.is_file() and not MANIFEST.is_symlink() and MANIFEST.stat().st_size <= 8 * 1024 * 1024
        saved = json.loads(MANIFEST.read_text())
        assert saved["format"] == 1 and len(saved["files"]) <= 20000
        print(json.dumps({"restored": restore(ROOT, saved["files"]), "cache_present": True}))
    else:
        MANIFEST.parent.mkdir(parents=True, exist_ok=True)
        MANIFEST.write_text(json.dumps({"format": 1, "files": snapshot(ROOT)}, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
