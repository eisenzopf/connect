#!/usr/bin/env python3
"""Source-bound Linux browser media build/run lanes; no named remote/account calls."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import platform
import re
import runpy
import shutil
import subprocess
import threading

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / '.local/gate6-browser-media'
INPUTS = runpy.run_path(str(ROOT / 'scripts/check-gate6-ice-restart.py'))['inputs']
BOUNDED = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-turn-relay.py'))['bounded_child']
RUNTIME_LIBRARIES = frozenset({'connect_client', 'connect_server', 'connect_service', 'connect_store',
    'rvoip_core', 'rvoip_media_core', 'rvoip_rtp_core', 'rvoip_webrtc', 'rtc', 'rtc_ice', 'rtc_dtls'})


def runtime_library_profiles(rows):
    profiles = {}
    artifacts = 0
    for row in rows:
        if row.get('reason') != 'compiler-artifact':
            continue
        artifacts += 1
        name = row.get('target', {}).get('name')
        if name in RUNTIME_LIBRARIES and row['target'].get('kind') == ['lib']:
            level = row.get('profile', {}).get('opt_level')
            profiles.setdefault(name, set()).add(level if level in {'0', '1', '2', '3', 's', 'z'} else 'invalid')
    missing = sorted(RUNTIME_LIBRARIES - profiles.keys())
    return dict(completed_artifacts=artifacts, profiles={name:sorted(values) for name, values in sorted(profiles.items())},
        missing=missing, passed=not missing and all(values == {'3'} for values in profiles.values()))


def read_build_rows(path):
    rows = []
    if path.exists():
        with path.open(errors='replace') as stream:
            for line in stream:
                try:
                    row = json.loads(line)
                    if isinstance(row, dict):
                        rows.append(row)
                except ValueError:
                    continue
    return rows


def report_build_progress(path, stopped):
    previous = None
    while not stopped.wait(30):
        progress = runtime_library_profiles(read_build_rows(path))
        if progress != previous:
            # Only fixed library labels, optimization levels and a count leave
            # private compiler output. No paths, package IDs or diagnostics.
            print(json.dumps(dict(build_progress=progress)), flush=True)
            previous = progress


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def build():
    fixture_level = os.environ.get('CONNECT_GATE6_FIXTURE_OPT_LEVEL', '3')
    assert fixture_level in {'0', '3'}, 'Use explicit fixture optimization0 or3; production remains opt3'
    output = BASE / 'evidence/build'
    output.mkdir(parents=True, exist_ok=False)
    subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True)
    before = INPUTS()
    save(output / 'source-inputs.json', before)
    command = ['cargo', 'rustc', '--release', '--offline', '--locked', '-j1',
        '-p', 'connect-server', '--features', 'test-support', '--test',
        'gate6_canonical_webtransport', '--message-format=json', '--', '-C', 'strip=none',
        '-C', 'opt-level=' + fixture_level]
    stopped = threading.Event()
    progress = threading.Thread(target=report_build_progress, args=(output / 'build.log', stopped))
    progress.start()
    try:
        result = BOUNDED(command, dict(os.environ), output / 'build.log', 3600)
    finally:
        stopped.set()
        progress.join()
    rows = read_build_rows(output / 'build.log')
    runtime_profiles = runtime_library_profiles(rows)
    artifacts = [row['executable'] for row in rows if row.get('reason') == 'compiler-artifact'
        and row['target']['name'] == 'gate6_canonical_webtransport' and row.get('executable')]
    unchanged = INPUTS() == before
    report = dict(scope='Native Linux x86_64 production runtime libraries at release opt3, no root package exceptions; integration test driver opt' + fixture_level + '. Build only.',
        fixture_opt_level=fixture_level,
        command=command, execution=result, runtime_library_profiles=runtime_profiles,
        source_count=len(before), sources_unchanged=unchanged,
        source_manifest_sha256=digest(output / 'source-inputs.json'),
        artifacts=[dict(path=value, sha256=digest(value)) for value in artifacts],
        passed=result['exit_code'] == 0 and not result['timed_out'] and not result['forced_kill'] and len(artifacts) == 1 and unchanged and runtime_profiles['passed'])
    save(output / 'summary.json', report)
    if os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a') as stream:
            stream.write('source_unchanged=' + str(unchanged).lower() + '\n')
    # A bounded cold build can leave useful completed artifacts. Preserve the
    # verified source timestamps for those too, without claiming a producer.
    # Otherwise a restored checkout invalidates partial-cache fingerprints.
    if unchanged:
        cache = runpy.run_path(str(ROOT / 'scripts/lib/source_build_cache.py'))
        cache['MANIFEST'].parent.mkdir(parents=True, exist_ok=True)
        cache['MANIFEST'].write_text(json.dumps({'format':1, 'files':cache['snapshot'](ROOT)}, sort_keys=True) + '\n')
    assert report['passed'], 'Source-bound browser producer failed; private build.log retained'


def runtime():
    assert os.environ.get('CONNECT_GATE6_OWNED_BROWSER_CONTAINER') == '1' and os.environ.get('PULSE_SERVER')
    assert re.fullmatch(r'sha256:[0-9a-f]{64}', os.environ.get('CONNECT_GATE6_BROWSER_IMAGE_ID', ''))
    output = BASE / 'evidence/runtime'
    output.mkdir(parents=True, exist_ok=False)
    producer = json.loads((BASE / 'evidence/build/summary.json').read_text())
    assert producer['passed'] and INPUTS() == json.loads((BASE / 'evidence/build/source-inputs.json').read_text())
    binary = Path(producer['artifacts'][0]['path'])
    assert digest(binary) == producer['artifacts'][0]['sha256']
    chrome = Path(os.environ['CONNECT_GATE6_CHROME']).resolve(strict=True)
    module = Path(os.environ['CONNECT_PLAYWRIGHT_MODULE']).resolve(strict=True)
    node = Path(shutil.which('node')).resolve(strict=True)
    version = subprocess.check_output([str(chrome), '--version'], text=True).strip()
    assert version == 'Google Chrome for Testing 156.0.8078.4'
    repository = INPUTS()
    for path in (ROOT / 'examples/conversation-client').rglob('*'):
        if path.is_file():
            repository[str(path.relative_to(ROOT))] = digest(path)
    save(output / 'repository-inputs.json', repository)
    tools = {str(path):digest(path) for path in [binary, chrome, node,
        Path(shutil.which('pulseaudio')).resolve(strict=True),
        Path('/usr/local/share/connect-browser-packages.tsv')]}
    for folder in [module.parent, module.parent.parent / 'playwright-core']:
        for path in folder.rglob('*'):
            if path.is_file() and path.suffix in ('.js', '.mjs', '.json'):
                tools[str(path)] = digest(path)
    save(output / 'tool-inputs.json', tools)
    env = dict(os.environ, CONNECT_GATE6_NODE=str(node), CONNECT_GATE6_ICE_DIAGNOSTICS='1')
    failure_diagnostic = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['rtc_operation_failure_diagnostic']
    worker_diagnostic = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['rtc_ice_worker_failure_diagnostic']
    steps = []
    try:
        clock = BOUNDED([str(node), 'scripts/video/check-audio-clock.mjs', str(output / 'clock')],
            env, output / 'clock.log', 40)
        assert clock['exit_code'] == 0 and not clock['timed_out'] and not clock['forced_kill'], 'Unpaced software audio cannot qualify media'
        cases = [
            ('native-ice', 'signed_rtc_ice_restart', False, 2),
            ('baseline', 'canonical_webrtc_browser::canonical_chrome_signed_av_normal_trust_clean_eddsa', True, 1),
            ('ice-eddsa', 'canonical_webrtc_browser::canonical_chrome_original_peer_ice_restart_clean_eddsa', True, 1),
            ('ice-es256', 'canonical_webrtc_browser::canonical_chrome_original_peer_ice_restart_clean_es256', True, 1),
            ('unreachable-eddsa', 'canonical_webrtc_browser::canonical_chrome_unreachable_turn_eddsa', True, 1),
            ('unreachable-es256', 'canonical_webrtc_browser::canonical_chrome_unreachable_turn_es256', True, 1),
        ]
        for name, case, ignored, count in cases:
            directory = output / name
            directory.mkdir()
            command = [str(binary), case] + (['--exact', '--ignored'] if ignored else []) + ['--test-threads=1']
            step = BOUNDED(command, dict(env, CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS=str(directory)),
                output / (name + '.log'), 120)
            raw = (output / (name + '.log')).read_text(errors='replace')
            step.update(name=name, passed=step['exit_code'] == 0 and not step['timed_out'] and not step['forced_kill'] and re.findall(
                r'^test result: ok\. (\d+) passed; 0 failed;', raw, re.M) == [str(count)])
            step['rtc_operation_failures'] = [failure_diagnostic(json.loads(line.split(' ', 1)[1])) for line in raw.splitlines()
                if line.startswith('rtc-operation-failure ')]
            assert len(step['rtc_operation_failures']) <= 16
            step['rtc_ice_worker_failures'] = [worker_diagnostic(json.loads(line.split(' ', 1)[1])) for line in raw.splitlines()
                if line.startswith('rtc-ice-worker-failure ')]
            assert len(step['rtc_ice_worker_failures']) <= 16
            for filename in ['browser-result.jsonl', 'ownership.json', 'ice-restart-oracle.json',
                             'unreachable-relay-oracle.json', 'unreachable-turn.json']:
                path = directory / filename
                if path.is_file() and path.stat().st_size:
                    value = json.loads(path.read_text())
                    if filename == 'browser-result.jsonl':
                        value = dict(status=value.get('status'), phase=value.get('phase'), error_class=value.get('error_class'), ice_restarts=value.get('ice_restarts'),
                            unreachable_relay=value.get('unreachable_relay'), observations=[dict(
                                decoded_fps=o['decoded']/o['elapsed_seconds'], changing_fps=o['changing']/o['elapsed_seconds'],
                                audio=o['audio_profile']['status'], motion=o['motion_profile']['status'],
                                source_identity=o['source_identity']['status'], av_sync=o['av_sync']['status'],
                                av_acceptance=o['av_acceptance']) for o in value.get('observations', [])])
                    step[filename] = value
            steps.append(step)
            print(json.dumps(dict(name=name, passed=step['passed'])), flush=True)
        for algorithm in ['EdDSA', 'ES256']:
            name = 'impairment-' + algorithm.lower()
            command = ['python3', 'scripts/rehearse-gate6-browser-network.py', '--binary', str(binary),
                '--source-manifest', str(output / 'repository-inputs.json'), '--algorithm', algorithm,
                '--output', str(output / name)]
            step = BOUNDED(command, env, output / (name + '.log'), 140)
            step.update(name=name, passed=step['exit_code'] == 0 and not step['timed_out'] and not step['forced_kill'])
            path = output / name / 'summary.json'
            if path.exists():
                step['network'] = json.loads(path.read_text())
            steps.append(step)
            print(json.dumps(dict(name=name, passed=step['passed'])), flush=True)
    finally:
        changed = [name for name, value in repository.items() if not (ROOT / name).is_file() or digest(ROOT / name) != value]
        changed += [name for name, value in tools.items() if digest(name) != value]
        report = dict(scope='Native Linux x86_64 anonymous network-disabled container: production runtime opt3 with explicitly recorded integration-driver optimization, normal private CA, original browser ICE, decoded clean/impairment media and owned UDP faults. No distinct-network, provider, installation or acceptance qualification.',
            browser=version, image_id=os.environ.get('CONNECT_GATE6_BROWSER_IMAGE_ID'),
            producer=producer, source_count=len(repository), tool_count=len(tools),
            sources_unchanged=not changed, changed_inputs=changed, steps=steps,
            passed=len(steps) == 8 and all(step['passed'] for step in steps) and not changed)
        save(output / 'summary.json', report)
    assert report['passed'], 'One or more required browser media cases failed; independent results retained'


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('build', 'run'))
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'x86_64'
    assert re.search(r'^vendor_id\s*:', Path('/proc/cpuinfo').read_text(), re.M)
    os.chdir(ROOT)
    build() if args.mode == 'build' else runtime()
