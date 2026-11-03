#!/usr/bin/env python3
"""Install verified local runtime/Node archives for a scoped app rehearsal.

This consumes actual producer proofs and performs an ordinary offline npm
installation. It does not create or claim a complete qualified SDK candidate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
import shutil
import subprocess

from lib.package_environment import fixture_environment
from lib.sdk_candidate import digest, install_runtime
from lib.sdk_runtime_build import require_bundled_opus

ROOT = Path(__file__).resolve().parents[1]


def checked_file(path, parent):
    path = path.absolute()
    if (path.is_symlink() or not path.is_file() or path.resolve() != path
            or not path.is_relative_to(parent.resolve())):
        raise ValueError('Component input is not an ordinary owned file')
    return path


def install(runtime_path, runtime_proof_path, node_path, node_proof_path, output):
    os.umask(0o077)
    runtime_path = checked_file(runtime_path, ROOT / 'dist')
    node_path = checked_file(node_path, ROOT / 'dist')
    runtime_proof_path = checked_file(runtime_proof_path, ROOT / '.local')
    node_proof_path = checked_file(node_proof_path, ROOT / '.local')
    runtime = json.loads(runtime_proof_path.read_text())
    node = json.loads(node_proof_path.read_text())
    revision = runtime['source_revision']
    target = runtime['target']
    expected_host = {'aarch64-apple-darwin': ('Darwin', 'arm64'),
                     'aarch64-unknown-linux-gnu': ('Linux', 'aarch64'),
                     'x86_64-unknown-linux-gnu': ('Linux', 'x86_64')}
    if target in ('x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'):
        release = platform.freedesktop_os_release()
        if release.get('ID') != 'ubuntu' or release.get('VERSION_ID') != '24.04':
            raise ValueError('Native Ubuntu24.04 component consumer required')
    arm64_component = target == 'aarch64-unknown-linux-gnu'
    if arm64_component and runtime.get('arm64_component_only') is not True:
        raise ValueError('Explicit ARM64 component provenance required')
    if (target not in expected_host or (platform.system(), platform.machine()) != expected_host[target]
            or not re.fullmatch('[0-9a-f]{40}', revision)
            or runtime.get('format') != 'connect/sdk-native-runtime-candidate/1'
            or runtime.get('passed') is not True or runtime.get('production_release') is not True
            or runtime.get('test_support') is not False or runtime.get('source_inputs_unchanged') is not True
            or runtime['archive_name'] != runtime_path.name or digest(runtime_path) != runtime['archive_sha256']):
        raise ValueError('Native producer, host or archive identity differs')
    require_bundled_opus(runtime)
    if (node.get('format') != 'connect/installed-node-sdk-candidate/1'
            or node.get('passed') is not True or node.get('source_inputs_unchanged') is not True
            or node.get('consumer_outside_repository') is not True or node.get('empty_cache') is not True
            or node.get('install_scripts_disabled') is not True or node.get('external_declarations_checked') is not True
            or node.get('package_name') != '@rudeless/connect' or node.get('version') != runtime['version']
            or node['archive_name'] != node_path.name or digest(node_path) != node['archive_sha256']):
        raise ValueError('Node producer or archive identity differs')
    node_inputs_path = checked_file(node_proof_path.parent / 'source-inputs.json', ROOT / '.local')
    if digest(node_inputs_path) != node['source_inputs_sha256']:
        raise ValueError('Node producer input inventory differs')
    node_inputs = json.loads(node_inputs_path.read_text())
    # Reuse the producer's exact inventory, then bind its bytes to the runtime
    # revision. A matching version label alone cannot join unrelated sources.
    current_inputs = runpy.run_path(str(ROOT / 'sdk/tests/check_node_installation.py'))['inputs']()
    if node_inputs != current_inputs:
        raise ValueError('Current Node source differs from its package proof')
    for name, fact in node_inputs.items():
        original = subprocess.check_output(['git', 'show', revision + ':' + name], cwd=ROOT)
        if hashlib.sha256(original).hexdigest() != fact['sha256']:
            raise ValueError('Node archive and native runtime have different source inputs')
    dependencies = {node['package_name']: 'file:' + str(node_path)}
    for item in node['dependency_archives']:
        leaf = item['file_name']
        if Path(leaf).name != leaf or not re.fullmatch(r'(?:@[a-z0-9._-]+/)?[a-z0-9._-]+', item['name']):
            raise ValueError('Dependency identity or archive path differs')
        path = checked_file(node_path.parent / 'dependencies' / leaf, ROOT / 'dist')
        if digest(path) != item['sha256'] or item['name'] in dependencies:
            raise ValueError('Dependency archive differs')
        dependencies[item['name']] = 'file:' + str(path)
    output = output.absolute()
    if (not output.is_relative_to((ROOT / '.local').resolve()) or output.exists() or output.is_symlink()
            or output.parent.resolve() != output.parent):
        raise ValueError('Use a new ordinary private output directory')
    output.mkdir(parents=True, mode=0o700)
    native = output / 'runtime'
    install_runtime(runtime_path, runtime, native)
    consumer = output / 'consumer'
    consumer.mkdir(mode=0o700)
    shutil.copytree(ROOT / 'examples/parley-on-connect/app', consumer / 'app')
    (consumer / 'package.json').write_text(json.dumps(dict(private=True, type='module', dependencies=dependencies)) + '\n')
    empty = output / 'empty-npmrc'
    empty.touch()
    env = fixture_environment()
    env.update(NPM_CONFIG_USERCONFIG=str(empty), NPM_CONFIG_GLOBALCONFIG=str(output / 'unused-global-npmrc'),
               NPM_CONFIG_CACHE=str(output / 'cold-cache'))
    with (output / 'install.log').open('wb') as log:
        subprocess.run(['npm', 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund'],
                       cwd=consumer, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=90, check=True)
    result = dict(format='connect/reference-app-local-components/1', target=target, source_revision=revision,
        arm64_component_only=arm64_component,
        runtime_proof_sha256=digest(runtime_proof_path), runtime_archive_sha256=digest(runtime_path),
        node_proof_sha256=digest(node_proof_path), node_inputs_sha256=digest(node_inputs_path),
        manifest_sha256=runtime['manifest_sha256'], binary_sha256=runtime['binary_sha256'],
        node_archive_sha256=node['archive_sha256'], regular_offline_install=True,
        complete_sdk_candidate_qualified=False, source_inputs_matched=True,
        verification_scope='Ordinary offline component install only; full SDK candidate and app behavior remain unqualified.',
        installer_sha256=digest(Path(__file__)), consumer=str(consumer), executable=str(native / 'bin/connect'),
        passed=True, task_complete=False, whole_gate_qualified=False)
    (output / 'installation.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: result[key] for key in ['format', 'target', 'passed', 'complete_sdk_candidate_qualified']}))
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['runtime', 'runtime-proof', 'node', 'node-proof', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    install(args.runtime, args.runtime_proof, args.node, args.node_proof, args.output)
