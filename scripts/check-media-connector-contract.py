#!/usr/bin/env python3
"""Execute the public outbound-PCMU fixture harness with private build logs.

The selected independently buildable package must invoke the reusable Rust
harness. No precomputed report/pass-fail input is accepted. This component lane
cannot qualify a stock Connect application, provider, network or model response.
"""
import argparse
import hashlib
import json
import os
import shutil
import tomllib
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PROFILE = 'connect-live-media-fixture-v1'
CHECKS = {
    'staged_admission_uses_current_effect_custody_and_exact_original_intent',
    'actual_duplex_pcmu_decodes_speaker_markers_and_owned_end_stops_both_queues',
    'stale_buffered_generation_is_discarded_at_the_actual_decode',
    'revocation_expiry_and_backpressure_fence_media_instead_of_inventing_health',
    'closed_terminal_queue_uses_owned_fallback_and_restart_keeps_original_effect',
    'capabilities_and_receipt_retention_refuse_unsupported_or_excess_work_before_effect',
    'cancelling_drain_retains_the_original_join_for_cleanup_retry',
    'guarded_core_registers_atomic_outbound_fixture_and_host_cancel_is_terminal',
}
CUSTODY_CHECKS = {
    'staged_admission_uses_current_effect_custody_and_exact_original_intent',
    'revocation_expiry_and_backpressure_fence_media_instead_of_inventing_health',
}
UNSUPPORTED = ['video', 'inbound', 'network', 'live-provider', 'model-response',
               'process-crash-persistence', 'stock-application-journey']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def inputs(package):
    paths = {}
    def add(path, label):
        if path.is_file():
            paths.setdefault(str(path), {"label": label, "sha256": digest(path)})
    for name in ['Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
                 'scripts/check-media-connector-contract.py', 'docs/CONNECTORS.md',
                 'contracts/connectors/live-fixture-v1.md']:
        add(ROOT / name, name)
    metadata = json.loads(subprocess.check_output([
        'cargo', 'metadata', '--offline', '--locked', '--format-version=1',
        '--manifest-path', str(package / 'Cargo.toml'), '--features', 'conformance'],
        text=True, stderr=subprocess.PIPE, timeout=30))
    for dependency in metadata['packages']:
        if dependency['source'] is not None:
            continue
        directory = Path(dependency['manifest_path']).parent
        prefix = (str(directory.relative_to(ROOT)) if directory.is_relative_to(ROOT)
                  else f"prepared/{dependency['name']}-{dependency['version']}")
        for path in directory.rglob('*'):
            if (path.is_file() and (path.suffix in ('.rs', '.toml', '.lock', '.json')
                                    or path.name == 'build.rs')
                    and not any(part in ('target', '.git', '.local') for part in path.relative_to(directory).parts)):
                add(path, prefix + '/' + str(path.relative_to(directory)))
        # Workspace inheritance is also a compilation input, including prepared
        # engine parents outside the fixture's independent Cargo workspace.
        for distance, parent in enumerate(directory.parents):
            manifest = parent / 'Cargo.toml'
            if manifest.is_file():
                add(manifest, prefix + f'/workspace-{distance}.toml')
    return paths


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest-path', type=Path, default=ROOT / 'examples/media-connector-template/Cargo.toml')
    parser.add_argument('--target-dir', type=Path, required=True, help='Explicit owned build target; no hidden second target')
    parser.add_argument('--work-root', type=Path, default=ROOT / '.local/gate-6/media-conformance')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', help='Reporter target (default: package name)')
    parser.add_argument('--build-timeout', type=int, default=600, choices=range(1, 901), metavar='1..900')
    parser.add_argument('--negative-control', action='store_true', help='Verify actual --broken-custody refusal; never mark fixture passed')
    args = parser.parse_args()
    if args.output.exists():
        parser.error('Output already exists; choose a fresh report path')
    manifest = args.manifest_path.resolve(strict=True)
    guard = inputs(manifest.parent)
    scratch = args.work_root.resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='media-', dir=scratch) as directory:
        log = Path(directory) / 'build.log'
        binary_name = args.binary or tomllib.loads(manifest.read_text())['package']['name']
        command = ['cargo', 'build', '--release', '--offline', '--locked', '--manifest-path', str(manifest),
                   '--features', 'conformance', '--bin', binary_name,
                   '--message-format=json', '--target-dir', str(args.target_dir.resolve())]
        try:
            build = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=args.build_timeout)
        except subprocess.TimeoutExpired as error:
            (scratch / 'latest-build.log').write_bytes(error.stderr or b'')
            print('Media fixture exceeded its bounded build deadline; no pass recorded')
            return 1
        log.write_text(build.stderr)
        (scratch / 'latest-build.log').write_text(build.stderr)
        (scratch / 'latest-artifacts.jsonl').write_text(build.stdout)
        if build.returncode:
            print('Media fixture compilation failed; see private latest-build.log')
            return 1
        artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
        binaries = {row['executable'] for row in artifacts if row.get('reason') == 'compiler-artifact'
                    and row['target']['name'] == binary_name and row.get('executable')}
        if len(binaries) != 1:
            print('Expected exactly one compiled reporter artifact; no pass recorded')
            return 1
        original = Path(binaries.pop())
        executable = Path(directory) / ('reporter.exe' if os.name == 'nt' else 'reporter')
        shutil.copy2(original, executable)
        binary_sha256 = digest(executable)
        if (digest(original) != binary_sha256 or original.stat().st_ino == executable.stat().st_ino
                or not os.access(executable, os.X_OK)):
            print('Reporter executable seal failed; no pass recorded')
            return 1
        execution = [str(executable), '--contract-report']
        if args.negative_control:
            execution.append('--broken-custody')
        try:
            result = subprocess.run(execution, cwd=ROOT, capture_output=True, text=True, timeout=60)
        except subprocess.TimeoutExpired as error:
            (scratch / 'latest-runtime.log').write_bytes(error.stderr or b'')
            print('Media fixture exceeded its 60 second runtime deadline; no pass recorded')
            return 1
        (scratch / 'latest-runtime.log').write_text(result.stderr)
        if digest(executable) != binary_sha256:
            print('Reporter executable changed during execution; no pass recorded')
            return 1
        try:
            report = json.loads(result.stdout)
            checks = report['checks']
            names = [item['name'] for item in checks]
            valid_shape = (report['profile'] == PROFILE and report['manifest']['version'] == 2
                           and report['manifest']['media']['kind'] == 'live'
                           and len(names) == len(CHECKS) and set(names) == CHECKS
                           and all(type(item['passed']) is bool for item in checks)
                           and report['unsupported'] == UNSUPPORTED)
            failed = {item['name'] for item in checks if not item['passed']}
            unchanged = guard == inputs(manifest.parent)
            fixture_passed = valid_shape and not failed and result.returncode == 0 and unchanged
            negative_verified = (args.negative_control and valid_shape and result.returncode == 1
                                 and CUSTODY_CHECKS <= failed and unchanged)
            valid = negative_verified if args.negative_control else fixture_passed
        except (ValueError, KeyError, TypeError):
            print('Media fixture report is missing or invalid; see private latest-build.log')
            return 1
        report.update({'fixture_passed': fixture_passed, 'negative_control_verified': negative_verified,
                       'status': 'expected-refusal' if negative_verified else ('passed' if fixture_passed else 'failed'),
                       'exit_code': result.returncode, 'source_unchanged': unchanged,
                       'source_files': {row['label']: row['sha256'] for row in guard.values()},
                       'build_log_sha256': digest(log), 'runtime_log_sha256': digest(scratch / 'latest-runtime.log'),
                       'binary_sha256': binary_sha256, 'binary_seal': 'independent full-byte copy; distinct inode; hash checked before/after execution',
                       'scope': 'Explicitly simulated outbound PCMU component. No stock application, installed kit, real provider/model, network/browser/Linux/remote or crash-persistence qualification.'})
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({'status': report['status'], 'checks': len(checks), 'failed': sorted(failed),
                          'fixture_passed': fixture_passed, 'negative_control_verified': negative_verified,
                          'source_unchanged': unchanged}))
        return 0 if valid else 1


if __name__ == '__main__':
    raise SystemExit(main())
