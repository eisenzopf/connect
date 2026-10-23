#!/usr/bin/env python3
"""Qualify the optional WebSocket co-located WebRTC media primitive for G6-024."""
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('media_profiles', ROOT / 'scripts/check-gate6-media-profiles.py')
BASE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(BASE)
REQUIRED = ['delayed_bidirectional_opus_is_automatically_attached_and_decodes_after_connection_wait',
            'closing_idle_bridge_joins_track_attacher_and_cannot_recreate_media',
            'media_bridge_substrate_setup_loopback', 'ws_to_ws_bridge_flows_frames_end_to_end',
            'envelope_only_sdp_exchange_completes_dtls_handshake',
            'trickle_ice_candidate_api_round_trips_between_bridges',
            'end_of_candidates_marker_serializes_with_empty_candidate',
            'outbound_trickle_pump_forwards_candidates_as_envelopes',
            'ice_candidate_init_round_trips_through_wire_json', 'loopback_auth_handshake_over_wss']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    inputs = BASE.source_inputs()
    for name in ('scripts/check-gate6-websocket-media.py', 'docs/developer/websocket-media.md',
                 '.deps/rvoip/Cargo.lock', '.deps/rvoip/crates/uctp/rvoip-websocket/Cargo.toml'):
        inputs[name] = digest(ROOT / name)
    return dict(sorted(inputs.items()))


def passed_tests(raw):
    # Some inherited tracing subscribers write directly to stdout, between
    # libtest's name prefix and its terminal status. Single-thread execution
    # preserves the pairing; also require the final summary count to agree.
    tests=[];pending=None
    for line in raw.splitlines():
        m=re.match(r'^test (\S+) \.\.\. ?(.*)$',line)
        if m:
            assert pending is None, 'unterminated libtest case'
            name,rest=m.groups()
            if rest=='ok': tests.append(name)
            elif rest.startswith('ignored') or rest=='FAILED': pass
            else: pending=name
        elif line in ('ok','FAILED') and pending is not None:
            if line=='ok':tests.append(pending)
            pending=None
    summary=re.findall(r'^test result: (?:ok|FAILED)\. (\d+) passed;',raw,re.M)
    if summary:assert sum(map(int,summary))==len(tests), 'libtest summary/name mismatch'
    return tests


def main():
    directory = ROOT / '.local/gate-6/websocket-media' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    steps = []

    def run(name, command, minimum=0, artifact=None):
        log = directory / (name + '.log')
        env = dict(os.environ)
        if artifact:
            env['CARGO_MANIFEST_DIR'] = str(pathlib.Path(artifact['manifest']).parent)
            assert digest(pathlib.Path(command[0])) == artifact['sha256']
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = passed_tests(raw)
        ignored = re.findall(r'^test (.+?) \.\.\. ignored[^\n]*$', raw, re.M)
        binaries = {str(pathlib.Path(command[0]).relative_to(ROOT)): digest(pathlib.Path(command[0]))} if artifact else {}
        passed = result.returncode == 0 and len(tests) >= minimum
        if artifact:
            passed = passed and artifact['sha256'] in binaries.values()
        steps.append(dict(name=name, command=command,
                          environment={'CARGO_MANIFEST_DIR': env['CARGO_MANIFEST_DIR']} if artifact else {},
                          passed=passed, exit_code=result.returncode, tests_passed=tests, tests_ignored=ignored,
                          log=str(log.relative_to(ROOT)), log_sha256=digest(log), test_binaries_sha256=binaries))
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests), ignored=len(ignored))), flush=True)
        return passed

    ok = run('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
    if ok:
        ok = run('format', ['bash', 'scripts/format.sh', '--check'])
    files = re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / 'patches/rvoip/gate6-websocket-media.patch').read_text(), re.M)
    if ok:
        ok = run('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in files])
    artifacts = {}
    targets = ['media_bridge_loopback', 'ws_bridge_flow', 'ws_envelope_sdp_bridge', 'trickle_ice', 'wss_loopback']
    if ok:
        command = ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml',
                   '-p', 'rvoip-websocket', '-p', 'rvoip-webrtc', '--features', 'media-webrtc,wss,rvoip-core/opus', '--lib']
        for target in targets:
            command += ['--test', target]
        ok = run('build', command + ['--no-run', '--message-format=json'])
        for line in (directory / 'build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'], sha256=digest(pathlib.Path(item['executable'])))
        ok = ok and set(artifacts) == set(targets + ['rvoip_websocket','rvoip_webrtc'])
        steps[-1]['passed'] = ok
    for name, minimum in [('media_bridge_loopback', 3), ('ws_bridge_flow', 1), ('ws_envelope_sdp_bridge', 1),
                          ('trickle_ice', 4), ('wss_loopback', 1), ('rvoip_websocket', 0)]:
        if not ok:
            break
        artifact = artifacts[name]
        ok = run(name, [artifact['executable'], '--test-threads=1'], minimum, artifact)
    if ok:
        artifact = artifacts['rvoip_webrtc']
        ok = run('webrtc-media', [artifact['executable'], 'media::', '--test-threads=1'], 48, artifact)
    if ok:
        ok = run('server-integration', ['cargo', 'check', '--locked', '--offline', '-j2', '-p', 'connect-server'])
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(set(REQUIRED) - observed)
    unchanged = before == source_inputs()
    prior = ROOT / 'docs/evidence/gate-6/media-profiles.json'
    report = dict(task='G6-024', implementation_status='in-progress', full_task_qualification=False,
                  qualification='Local optional WebSocket co-located WebRTC primitive: automatically attached delayed bidirectional mono Opus with actual decode, Core-bridged decoded audio without manual priming, owned idle shutdown, SDP-envelope/trickle/WSS/library regressions. Reuses existing isolated backend signaling fixtures.',
                  not_qualified=['Production canonical Store/journal/service/HTTP/SDK installation, live browser and additional WebSocket media profiles.',
                                 'G6-028 combined upstream qualification; Linux x86_64, named remote/provider G6-093 rehearsals and full Gate 6 acceptance/publication.'],
                  prior_component=dict(path=str(prior.relative_to(ROOT)), sha256=digest(prior), independently_preserved=True, runtime_totals_not_combined=True),
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  required_cases=REQUIRED, missing_required_cases=missing,
                  passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/websocket-media.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('passed', 'runtime_tests', 'runtime_unique_tests', 'sources_unchanged', 'missing_required_cases')}))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
