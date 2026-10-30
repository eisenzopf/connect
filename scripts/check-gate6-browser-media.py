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


def browser_participation_states(value):
    allowed = dict(connection={'new','connecting','connected','disconnected','failed','closed'},
        ice={'new','checking','connected','completed','disconnected','failed','closed'},
        gathering={'new','gathering','complete'},
        signaling={'stable','have-local-offer','have-remote-offer','have-local-pranswer','have-remote-pranswer','closed'})
    result = []
    for participant, diagnostic in enumerate(value.get('peer_diagnostics', [])[:2]):
        for peer in diagnostic.get('peers', [])[:4]:
            current = peer.get('current', {})
            row = {name:current.get(name) if isinstance(current.get(name),str) and current.get(name) in states else 'other' for name,states in allowed.items()}
            row.update(participant=participant,
                local_description_set=current.get('local_description_set') is True,
                remote_description_set=current.get('remote_description_set') is True,
                observed_gathering_states=sorted({event.get('gathering') for event in peer.get('events', [])[:128]
                    if isinstance(event.get('gathering'),str) and event.get('gathering') in allowed['gathering']}))
            result.append(row)
    return result


def cpu_snapshot():
    result = dict(logical_cpus=os.cpu_count(), affinity_cpus=len(os.sched_getaffinity(0)))
    try:
        quota,period=Path('/sys/fs/cgroup/cpu.max').read_text().split()
        result['quota_cores']=None if quota=='max' else int(quota)/int(period)
        result['cpu_stat']={key:int(value) for key,value in
            (line.split() for line in Path('/sys/fs/cgroup/cpu.stat').read_text().splitlines())
            if key in {'usage_usec','user_usec','system_usec','nr_periods','nr_throttled','throttled_usec'}}
    except (OSError,ValueError):
        result['available']=False
    return result


def cpu_measurement(before,after,seconds):
    delta={key:after['cpu_stat'][key]-value for key,value in before.get('cpu_stat',{}).items()
        if key in after.get('cpu_stat',{}) and after['cpu_stat'][key]>=value}
    return dict(profile={key:before.get(key) for key in ['logical_cpus','affinity_cpus','quota_cores']},
        cpu_stat_delta=delta,average_cores_used=delta['usage_usec']/1_000_000/seconds
        if 'usage_usec' in delta and seconds>0 else None)


def pipeline_diagnostic(path):
    value=json.loads(path.read_text())
    messages={'Current native packet authority':'packet_authority_refused',
        'Guarded audio delivery refused':'audio_delivery_refused',
        'Guarded audio pending burst trimmed':'audio_burst_trimmed',
        'RTC original packet path refused':'rtc_packet_path_refused'}
    counts={name:0 for name in messages.values()}
    problems={code:0 for code in ['authority-busy','forbidden','storage-unavailable','conflict','other']}
    for row in value['events']:
        fields=row.get('fields',{})
        message=fields.get('message')
        if message in messages:counts[messages[message]]+=1
        if message=='Current native packet authority' and fields.get('problem'):
            code=fields['problem'];problems[code if code in problems else 'other']+=1
    return dict(scope='Bounded private synthetic-lane trace counts only; instrumentation can affect timing.',
        captured_events=len(value['events']),omitted_events=value['omitted_events'],
        suppressed_known_successes=value['suppressed_known_successes'],counts=counts,authority_problems=problems)


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
        '-p', 'connect-server', '--no-default-features', '--features', 'test-support', '--test',
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
    unrequested_cli_artifacts = sum(row.get('reason') == 'compiler-artifact'
        and row.get('target', {}).get('name') == 'connect'
        and row['target'].get('kind') == ['bin'] for row in rows)
    artifacts = [row['executable'] for row in rows if row.get('reason') == 'compiler-artifact'
        and row['target']['name'] == 'gate6_canonical_webtransport' and row.get('executable')]
    unchanged = INPUTS() == before
    report = dict(scope='Native Linux x86_64 production runtime libraries at release opt3, no root package exceptions; integration test driver opt' + fixture_level + '. Build only.',
        fixture_opt_level=fixture_level, unrequested_cli_artifacts=unrequested_cli_artifacts,
        command=command, execution=result, runtime_library_profiles=runtime_profiles,
        source_count=len(before), sources_unchanged=unchanged,
        source_manifest_sha256=digest(output / 'source-inputs.json'),
        artifacts=[dict(path=value, sha256=digest(value)) for value in artifacts],
        passed=result['exit_code'] == 0 and not result['timed_out'] and not result['forced_kill'] and len(artifacts) == 1 and unchanged and runtime_profiles['passed'] and unrequested_cli_artifacts == 0)
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


def runtime(component=None):
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
        Path(shutil.which('pulseaudio')).resolve(strict=True), Path(shutil.which('tc')).resolve(strict=True),
        Path('/usr/local/share/connect-browser-packages.tsv')]}
    for folder in [module.parent, module.parent.parent / 'playwright-core']:
        for path in folder.rglob('*'):
            if path.is_file() and path.suffix in ('.js', '.mjs', '.json'):
                tools[str(path)] = digest(path)
    save(output / 'tool-inputs.json', tools)
    env = dict(os.environ, CONNECT_GATE6_NODE=str(node), CONNECT_GATE6_ICE_DIAGNOSTICS='1',
        RUST_LOG='connect_service::canonical_webrtc=debug')
    failure_diagnostic = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['rtc_operation_failure_diagnostic']
    worker_diagnostic = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['rtc_ice_worker_failure_diagnostic']
    admission_diagnostic = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['rtc_admission_observations']
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
        if component:
            cases=[case for case in cases if case[0]==component]
        for name, case, ignored, count in cases:
            directory = output / name
            directory.mkdir()
            command = [str(binary), case] + (['--exact', '--ignored'] if ignored else []) + ['--test-threads=1']
            cpu_before=cpu_snapshot()
            case_env=dict(env, CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS=str(directory))
            if component=='baseline':
                case_env['CONNECT_GATE6_CANONICAL_AUDIO_TRACE']=str(directory)
            step = BOUNDED(command, case_env,
                output / (name + '.log'), 120)
            step['cpu']=cpu_measurement(cpu_before,cpu_snapshot(),step['seconds'])
            if (directory/'core-audio-timeline.json').is_file():
                step['pipeline_diagnostic']=pipeline_diagnostic(directory/'core-audio-timeline.json')
            raw = (output / (name + '.log')).read_text(errors='replace')
            step['rtc_admission_observations'] = admission_diagnostic(raw) if step['exit_code'] else []
            step.update(name=name, passed=step['exit_code'] == 0 and not step['timed_out'] and not step['forced_kill'] and re.findall(
                r'^test result: ok\. (\d+) passed; 0 failed;', raw, re.M) == [str(count)])
            step['rtc_operation_failures'] = [failure_diagnostic(json.loads(line.split(' ', 1)[1])) for line in raw.splitlines()
                if line.startswith('rtc-operation-failure ')]
            assert len(step['rtc_operation_failures']) <= 16
            step['rtc_ice_worker_failures'] = [worker_diagnostic(json.loads(line.split(' ', 1)[1])) for line in raw.splitlines()
                if line.startswith('rtc-ice-worker-failure ')]
            assert len(step['rtc_ice_worker_failures']) <= 16
            for filename in ['browser-result.jsonl', 'browser-profile.json', 'ownership.json', 'ice-restart-oracle.json',
                             'unreachable-relay-oracle.json', 'unreachable-turn.json', 'media-diagnostics.json']:
                path = directory / filename
                if path.is_file() and path.stat().st_size:
                    value = json.loads(path.read_text())
                    if filename == 'browser-result.jsonl':
                        peer_states = browser_participation_states(value)
                        value = dict(status=value.get('status'), phase=value.get('phase'), error_class=value.get('error_class'), ice_restarts=value.get('ice_restarts'),
                            peer_states=peer_states,
                            unreachable_relay=value.get('unreachable_relay'), observations=[dict(
                                decoded_fps=o['decoded']/o['elapsed_seconds'], changing_fps=o['changing']/o['elapsed_seconds'],
                                rendered_fps=o['rendered']/o['elapsed_seconds'],
                                video_observer=o.get('video_observer'),native_decoded_observer=o.get('native_decoded_observer'),
                                render_callback_changing_fps=o['render_callback_changing']/o['elapsed_seconds']
                                    if isinstance(o.get('render_callback_changing'),(int,float)) else None,
                                presentation_counter_fps=o['presentation_counter_frames']/o['elapsed_seconds']
                                    if isinstance(o.get('presentation_counter_frames'),(int,float)) else None,
                                motion_sample_pixels=o.get('motion_sample_pixels'),identity_sample_pixels=o.get('identity_sample_pixels'),
                                source_draw_fps=o.get('source_observation',{}).get('draws_per_second'),
                                encoded_fps=o.get('outbound_observation',{}).get('frames')/o['elapsed_seconds']
                                    if isinstance(o.get('outbound_observation',{}).get('frames'),(int,float)) else None,
                                packets=o.get('packets'),lost=o.get('lost'),jitter=o.get('jitter'),
                                audio_coverage_complete=o['audio_profile'].get('coverage_complete'),
                                audio_maximum_sampling_gap_seconds=o['audio_profile'].get('maximum_sampling_gap_seconds'),
                                audio_bad_samples=o['audio_profile'].get('bad_unexcepted_samples'),
                                audio_bad_sample_diagnostics=o['audio_profile'].get('bad_sample_diagnostics'),
                                rtp_feedback=o.get('rtp_feedback'),
                                worker_pipeline=o.get('worker_pipeline'),
                                audio=o['audio_profile']['status'], motion=o['motion_profile']['status'],
                                source_identity=o['source_identity']['status'], av_sync=o['av_sync']['status'],
                                av_acceptance=o['av_acceptance']) for o in value.get('observations', [])])
                    step[filename] = value
            steps.append(step)
            print(json.dumps(dict(name=name, passed=step['passed'])), flush=True)
        for algorithm in ['EdDSA', 'ES256']:
            name = 'impairment-' + algorithm.lower()
            if component and component!=name:
                continue
            command = ['python3', 'scripts/rehearse-gate6-browser-network.py', '--binary', str(binary),
                '--source-manifest', str(output / 'repository-inputs.json'), '--algorithm', algorithm,
                '--output', str(output / name)]
            cpu_before=cpu_snapshot()
            step = BOUNDED(command, env, output / (name + '.log'), 140)
            step['cpu']=cpu_measurement(cpu_before,cpu_snapshot(),step['seconds'])
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
            lane=component or 'full',full_matrix=component is None,
            browser=version, image_id=os.environ.get('CONNECT_GATE6_BROWSER_IMAGE_ID'),
            producer=producer, source_count=len(repository), tool_count=len(tools),
            sources_unchanged=not changed, changed_inputs=changed, steps=steps,
            passed=len(steps) == (1 if component else 8) and all(step['passed'] for step in steps) and not changed)
        if component:
            report['scope']='Single diagnostic component only: '+component+'. Native Linux x86 CFT156 with opt3 runtime libraries. No full-matrix, installation or acceptance qualification.'
        save(output / 'summary.json', report)
    assert report['passed'], 'One or more required browser media cases failed; independent results retained'


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('build', 'run'))
    parser.add_argument('--component',choices=('native-ice','baseline','ice-eddsa','ice-es256',
        'unreachable-eddsa','unreachable-es256','impairment-eddsa','impairment-es256'))
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'x86_64'
    assert re.search(r'^vendor_id\s*:', Path('/proc/cpuinfo').read_text(), re.M)
    os.chdir(ROOT)
    assert not args.component or args.mode=='run'
    build() if args.mode == 'build' else runtime(args.component)
