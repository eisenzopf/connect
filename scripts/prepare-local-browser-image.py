#!/usr/bin/env python3
"""Copy only public pinned ARM browser assets into a fresh local image context."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    host = runpy.run_path(str(ROOT / "scripts/lib/linux_arm64_profile.py"))["native_host"]()
    browser = args.browser.resolve(strict=True)
    assert browser.parent.name == "chrome-linux-arm64" and browser.parent.parent.name == "chromium-1248"
    distribution = browser.parent.parent
    version = subprocess.check_output([str(browser), "--version"], text=True, timeout=10).strip()
    assert version == "Google Chrome for Testing 156.0.8078.4"
    with browser.open("rb") as stream:
        header = stream.read(64)
    assert header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[18:20], "little") == 183
    for path in distribution.rglob("*"):
        assert path.resolve().is_relative_to(distribution), "Browser symlink escapes its public distribution"
    output = args.output.resolve()
    assert output.is_relative_to(ROOT / ".local")
    os.umask(0o077)
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    for name in ("package.json", "package-lock.json"):
        shutil.copy2(ROOT / "scripts/video/observer-tools" / name, output / name)
    shutil.copytree(distribution, output / distribution.name, symlinks=True)
    files = {}
    for path in output.rglob("*"):
        if path.is_file():
            with path.open("rb") as stream:
                files[str(path.relative_to(output))] = hashlib.file_digest(stream, "sha256").hexdigest()
    proof = dict(format="connect/local-arm-browser-image-context/1", host=host, browser_version=version,
        browser_elf_machine=183, files=files, no_repository_or_private_state_copied=True,
        image_build_executed=False, tests_executed=False)
    (output / "context-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
    print(json.dumps(dict(browser_version=version, architecture="aarch64", public_files=len(files), tests_executed=False)))


if __name__ == "__main__":
    main()
