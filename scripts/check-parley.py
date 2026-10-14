#!/usr/bin/env python3
"""Run pinned Parley conversation regressions against the prepared source graph."""
import json
import os
import pathlib
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
OVERRIDES = ('rtc-dtls', 'rtc-interceptor', 'quinn-proto')


def verify_metadata(metadata, root, lock):
    """Cargo patches belong to the selected root, not dependency workspaces."""
    for name in OVERRIDES:
        packages = [p for p in metadata['packages'] if p['name'] == name]
        expected = (root / '.deps' / name / 'Cargo.toml').resolve()
        if len(packages) != 1:
            raise ValueError(f'Expected exactly one prepared {name} package')
        package = packages[0]
        if (package['source'] is not None
                or package['version'] != lock['sources'][name]['version']
                or pathlib.Path(package['manifest_path']).resolve() != expected):
            raise ValueError(f'Parley did not select the pinned prepared {name}')


def main():
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    with tempfile.TemporaryDirectory(prefix='connect-parley-') as work:
        work = pathlib.Path(work)
        dest = work / 'parley'
        shutil.copytree(ROOT / '.deps/parley', dest,
                        ignore=shutil.ignore_patterns('.git', 'target'))
        (work / 'rvoip-conference').symlink_to(ROOT / '.deps/rvoip', target_is_directory=True)
        for name in OVERRIDES:
            (work / name).symlink_to(ROOT / '.deps' / name, target_is_directory=True)
        environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target/upstream'))
        version = subprocess.check_output(['rustc', '-vV'], env=environment, text=True)
        host = next(line.removeprefix('host: ') for line in version.splitlines()
                    if line.startswith('host: '))
        # Resolve optional media dependencies too, without compiling them. The
        # three conversation suites retain their original selected features.
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--locked', '--format-version=1',
             '--filter-platform', host, '--all-features'], cwd=dest, env=environment))
        verify_metadata(metadata, ROOT, lock)
        subprocess.run(
            ['cargo', 'test', '--locked', '--no-default-features', '--features',
             'uctp,sms-fake', '--test', 'api_conversations', '--test',
             'uctp_conference', '--test', 'uctp_create'],
            cwd=dest, env=environment, check=True)


if __name__ == '__main__':
    main()
