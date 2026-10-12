"""Retire only a baseline builder's generated target after verifying its seal."""
import hashlib
import json
import shutil


def retire_baseline_cache(output, revision):
    if output.name != revision or output.parent.name != "recovery-baselines":
        raise ValueError("Unexpected baseline cache owner")
    binary = output / "connect"
    manifest = json.loads((output / "manifest.json").read_text())
    if (binary.is_symlink() or manifest.get("source_revision") != revision
            or hashlib.sha256(binary.read_bytes()).hexdigest() != manifest.get("binary_sha256")):
        raise ValueError("Baseline seal failed; retain its build cache")
    target = output / "target"
    if target.is_symlink() or (target.exists() and not target.is_dir()):
        raise ValueError("Baseline target must be an owned generated directory")
    before = shutil.disk_usage(output).free
    existed = target.exists()
    if existed:
        shutil.rmtree(target)
    return {"source_revision": revision, "binary_sha256": manifest["binary_sha256"],
            "target": str(target), "cache_removed": existed,
            "free_bytes_before": before, "free_bytes_after": shutil.disk_usage(output).free}
