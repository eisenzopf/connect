#!/usr/bin/env python3
"""Install actual SDK/runtime archives outside the repo and join original owners.

Read-only local synthetic signed peers only; no media, providers or model hosts.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import runpy
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import tomllib
import urllib.request
import venv

from lib.package_environment import fixture_environment
from lib.sdk_candidate import digest, install_runtime
from lib.sdk_rust_package import install_archive

ROOT = Path(__file__).resolve().parents[1]

NODE_OBSERVER = r"""
import cp from 'node:child_process';
import {syncBuiltinESMExports} from 'node:module';
import {pathToFileURL} from 'node:url';
import assert from 'node:assert/strict';
const [example,config]=process.argv.slice(2);
const spawn=cp.spawn,children=[];
cp.spawn=function(file,args,options){
    assert.equal(file,'connect');assert.deepEqual(args,['sdk-stdio','--config',config]);
    const child=spawn.call(this,file,args,options);children.push(child);return child;
};
syncBuiltinESMExports();
process.argv=[process.execPath,example,config];
assert.ok(import.meta.resolve('@rudeless/connect').startsWith(pathToFileURL(process.cwd()+'/node_modules/').href));
await import(pathToFileURL(example).href);
assert.equal(children.length,1);
for(const child of children){assert.equal(child.exitCode,0);assert.equal(child.signalCode,null);}
process.stdout.write(JSON.stringify({native_helpers_joined:children.length,actual_default_executable:true})+'\n');
"""

PYTHON_OBSERVER = r"""
import asyncio,json,runpy,sys
from pathlib import Path
import connect_sdk
assert Path(connect_sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
assert 'PYTHONPATH' not in __import__('os').environ
example,config=sys.argv[1:];children=[];spawn=asyncio.create_subprocess_exec
async def capture(*args,**kwargs):
    assert args==('connect','sdk-stdio','--config',config)
    child=await spawn(*args,**kwargs);children.append(child);return child
asyncio.create_subprocess_exec=capture
sys.argv=[example,config];runpy.run_path(example,run_name='__main__')
assert len(children)==1 and all(c.returncode==0 for c in children)
print(json.dumps({'native_helpers_joined':len(children),'actual_default_executable':True}))
"""


def port(kind):
    with socket.socket(socket.AF_INET, kind) as stream:
        stream.bind(('127.0.0.1', 0))
        return stream.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('runtime', 'runtime-proof', 'node-proof', 'python-proof', 'rust-source', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Keep evidence under ignored .local/')
    output.mkdir(parents=True, exist_ok=False)
    proof = json.loads(args.runtime_proof.read_text())
    node = json.loads(args.node_proof.read_text()); python = json.loads(args.python_proof.read_text())
    rust = json.loads((args.rust_source / 'manifest.json').read_text())
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if not (proof['source_revision'] == rust['source_revision'] == head and node['passed'] and python['passed']):
        raise ValueError('Candidate source/component proof differs')
    private = ROOT / '.local/gate6-sdk-native/private' / proof['target']
    private.mkdir(parents=True, mode=0o700, exist_ok=False)
    sources = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['sources']
    before = sources(); (output / 'source-inputs.json').write_text(json.dumps(before, sort_keys=True) + '\n')
    report = {'format': 'connect/installed-sdk-native-candidate/1', 'source_revision': head,
              'target': proof['target'], 'source_inputs_sha256': digest(output / 'source-inputs.json'),
              'runtime_archive_sha256': proof['archive_sha256'], 'steps': [], 'passed': False,
              'scope': 'Clean external archive installations and original read-only native Rust/Node/Python signed peers against matching packaged production server. No full facade/version/media/remote matrix.',
              'task_complete': False, 'whole_gate_qualified': False}
    bounded = runpy.run_path(str(ROOT / 'scripts/check-linux-package-build.py'))['run_bounded']
    server = None; server_log = None
    try:
        with tempfile.TemporaryDirectory(prefix='connect-installed-candidate-') as temporary, contextlib.ExitStack() as ownership:
            project = Path(temporary).resolve()
            if project.is_relative_to(ROOT.resolve()):
                raise ValueError('Consumer must be outside the repository')
            runtime = project / 'runtime'; install_runtime(args.runtime, proof, runtime)
            binary = runtime / 'bin/connect'
            environment = fixture_environment()
            environment.update(PATH=str(runtime / 'bin') + os.pathsep + environment['PATH'],
                RUSTUP_HOME=environment.get('RUSTUP_HOME', str(Path.home() / '.rustup')),
                HOME=str(project / 'cold-home'))
            Path(environment['HOME']).mkdir()
            def run(name, command, cwd=project, budget=30):
                started = time.monotonic(); log = private / (name + '.log')
                status = bounded([str(c) for c in command], log, environment, budget, cwd=cwd)
                report['steps'].append({'name': name, 'exit_code': status,
                    'elapsed_seconds': round(time.monotonic() - started, 3), 'log_sha256': digest(log)})
                if status:
                    raise ValueError('Installed native candidate step failed: ' + name)
                return log.read_text()
            # Only copied SDK examples and the installed binary see this fresh state.
            examples = project / 'examples'; shutil.copytree(ROOT / 'examples/sdk-installation', examples)
            state = project / 'state'; udp = port(socket.SOCK_DGRAM); http = port(socket.SOCK_STREAM)
            run('initialize', [binary, 'init', '--directory', state, '--bind', f'127.0.0.1:{udp}', '--http-bind', f'127.0.0.1:{http}'], budget=15)
            run('configuration', ['python3', examples / 'make-config.py', state])
            config = state / 'operator.sdk.toml'
            application = tomllib.loads((state / 'operator.application.toml').read_text())
            server_log = (private / 'server.log').open('wb')
            server = subprocess.Popen([str(binary), 'serve', '--config', str(state / 'server.toml')],
                cwd=project, env=environment, stdout=server_log, stderr=subprocess.STDOUT, start_new_session=True)
            def retire_owned_server():
                if server.poll() is None:
                    os.killpg(server.pid, signal.SIGKILL)
                    server.wait()
            # Retire the process before TemporaryDirectory removes its state on
            # failure. Success still requires the original graceful join below.
            ownership.callback(retire_owned_server)
            def health():
                request = urllib.request.Request(f'http://127.0.0.1:{http}/v1/operations', data=b'{"op":"health"}',
                    headers={'Content-Type': 'application/json', 'Authorization': 'Bearer ' + application['credential']})
                with urllib.request.urlopen(request, timeout=.5) as response:
                    return json.load(response)['result']['Ok']['data']
            def wait_closed(minimum):
                deadline = time.monotonic() + 10
                while True:
                    if server.poll() is not None:
                        raise ValueError('Original packaged server exited')
                    try:
                        value = health(); operational = value.get('operational')
                        pools = {p['kind']: p.get('usage') for p in (operational or {}).get('resource_pools', []) or []}
                        native = pools.get('native-quic'); ingress = pools.get('quic-ingress')
                        if (value.get('readiness', {}).get('state') == 'ready' and native and ingress
                                and native['used'] == 0 and ingress['used'] == 0 and native['accepted'] >= minimum):
                            return native['accepted']
                    except (OSError, KeyError, ValueError):
                        pass
                    if time.monotonic() >= deadline:
                        raise ValueError('Original peer/readiness reclamation deadline')
                    time.sleep(.05)
            baseline = wait_closed(0)
            # Archives are copied into the external installation first. Neither
            # installed facade needs the repository after its package is installed.
            package_dir = project / 'packages'; package_dir.mkdir()
            node_archive = ROOT / 'dist/sdk/typescript' / args.node_proof.parent.name / node['archive_name']
            if digest(node_archive) != node['archive_sha256']:
                raise ValueError('Node archive changed')
            node_inputs = []
            for original in [node_archive, *[node_archive.parent / 'dependencies' / d['file_name'] for d in node['dependency_archives']]]:
                destination = package_dir / original.name; shutil.copyfile(original, destination); node_inputs.append(destination)
            for value, path in zip(node['dependency_archives'], node_inputs[1:]):
                if digest(path) != value['sha256']:
                    raise ValueError('Node dependency archive changed')
            node_project = examples / 'node'; cache = project / 'empty-npm-cache'
            run('node-install', ['npm', 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', '--cache', cache, *node_inputs], cwd=node_project)
            if (node_project / 'node_modules/@rudeless/connect').is_symlink():
                raise ValueError('Node candidate is not a regular installation')
            for path in node_inputs: path.unlink()
            observer = node_project / 'observe.mjs'; observer.write_text(NODE_OBSERVER)
            lines = run('node-example', ['node', observer, node_project / 'inspect.mjs', config], cwd=node_project, budget=20).splitlines()
            observed, joined = [json.loads(line) for line in lines]
            if observed['profile'] != 'connect/2' or observed['feature_count'] < 4 or joined != {'native_helpers_joined': 1, 'actual_default_executable': True}:
                raise ValueError('Actual Node example/helper observation differs')
            baseline = wait_closed(baseline + 1)
            report['node'] = {'example': observed, 'helper': joined, 'peer_capacity_reclaimed': True}
            wheel = ROOT / 'dist/sdk/python' / args.python_proof.parent.name / python['wheel_name']
            if digest(wheel) != python['wheel_sha256']:
                raise ValueError('Python wheel changed')
            copied_wheel = package_dir / wheel.name; shutil.copyfile(wheel, copied_wheel)
            target = project / 'venv'; venv.EnvBuilder(with_pip=False, symlinks=True).create(target)
            executable = target / 'bin/python'
            run('python-install', ['python3', '-m', 'pip', '--python', executable, 'install', '--no-cache-dir', '--disable-pip-version-check', '--no-index', '--no-deps', copied_wheel])
            copied_wheel.unlink()
            observer = examples / 'python/observe.py'; observer.write_text(PYTHON_OBSERVER)
            lines = run('python-example', [executable, '-I', observer, examples / 'python/inspect_candidate.py', config], budget=20).splitlines()
            observed, joined = [json.loads(line) for line in lines]
            if observed['profile'] != 'connect/2' or observed['feature_count'] < 4 or joined != {'native_helpers_joined': 1, 'actual_default_executable': True}:
                raise ValueError('Actual Python example/helper observation differs')
            baseline = wait_closed(baseline + 1)
            report['python'] = {'example': observed, 'helper': joined, 'peer_capacity_reclaimed': True}
            rust_project = project / 'rust'; rust_project.mkdir(); vendor = rust_project / 'vendor'; vendor.mkdir()
            for value in rust['prepared_packages'] + rust['registry_packages']:
                install_archive(args.rust_source / 'archives' / value['archive'], vendor, value['name'], value['version'], value['sha256'])
            shutil.copytree(args.rust_source / '.cargo', rust_project / '.cargo')
            shutil.copytree(examples / 'rust', rust_project / 'example')
            shutil.copyfile(args.rust_source / 'rust-toolchain.toml', rust_project / 'rust-toolchain.toml')
            environment.update(CARGO_HOME=str(project / 'empty-cargo-home'), CARGO_TARGET_DIR=str(project / 'empty-rust-target'))
            if proof['target'] == 'aarch64-apple-darwin':
                environment.update(MACOSX_DEPLOYMENT_TARGET='15.0', OPUS_STATIC='1',
                                   LIBOPUS_NO_PKG='1', CMAKE_POLICY_VERSION_MINIMUM='3.5')
            if any(Path(environment[key]).exists() for key in ('CARGO_HOME', 'CARGO_TARGET_DIR')):
                raise ValueError('Rust consumer cache was not empty')
            run('rust-resolve', ['cargo', 'generate-lockfile', '--offline', '--manifest-path', 'example/Cargo.toml'], cwd=rust_project, budget=120)
            build = run('rust-build', ['cargo', 'build', '--frozen', '--message-format=json', '--manifest-path', 'example/Cargo.toml'], cwd=rust_project, budget=3600)
            artifacts = []
            for line in build.splitlines():
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                if (value.get('reason') == 'compiler-artifact' and value['target']['name'] == 'connect-installed-rust-example'
                        and value.get('executable') and not value['profile']['test']):
                    artifacts.append(Path(value['executable']).resolve(strict=True))
            consumer, = artifacts
            if not consumer.is_relative_to(Path(environment['CARGO_TARGET_DIR'])):
                raise ValueError('Rust consumer did not build in the external target')
            raw = run('rust-metadata', ['cargo', 'metadata', '--offline', '--locked', '--manifest-path', 'example/Cargo.toml', '--format-version', '1'], cwd=rust_project, budget=120)
            metadata, = [json.loads(line) for line in raw.splitlines() if line.startswith('{') and '"packages"' in line]
            selected = next(p for p in metadata['packages'] if p['name'] == 'connect-client')
            if not selected['source'].startswith('registry+') or any(d.get('path') for p in metadata['packages'] for d in p['dependencies']):
                raise ValueError('Rust SDK retained a checkout dependency')
            observed = json.loads(run('rust-example', [consumer, config], cwd=rust_project, budget=20))
            if observed['profile'] != 'connect/2' or observed['feature_count'] < 4:
                raise ValueError('Actual Rust example observation differs')
            wait_closed(baseline + 1)
            report['rust'] = {'example': observed, 'cold_offline_crate_installation': True, 'peer_capacity_reclaimed': True,
                              'consumer_sha256': digest(consumer), 'resolved_packages': len(metadata['packages'])}
            server.send_signal(signal.SIGTERM)
            if server.wait(timeout=10) != 0 or digest(binary) != proof['binary_sha256']:
                raise ValueError('Original packaged server did not join unchanged')
            report.update(passed=True, original_server_joined=True, consumer_outside_repository=True,
                native_helpers_joined=2, node_archive_sha256=node['archive_sha256'], python_wheel_sha256=python['wheel_sha256'],
                rust_source_manifest_sha256=digest(args.rust_source / 'manifest.json'))
    finally:
        if server is not None and server.poll() is None:
            os.killpg(server.pid, signal.SIGKILL); server.wait()
        if server_log is not None: server_log.close()
        report['source_inputs_unchanged'] = sources() == before
        report['passed'] = report['passed'] and report['source_inputs_unchanged']
        (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({'passed': report['passed'], 'target': report['target']}))
    if not report['passed']:
        raise SystemExit('Installed native candidate remains unqualified')


if __name__ == '__main__':
    main()
