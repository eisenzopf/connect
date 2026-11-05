"""Losslessly archive a joined cell before the next cell consumes scratch."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tarfile


def inventory(directory):
    result = {}
    for path in sorted(directory.rglob("*")):
        info = path.lstat()
        name = path.relative_to(directory).as_posix()
        if stat.S_ISLNK(info.st_mode):
            result[name] = dict(kind="symlink", mode=stat.S_IMODE(info.st_mode), target=os.readlink(path))
        elif stat.S_ISDIR(info.st_mode):
            result[name] = dict(kind="directory", mode=stat.S_IMODE(info.st_mode))
        elif stat.S_ISREG(info.st_mode):
            with path.open("rb") as source:
                sha = hashlib.file_digest(source, "sha256").hexdigest()
            result[name] = dict(kind="file", mode=stat.S_IMODE(info.st_mode), bytes=info.st_size, sha256=sha)
        else:
            raise ValueError("Cell still contains a live or special resource")
    return result


def verify(archive, expected):
    seen = set()
    with tarfile.open(archive, "r:gz") as packed:
        for member in packed:
            if member.name in seen or member.name not in expected:
                raise ValueError("Duplicate or unexpected archived cell path")
            seen.add(member.name)
            original = expected[member.name]
            if member.mode != original["mode"]:
                raise ValueError("Archived cell permissions differ")
            if original["kind"] == "file":
                if not member.isfile() or member.size != original["bytes"]:
                    raise ValueError("Archived cell file shape differs")
                with packed.extractfile(member) as source:
                    if hashlib.file_digest(source, "sha256").hexdigest() != original["sha256"]:
                        raise ValueError("Archived cell content differs")
            elif original["kind"] == "directory":
                if not member.isdir():
                    raise ValueError("Archived directory differs")
            elif not member.issym() or member.linkname != original["target"]:
                raise ValueError("Archived symbolic link differs")
    if seen != set(expected):
        raise ValueError("Archived cell entries missing")


def archive_joined(directory, report):
    if not report.get("servers") or any(s.get("original_joined") is not True for s in report["servers"]):
        return None
    if report.get("format") == "connect/browser-matrix-component/1":
        if report.get("browser_owner_joined") is not True:
            return None
    elif report.get("conversation_control", {}).get("all_observed_helpers_joined") is not True:
        return None
    directory = Path(directory)
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("Original cell directory required")
    original = inventory(directory)
    archive = directory.with_suffix(".tar.gz")
    if archive.exists():
        raise ValueError("Original cell archive already exists")
    with tarfile.open(archive, "x:gz", compresslevel=1, dereference=False) as packed:
        for name in original:
            # Retain each file's bytes; do not replace sealed executable copies
            # with tar hard-link records that lack an independent content hash.
            packed.inodes.clear()
            packed.add(directory / name, arcname=name, recursive=False)
    verify(archive, original)
    if inventory(directory) != original:
        raise ValueError("Original cell changed during archival")
    with archive.open("rb") as source:
        sha = hashlib.file_digest(source, "sha256").hexdigest()
    receipt = dict(format="connect/g6-058-original-cell-archive/1", archive_sha256=sha,
        archive_bytes=archive.stat().st_size, entries=original, verified_before_removal=True)
    directory.with_suffix(".archive.json").write_text(json.dumps(receipt, indent=2) + "\n")
    shutil.rmtree(directory)
    return dict(path=str(archive), sha256=sha, bytes=archive.stat().st_size,
                original_entries=len(original), verified_before_removal=True)
