#!/usr/bin/env python3
"""Prepare an unpublished standalone Rust SDK and offline registry-source bundle.

Run on a prepared clean checkout. This assembles source, not a native-runtime
qualification. The separate external checker owns its cold Cargo home and build.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib

from lib.sdk_rust_package import install_archive, prepare_package, sha256, write_toml

ROOT = Path(__file__).resolve().parents[1]


def git(*arguments):
    return subprocess.check_output(['git', *arguments], cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if git('status', '--porcelain'):
        parser.error('Commit source first; clean checkout required')
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / 'dist').resolve()):
        parser.error('Candidate packages belong under dist/')
    revision = git('rev-parse', 'HEAD')
    epoch = int(git('show', '-s', '--format=%ct', revision))
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1'], cwd=ROOT, timeout=120))
    inputs = {name: sha256(ROOT / name) for name in git('ls-files').splitlines() if (ROOT / name).is_file() and not (ROOT / name).is_symlink()}
    output.mkdir(parents=True, exist_ok=False)
    vendor = output / 'vendor'; vendor.mkdir()
    archives = output / 'archives'; archives.mkdir()
    package = next(p for p in metadata['packages'] if p['name'] == 'connect-client' and p['source'] is None)
    pending = [Path(package['manifest_path'])]
    # These are explicitly patched crates in the candidate's locked source.
    manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text())
    pending.extend(ROOT / item['path'] / 'Cargo.toml' for item in manifest['patch']['crates-io'].values())
    found = set(); prepared = []
    while pending:
        source = pending.pop().resolve(strict=True)
        if source in found:
            continue
        if not source.is_relative_to(ROOT.resolve()) or len(found) >= 128:
            raise ValueError('Prepared source closure escapes its bounded source root')
        found.add(source)
        record, dependencies = prepare_package(source, vendor, archives, epoch)
        prepared_root = (ROOT / '.deps').resolve(strict=True)
        record['manifest_source'] = ('.deps/' + source.relative_to(prepared_root).as_posix()
                                     if source.is_relative_to(prepared_root)
                                     else source.relative_to(ROOT.resolve()).as_posix())
        prepared.append(record)
        pending.extend(dependencies)
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    checksums = {(p['name'], p['version'], p.get('source')): p.get('checksum') for p in lock['package']}
    registry = []
    for item in metadata['packages']:
        if item['source'] is None:
            continue
        if not item['source'].startswith('registry+'):
            raise ValueError('Unsupported unrecorded source kind')
        identity = item['name'] + '-' + item['version']
        if (vendor / identity).exists():
            raise ValueError('Prepared/registry identity collision')
        source = Path(item['manifest_path']).parent
        archive = source.parents[2] / 'cache' / source.parent.name / (identity + '.crate')
        expected = checksums[(item['name'], item['version'], item['source'])]
        if not expected or not archive.is_file():
            raise ValueError('Locked original registry archive is absent')
        install_archive(archive, vendor, item['name'], item['version'], expected)
        shutil.copyfile(archive, archives / archive.name)
        registry.append({'name': item['name'], 'version': item['version'], 'sha256': expected, 'archive': archive.name})
    config = output / '.cargo'; config.mkdir()
    write_toml(config / 'config.toml', {'source': {'crates-io': {'replace-with': 'connect-candidate'}, 'connect-candidate': {'directory': 'vendor'}}})
    example = output / 'examples/rust-sdk-journey'
    shutil.copytree(ROOT / 'examples/rust-sdk-journey/src', example / 'src')
    example_manifest = tomllib.loads((ROOT / 'examples/rust-sdk-journey/Cargo.toml').read_text())
    example_manifest.pop('patch', None)
    example_manifest['dependencies']['connect-client'] = '=' + package['version']
    write_toml(example / 'Cargo.toml', example_manifest)
    shutil.copyfile(ROOT / 'LICENSE', output / 'LICENSE')
    shutil.copyfile(ROOT / 'rust-toolchain.toml', output / 'rust-toolchain.toml')
    shutil.copyfile(ROOT / 'dependencies/source-lock.json', output / 'prepared-source-lock.json')
    patches = output / 'patches'; patches.mkdir()
    for name in git('ls-files', 'patches').splitlines():
        path = ROOT / name
        if path.is_file():
            destination = output / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
    (output / 'README.md').write_text('''# Connect Rust SDK candidate

Unpublished candidate source. `archives/` contains registry-format `.crate`
archives, including exact modified upstream sources and original manifests.
`vendor/` is a checksummed Cargo directory source. The source replacement in
`.cargo/config.toml` resolves ordinary version dependencies without a repository,
network, prepopulated Cargo cache or `.deps` paths. Keep this directory intact.

Run from this directory with Rust 1.97.1 and the documented native prerequisites:

    cargo check --offline --manifest-path examples/rust-sdk-journey/Cargo.toml

The example communicates through the public connect-client API. Its execution
requires the matching server/helper candidate and private endpoint configuration.
Source assembly is not a native build, runtime or full SDK qualification.
No publication is authorized; every prepared crate disables publication.
''')
    files = {p.relative_to(output).as_posix(): sha256(p) for p in output.rglob('*') if p.is_file()}
    record = {'format': 'connect/rust-sdk-source-candidate/1', 'source_revision': revision,
              'version': package['version'], 'prepared_packages': prepared, 'registry_packages': registry,
              'inputs': inputs, 'files': files, 'native_assets_included': False,
              'publication_authorized': False, 'task_complete': False}
    (output / 'manifest.json').write_text(json.dumps(record, indent=2, sort_keys=True) + '\n')
    if git('status', '--porcelain') or git('rev-parse', 'HEAD') != revision or any(sha256(ROOT / name) != value for name, value in inputs.items()):
        raise ValueError('Source changed during candidate assembly')
    print(json.dumps({'source_revision': revision, 'prepared_packages': len(prepared), 'registry_packages': len(registry), 'manifest_sha256': sha256(output / 'manifest.json')}))


if __name__ == '__main__':
    main()
