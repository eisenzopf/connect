#!/usr/bin/env python3
"""Install the exact independently verified G6-069 candidate for app checks.

No compilation, publishing or mutable registry resolution. This downloads one
retained public CI artifact, verifies its original ZIP/tar/manifest identities,
and installs only the selected native and npm packages in a private consumer.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import urllib.request
import zipfile
from lib.sdk_candidate import digest, install_runtime
from lib.package_environment import fixture_environment

ROOT = Path(__file__).resolve().parents[1]


def install(target, output):
    os.umask(0o077)
    output = output.resolve()
    if not output.is_relative_to(ROOT / '.local'):
        raise ValueError('Use an isolated ignored application fixture')
    output.mkdir(parents=True, exist_ok=False)
    public = json.loads((ROOT / 'docs/evidence/gate-6/sdk-candidates-verified.json').read_text())
    platform = public['platforms'][target]
    proof = platform['sealed_verification']
    if not (proof['passed'] and proof['source_revision'] == public['source_revision']):
        raise ValueError('Candidate qualification identity differs')
    artifact = platform['artifacts']['complete_sdk']
    request = urllib.request.Request(f'https://api.github.com/repos/eisenzopf/connect/actions/artifacts/{artifact}/zip',
        headers={'Authorization': 'Bearer ' + os.environ['GH_TOKEN'], 'Accept': 'application/vnd.github+json',
                 'X-GitHub-Api-Version': '2022-11-28'})
    # Strip credentials at the cross-origin redirect to the signed artifact URL.
    class Redirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
            if redirected is not None:
                redirected.remove_header('Authorization')
            return redirected
    archive_zip = output / 'candidate.zip'
    with urllib.request.build_opener(Redirect()).open(request, timeout=60) as response, archive_zip.open('xb') as out:
        total = 0
        while block := response.read(1024 * 1024):
            total += len(block)
            if total > 512 * 1024 * 1024:
                raise ValueError('Candidate ZIP size budget')
            out.write(block)
    if digest(archive_zip) != proof['artifact_zip_digests']['package']:
        raise ValueError('Candidate ZIP identity differs')
    archive = output / 'candidate.tar.gz'
    with zipfile.ZipFile(archive_zip) as source:
        names = [name for name in source.namelist() if name.endswith('.tar.gz')]
        if len(names) != 1 or source.getinfo(names[0]).file_size > 512 * 1024 * 1024:
            raise ValueError('Candidate archive inventory differs')
        with source.open(names[0]) as src, archive.open('xb') as dst:
            shutil.copyfileobj(src, dst)
    if digest(archive) != proof['archive_sha256']:
        raise ValueError('Candidate tar identity differs')
    prefix = f"connect-sdk-0.1.0-{target}-{public['source_revision'][:12]}"
    selected = output / 'selected'
    selected.mkdir()
    with tarfile.open(archive, 'r:gz') as source:
        member = source.getmember(prefix + '/manifest.json')
        if not member.isfile() or member.size > 16 * 1024 * 1024:
            raise ValueError('Candidate manifest bounds')
        encoded = source.extractfile(member).read()
        if hashlib.sha256(encoded).hexdigest() != proof['manifest_sha256']:
            raise ValueError('Candidate manifest identity differs')
        manifest = json.loads(encoded)
        if manifest['source_revision'] != public['source_revision'] or manifest['target'] != target:
            raise ValueError('Candidate source/platform differs')
        chosen = [name for name in manifest['files'] if name.startswith(('sdk/node/', 'runtime/')) or name in ['verification/node.json', 'verification/runtime.json']]
        for name in chosen:
            path = PurePosixPath(name)
            if path.is_absolute() or '..' in path.parts or str(path) != name:
                raise ValueError('Candidate member path refused')
            member = source.getmember(prefix + '/' + name)
            if not member.isfile() or member.size > 512 * 1024 * 1024:
                raise ValueError('Candidate member type/size refused')
            destination = selected.joinpath(*path.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as src, destination.open('xb') as dst:
                shutil.copyfileobj(src, dst)
            if digest(destination) != manifest['files'][name]['sha256']:
                raise ValueError('Candidate member checksum differs')
    runtime = json.loads((selected / 'verification/runtime.json').read_text())
    node = json.loads((selected / 'verification/node.json').read_text())
    if (runtime['binary_sha256'] != platform['native_verification']['binary_sha256']
            or runtime['source_revision'] != public['source_revision'] or node['package_name'] != '@rudeless/connect'
            or digest(selected / 'sdk/node' / node['archive_name']) != node['archive_sha256']):
        raise ValueError('Selected package/runtime identities differ')
    runtime_archive = selected / 'runtime' / runtime['archive_name']
    native = output / 'runtime'
    install_runtime(runtime_archive, runtime, native)
    if target == 'aarch64-unknown-linux-gnu':
        linkage = subprocess.check_output(['ldd', str(native / 'bin/connect')], text=True, timeout=5)
        (output / 'runtime-linkage.txt').write_text(linkage)
        if 'not found' in linkage:
            raise ValueError('A documented Ubuntu runtime library is missing')
    consumer = output / 'consumer'
    consumer.mkdir()
    shutil.copytree(ROOT / 'examples/parley-on-connect/app', consumer / 'app')
    dependencies = {node['package_name']: 'file:' + str(selected / 'sdk/node' / node['archive_name'])}
    for dependency in node['dependency_archives']:
        path = selected / 'sdk/node/dependencies' / dependency['file_name']
        if digest(path) != dependency['sha256']:
            raise ValueError('SDK dependency checksum differs')
        dependencies[dependency['name']] = 'file:' + str(path)
    (consumer / 'package.json').write_text(json.dumps(dict(private=True, type='module', dependencies=dependencies)) + '\n')
    empty = output / 'empty-npmrc'
    empty.touch()
    env = fixture_environment()
    env.update(NPM_CONFIG_USERCONFIG=str(empty), NPM_CONFIG_GLOBALCONFIG=str(output / 'unused-global-npmrc'),
               NPM_CONFIG_CACHE=str(output / 'cold-cache'))
    with (output / 'install.log').open('wb') as log:
        subprocess.run(['npm', 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund'],
            cwd=consumer, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=90, check=True)
    result = dict(format='connect/reference-app-candidate-installation/1', target=target,
        source_revision=public['source_revision'], artifact_id=artifact,
        artifact_zip_sha256=digest(archive_zip), archive_sha256=digest(archive),
        manifest_sha256=proof['manifest_sha256'], binary_sha256=runtime['binary_sha256'],
        node_archive_sha256=node['archive_sha256'], regular_offline_install=True,
        consumer=str(consumer), executable=str(native / 'bin/connect'), passed=True,
        task_complete=False, whole_gate_qualified=False)
    (output / 'installation.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: result[key] for key in ['target', 'artifact_id', 'regular_offline_install', 'passed']}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=['aarch64-unknown-linux-gnu', 'aarch64-apple-darwin'])
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    install(args.target, args.output)
