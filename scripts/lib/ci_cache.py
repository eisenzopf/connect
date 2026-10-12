"""Retire only a baseline builder's generated target after verifying its seal."""
import hashlib
import json
import pathlib
import re
import shutil
import stat

from .local_process import ROOT


def verify_owner(output, revision):
    """Require the builder's literal and resolved owner before any retirement."""
    parent = ROOT / ".local/recovery-baselines"
    expected = parent / revision
    if not re.fullmatch(r"[0-9a-f]{40}", revision) or output != expected:
        raise ValueError("Unexpected baseline cache owner")
    try:
        for directory in (ROOT, ROOT / ".local", parent, output):
            if not stat.S_ISDIR(directory.lstat().st_mode):
                raise ValueError("Baseline owner must be an ordinary non-symlink directory")
        if parent.resolve(strict=True) != parent or output.resolve(strict=True) != expected:
            raise ValueError("Resolved baseline cache escaped its owner")
    except (OSError, RuntimeError) as error:
        raise ValueError("Baseline cache owner could not be verified") from error


def retire_baseline_cache(output, revision):
    output = pathlib.Path(output).absolute()
    verify_owner(output, revision)
    binary = output / "connect"
    manifest_path = output / "manifest.json"
    if not all(stat.S_ISREG(p.lstat().st_mode) for p in (binary, manifest_path)):
        raise ValueError("Baseline seal must use owned ordinary files")
    manifest = json.loads(manifest_path.read_text())
    if (manifest.get("source_revision") != revision
            or hashlib.sha256(binary.read_bytes()).hexdigest() != manifest.get("binary_sha256")):
        raise ValueError("Baseline seal failed; retain its build cache")
    target = output / "target"
    if target.is_symlink() or (target.exists() and not target.is_dir()):
        raise ValueError("Baseline target must be an owned generated directory")
    before = shutil.disk_usage(output).free
    existed = target.exists()
    if existed:
        verify_owner(output, revision)
        if target.resolve(strict=True) != output / "target":
            raise ValueError("Resolved baseline target escaped its owner")
        shutil.rmtree(target)
    return {"source_revision": revision, "binary_sha256": manifest["binary_sha256"],
            "target": str(target), "cache_removed": existed,
            "free_bytes_before": before, "free_bytes_after": shutil.disk_usage(output).free}
