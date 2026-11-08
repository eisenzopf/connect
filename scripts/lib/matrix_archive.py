"""Losslessly archive a joined cell before the next cell consumes scratch."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tarfile

from .matrix_storage import archive_budget, atomic_json, preflight, sync_file, sync_directory, sync_ancestors


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
    from .matrix_archive_drain import original_owners_joined
    # Successful shutdown and physical custody are separate. Preserve a
    # nonzero original exit in the report while requiring its exact joined
    # process group and all other original owners before archival.
    if not original_owners_joined(report):
        return None
    directory = Path(directory)
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("Original cell directory required")
    original = inventory(directory)
    # A new archive overlaps the still-preserved expanded original. Charge its
    # actual independent tar members, not a historical compression ratio.
    preflight(directory, required=archive_budget(original))
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
    with archive.open("rb") as source:
        sync_file(source.fileno())
    cell_roots = [p.parent for p in directory.parents if p.name == "cells"]
    sync_ancestors(archive.parent,cell_roots[0].parent if cell_roots else directory.parent)
    receipt = dict(format="connect/g6-058-original-cell-archive/1", archive_sha256=sha,
        archive_bytes=archive.stat().st_size, entries=original, verified_before_removal=True)
    atomic_json(directory.with_suffix(".archive.json"), receipt, limit=32 * 1024**2)
    shutil.rmtree(directory)
    sync_directory(directory.parent)
    return dict(path=str(archive), sha256=sha, bytes=archive.stat().st_size,
                original_entries=len(original), verified_before_removal=True)
