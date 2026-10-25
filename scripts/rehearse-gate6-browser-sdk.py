#!/usr/bin/env python3
"""Explicit local production-SDK/installed-Chrome control checkpoint, not media qualification."""
import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import re
import runpy
import shutil
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-webtransport.py'))
ResourceSampler = runpy.run_path(str(ROOT / 'scripts/lib/browser_media_resources.py'))['ResourceSampler']
digest = BASE['digest']
CASES = ['canonical_webtransport_browser::installed_chrome_production_sdk_signed_control_receipts_and_refusals_' + alg for alg in ('eddsa', 'es256')] + BASE['BROWSER']
MEDIA = ['canonical_webtransport_browser::installed_chrome_production_sdk_negotiated_duplex_opus_vp8_and_reliable_data_' + alg for alg in ('eddsa', 'es256')]
CAPTURE = ['canonical_webtransport_browser::installed_chrome_production_sdk_captured_duplex_opus_vp8_and_reliable_data_' + alg for alg in ('eddsa', 'es256')]

NATIVE = ['canonical_webtransport_browser::installed_chrome_production_sdk_' + capture + 'browser_native_opus_vp8_and_reliable_data_' + alg for capture in ('', 'captured_') for alg in ('eddsa', 'es256')]

def sources():
    result = BASE['sources']()
    paths = [ROOT / p for p in ('scripts/rehearse-gate6-browser-sdk.py', 'scripts/lib/browser_media_resources.py', 'contracts/connect-media-v2.md', 'docs/decisions/015-gate-6-operating-profile.md', 'docs/evidence/gate-6/browser-client-clean60-profile.json')]
    for folder, extensions in [('sdk/typescript/src', ('.ts',)), ('sdk/typescript/dist', ('.js', '.ts')), ('sdk/tests', ('.mjs', '.py'))]:
        paths.extend(p for p in (ROOT / folder).rglob('*') if p.is_file() and p.suffix in extensions)
    paths.extend(ROOT / 'sdk/typescript' / p for p in ('package.json', 'package-lock.json', 'tsconfig.json'))
    paths.extend((ROOT / 'crates/connect-server/tests/fixtures/gate6-webtransport-browser/native').glob('*.py'))
    for path in paths:
        result[str(path.relative_to(ROOT))] = digest(path)
    return dict(sorted(result.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target-dir', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--installed-browser', action='store_true', required=True)
    parser.add_argument('--media', action='store_true', help='Also run short synthetic Opus/VP8/reliable-data over actual SDK and server; capture/native/full operating qualification remains separate.')
    parser.add_argument('--capture', action='store_true', help='Also run actual supplied-stream capture/worklet and decoded audio/canvas rendering over WT, alongside the media cases. No personal devices are opened.')
    parser.add_argument('--native', action='store_true', help='Also run actual public CanonicalClient QUIC/browser WT duplex Opus/VP8/reliable data with independently decoded native received packets. Native live encode/playout/full operating qualification remains separate.')
    parser.add_argument('--clean60', action='store_true', help='Capture cases use the frozen60second clean profile with>=24decoded/rendered fps,>=42uniquely paired audio-graph/video markers, median<=80ms/p95<=120ms and separate owned-process observations.')
    parser.add_argument('--diagnostic60', action='store_true', help='Single capture method uses the same60second source/observer with bounded packet summaries; diagnostic only, never frozen clean qualification. Requires --packet-trace summary.')
    parser.add_argument('--case', choices=CASES + MEDIA + CAPTURE + NATIVE, help='Explicit single-method diagnostic scope; does not qualify the omitted matrix.')
    parser.add_argument('--packet-trace', nargs='?', const='all', choices=('all', 'policy', 'audio', 'summary'), help='Record scoped all/policy/audio metadata or bounded summary counters under private raw logs; diagnostic overhead is not performance qualification. The default all scope includes SQL custody; summary avoids per-packet log IO.')
    args = parser.parse_args()
    if args.diagnostic60 and (args.clean60 or args.native or args.case not in CAPTURE or args.packet_trace != 'summary'):
        parser.error('Select one capture method with --packet-trace summary and no clean60/native qualification flags')
    if args.clean60 and (args.native or args.packet_trace or args.case and args.case not in CAPTURE):
        parser.error('Select clean60 capture methods without diagnostic packet tracing')
    args.capture = args.capture or args.clean60 or args.diagnostic60
    args.native = args.native or args.case in NATIVE
    selected = [args.case] if args.case else CASES + (MEDIA if args.media or args.capture else []) + (CAPTURE if args.capture else []) + (NATIVE if args.native else [])
    directory = args.output.resolve()
    if not directory.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Keep private evidence under ignored .local/')
    directory.mkdir(parents=True, exist_ok=False)
    (directory / 'tmp').mkdir()
    env = dict(os.environ, CARGO_INCREMENTAL='0', TMPDIR=str(directory / 'tmp'), CONNECT_GATE6_BROWSER_ARTIFACTS=str(directory / 'browser-artifacts'))
    for flag in ('CONNECT_GATE6_PACKET_TRACE', 'CONNECT_GATE6_BROWSER_CLEAN60', 'CONNECT_GATE6_NATIVE_FIXTURES'):
        env.pop(flag, None)
    if args.packet_trace:
        env['CONNECT_GATE6_PACKET_TRACE'] = args.packet_trace
    if args.clean60 or args.diagnostic60:
        env['CONNECT_GATE6_BROWSER_CLEAN60'] = '1'
    report = dict(passed=False, started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), host=platform.platform(),
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  scope='Actual production TypeScript canonical control in owned Chrome. Exact selected methods are recorded; a single diagnostic method does not qualify the omitted matrix. Optional media selects short synthetic source over SDK/WT/Store/Core with identified Opus/VP8/data and durable teardown. Optional capture selects supplied-stream capture/worklet and decoded audio/canvas rendering. Explicit clean60 applies frozen fps/paired graph-canvas marker limits and resource observations only to capture methods. Packet tracing and diagnostic60 are diagnostic only and never qualify the frozen clean profile. Optional native cases use public CanonicalClient QUIC and independently decode actual native received packets. No native live encoder/realtime presenter/full60second-native/impairment/remote/provider/complete G6-050/acceptance/publication claim.', media_selected=args.media or args.capture, capture_selected=args.capture, native_selected=args.native, clean60_selected=args.clean60, diagnostic60_selected=args.diagnostic60, selected=selected, packet_trace=args.packet_trace, steps=[], cases=[])

    def step(name, command, json_output=False):
        start = time.monotonic()
        with (directory / (name + '.log')).open('w') as log:
            if json_output:
                with (directory / 'build.jsonl').open('w') as output:
                    code = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=log).returncode
            else:
                code = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        report['steps'].append(dict(name=name, command=command, exit_code=code, elapsed_seconds=time.monotonic() - start, log_sha256=digest(directory / (name + '.log'))))
        print(json.dumps(dict(step=name, exit_code=code)), flush=True)
        assert code == 0, name

    before = sources()
    report['source_inputs_sha256'] = before
    try:
        step('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
        step('rust-format', ['bash', 'scripts/format.sh', '--check'])
        step('sdk-suite', ['npm', 'test', '--prefix', 'sdk/typescript'])
        if args.native:
            env['CONNECT_GATE6_NATIVE_FIXTURES'] = str(directory / 'native-fixtures')
            step('native-codec-sources', ['python3', 'crates/connect-server/tests/fixtures/gate6-webtransport-browser/native/prepare.py', str(directory / 'native-fixtures')])
        step('node', ['node', '--version'])
        step('rustc', ['rustc', '--version', '--verbose'])
        step('build', ['cargo', 'test', '-p', 'connect-server', '--test', 'gate6_canonical_webtransport', '--no-run', '--release', '--locked', '--offline', '-j2', '--target-dir', str(args.target_dir.resolve()), '--message-format=json'], True)
        rows = [json.loads(line) for line in (directory / 'build.jsonl').read_text().splitlines()]
        artifacts = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('executable') and r['target']['name'] == 'gate6_canonical_webtransport' and r['profile']['test']]
        assert len(artifacts) == 1
        binary = Path(artifacts[0]['executable'])
        sealed = directory / 'qualified-browser-sdk'
        sha = digest(binary)
        shutil.copy2(binary, sealed)
        report['executable'] = dict(path=str(binary), sealed_path=str(sealed.relative_to(ROOT)), sha256=sha)
        env['CARGO_MANIFEST_DIR'] = artifacts[0]['manifest_path'].rsplit('/', 1)[0]
        available = subprocess.check_output([str(sealed), '--list'], env=env, text=True)
        assert all(case + ': test' in available for case in selected)
        for index, case in enumerate(selected):
            env['CONNECT_GATE6_BROWSER_ARTIFACTS'] = str(directory / 'browser-artifacts' / str(index))
            logpath = directory / f'case-{index}.log'
            start = time.monotonic()
            resources = None
            with logpath.open('w') as log:
                process = subprocess.Popen([str(sealed), case, '--exact', '--ignored', '--test-threads=1', '--nocapture'], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                sampler = ResourceSampler(process.pid) if (args.clean60 or args.diagnostic60) and case in CAPTURE else None
                try:
                    code = process.wait(timeout=120)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                    code = 124
                finally:
                    if sampler:
                        resource_path = directory / f'case-{index}-resources.json'
                        resources = dict(**sampler.finish(resource_path), log=str(resource_path.relative_to(ROOT)), log_sha256=digest(resource_path))
            logtext = logpath.read_text()
            passed = code == 0 and bool(re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', logtext))
            if args.clean60 and case in CAPTURE:
                witness = re.search(r'^browser_evidence (.+)$', logtext, re.M)
                result = json.loads(witness[1])['result'] if witness else {}
                alignment = result.get('alignment', [])
                passed = passed and result.get('clean60') is True and result.get('measured_ms', 0) >= 60000 and len(result.get('decoded_fps', [])) == 2 and all(fps >= 24 for fps in result['decoded_fps']) and len(alignment) == 2 and all(a['paired_markers'] >= 42 and a['median_abs_skew_ms'] <= 80 and a['p95_abs_skew_ms'] <= 120 for a in alignment) and resources['valid']
            row = dict(case=case, passed=passed, exit_code=code, elapsed_seconds=time.monotonic() - start, log=str(logpath.relative_to(ROOT)), log_sha256=digest(logpath))
            if resources:
                row['resources'] = resources
            report['cases'].append(row)
            print(json.dumps(row), flush=True)
        report['executable_unchanged'] = digest(binary) == digest(sealed) == sha
    except Exception as error:
        report['failure'] = str(error)
    finally:
        report['source_inputs_unchanged'] = sources() == before
        report['passed'] = len(report['cases']) == len(selected) and all(row['passed'] for row in report['cases']) and report['source_inputs_unchanged'] and report.get('executable_unchanged') is True and 'failure' not in report
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (directory / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(passed=report['passed'], summary=str(directory / 'summary.json'))), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
