#!/usr/bin/env python3
"""Reproduce pinned upstream sources in a private ignored build directory."""
import argparse
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]


def git(path, *args):
    return subprocess.check_output(['git', '-C', str(path), *args], text=True).strip()


def run(*args):
    subprocess.run(args, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=pathlib.Path, help='Optional verified audit snapshot cache')
    args = parser.parse_args()
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    for patch in lock['patches']:
        assert hashlib.sha256((ROOT / patch['path']).read_bytes()).hexdigest() == patch['sha256'], patch['path']
    directory = ROOT / '.deps'
    directory.mkdir(exist_ok=True)
    for name, source in lock['sources'].items():
        dest = directory / name
        if not dest.exists():
            cache = args.cache / name if args.cache else None
            if cache and cache.exists() and git(cache, 'rev-parse', 'HEAD') == source['revision']:
                run('git', 'clone', '--quiet', '--no-local', str(cache), str(dest))
                run('git', '-C', str(dest), 'remote', 'set-url', 'origin', source['url'])
            else:
                dest.mkdir()
                run('git', '-C', str(dest), 'init', '--quiet')
                run('git', '-C', str(dest), 'remote', 'add', 'origin', source['url'])
                run('git', '-C', str(dest), 'fetch', '--quiet', '--depth=1', 'origin', source['revision'])
            run('git', '-C', str(dest), 'checkout', '--quiet', '--detach', source['revision'])
        if git(dest, 'rev-parse', 'HEAD') != source['revision'] or git(dest, 'remote', 'get-url', 'origin') != source['url']:
            raise SystemExit(f'{dest}: source identity mismatch; refusing to reset existing work')
        if name == 'rvoip':
            marker = dest / '.git/connect-prepared.json'
            if marker.exists():
                try:
                    prepared = json.loads(marker.read_text())
                except json.JSONDecodeError as error:
                    raise SystemExit(f'Invalid dependency preparation marker: {error}') from error
                if prepared != lock:
                    raise SystemExit('Dependency lock changed; use a new .deps directory after preserving any changes')
                # Verify exactly the intended patch content, including newly added files.
                manifest = json.loads((dest / '.git/connect-files.json').read_text())
                for relative, digest in manifest.items():
                    if hashlib.sha256((dest / relative).read_bytes()).hexdigest() != digest:
                        raise SystemExit(f'Prepared source changed: {relative}')
                expected_status = (dest / '.git/connect-status.txt').read_text().rstrip('\n')
                if git(dest, 'status', '--porcelain') != expected_status:
                    raise SystemExit('Unexpected prepared source changes')
                continue
            if git(dest, 'status', '--porcelain'):
                raise SystemExit(f'{dest}: unrecorded changes; refusing to reset')
            for patch in lock['patches']:
                run('git', '-C', str(dest), 'apply', '--check', str(ROOT / patch['path']))
                run('git', '-C', str(dest), 'apply', str(ROOT / patch['path']))
            paths = git(dest, 'diff', '--name-only').splitlines() + git(dest, 'ls-files', '--others', '--exclude-standard').splitlines()
            manifest = {p: hashlib.sha256((dest / p).read_bytes()).hexdigest() for p in paths}
            (dest / '.git/connect-files.json').write_text(json.dumps(manifest, sort_keys=True))
            (dest / '.git/connect-status.txt').write_text(git(dest, 'status', '--porcelain'))
            marker.write_text(json.dumps(lock, sort_keys=True))
        elif git(dest, 'status', '--porcelain'):
            raise SystemExit(f'{dest}: unexpected changes')
    print('Pinned sources and patch digests verified in .deps/')


if __name__ == '__main__':
    main()
