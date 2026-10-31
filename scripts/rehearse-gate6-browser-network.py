#!/usr/bin/env python3
"""Owned Linux Docker loopback UDP netem; no host or external network changes.

Both directions across the exact eight server media ports get 3% random loss,
0–100ms delay and one observed three-second blackhole. Raw state stays in .local.
"""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
import signal
import socket
import time

ROOT = Path(__file__).resolve().parents[1]
NETEM = runpy.run_path(str(ROOT/'scripts/lib/owned_netem.py'))['OwnedNetem']
HELPERS = runpy.run_path(str(ROOT/'scripts/rehearse-gate6-turn-relay.py'))


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


async def run(args):
    assert platform.system()=='Linux' and platform.machine()=='x86_64'
    binary = args.binary.resolve(strict=True)
    manifest = args.source_manifest.resolve(strict=True)
    sources = json.loads(manifest.read_text())
    assert len(sources)>100 and all(not Path(name).is_absolute() and '..' not in Path(name).parts
        and re.fullmatch('[0-9a-f]{64}', value) for name,value in sources.items())
    def changed_sources():
        return [name for name,value in sources.items()
            if not (ROOT/name).is_file() or digest(ROOT/name)!=value]
    assert not changed_sources(), 'Current inputs differ from the original producer'
    output=args.output.resolve()
    assert output.is_relative_to(ROOT/'.local')
    output.mkdir(parents=True,mode=0o700,exist_ok=False)
    browser=output/'browser';browser.mkdir(mode=0o700)
    test='canonical_webrtc_browser::canonical_chrome_owned_udp_impairment_'+args.algorithm.lower()
    report=dict(scope=__doc__,passed=False,test=test,source_count=len(sources),
        kernel_release=platform.release(),
        source_manifest_sha256=digest(manifest),binary_sha256=digest(binary),runner_sha256=digest(__file__),
        profile='impairment',configured_random_drop_probability=.03,configured_delay_ms=[0,100],
        configured_burst_seconds=3,delay_distribution='kernel netem uniform50ms +/-50ms',
        random_method='kernel netem per-packet random loss; realized drops are reported, no fixed-seed claim')
    held=[];child=None;burst_task=None;netem=None;stage='network-setup'
    try:
        start,held=HELPERS['reserve_range'](8)
        fixture=output/'network-fixture.json'
        ready=browser/'window-ready.json';burst_report=browser/'network-bursts.json'
        HELPERS['private_write'](fixture,json.dumps(dict(port_start=start,port_end=start+7,
            window_ready=str(ready),burst_report=str(burst_report)))+'\n')
        env=dict(os.environ,CONNECT_GATE6_BROWSER_NETWORK_FIXTURE=str(fixture),
            CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS=str(browser))
        netem=NETEM(start,start+7)
        with netem:
            report['initial_counters']=netem.snapshot()
            for sock in held:sock.close()
            async def burst():
                until=time.monotonic()+35
                while not ready.exists():
                    if child.returncode is not None:return
                    assert time.monotonic()<until, 'Original measurement window never became ready'
                    await asyncio.sleep(.025)
                target=time.monotonic()+20
                while time.monotonic()<target:
                    if child.returncode is not None:return
                    await asyncio.sleep(min(.025,max(0,target-time.monotonic())))
                before=netem.snapshot()
                netem.loss(100)
                began_unix=time.time();began=time.monotonic()
                # Start after both loss commands acknowledged; end before either
                # restoration starts. This never grants extra recovery time.
                await asyncio.sleep(3)
                ended_unix=time.time();elapsed=time.monotonic()-began
                netem.loss(3)
                after=netem.snapshot()
                assert 3<=elapsed<=3.2
                interval=dict(start_unix_seconds=began_unix,end_unix_seconds=ended_unix,duration_seconds=elapsed)
                HELPERS['private_write'](burst_report,json.dumps([interval])+'\n')
                report['burst']=dict(interval=interval,before=before,after=after,
                    dropped={direction:after[direction]['drops']-before[direction]['drops']
                        for direction in netem.handles})
            command=[str(binary),test,'--exact','--ignored','--test-threads=1']
            stage='original-test'
            with (output/'test.log').open('xb') as log:
                child=await asyncio.create_subprocess_exec(*command,cwd=ROOT,env=env,
                    stdout=log,stderr=asyncio.subprocess.STDOUT,start_new_session=True)
                burst_task=asyncio.create_task(burst())
                try:
                    await asyncio.wait_for(asyncio.gather(child.wait(),burst_task),120)
                except asyncio.TimeoutError:
                    report['timed_out']=True
                report['exit_code']=child.returncode
            if child.returncode is None:
                os.killpg(child.pid,signal.SIGTERM)
                await asyncio.wait_for(child.wait(),5)
                report['cleanup_required_sigterm']=True
            report['original_process_joined']=True
            report['final_counters']=netem.snapshot()
        stage='network-retirement'
        report['owned_qdiscs_removed']=True
        reclaimed=[]
        try:
            for port in range(start,start+8):
                sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);reclaimed.append(sock)
                sock.bind(('127.0.0.1',port))
        finally:
            for sock in reclaimed:sock.close()
        report['original_media_addresses_reclaimed']=8
        stage='decoded-media-verification'
        raw=(output/'test.log').read_text(errors='replace')
        report['test_log_sha256']=digest(output/'test.log')
        report['test_results']=re.findall(r'^test result:.*$',raw,re.M)
        report['panic_locations']=[dict(file=name,line=int(line),column=int(column))
            for name,line,column in re.findall(r'panicked at (crates/[A-Za-z0-9_/.-]+\.rs):(\d+):(\d+)',raw)[:8]]
        result_path=browser/'browser-result.jsonl'
        diagnostics_path=browser/'media-diagnostics.json'
        if diagnostics_path.is_file():
            report['server_media_diagnostics']=json.loads(diagnostics_path.read_text())
        if result_path.is_file():
            value=json.loads(result_path.read_text())
            states=runpy.run_path(str(ROOT/'scripts/check-gate6-browser-media.py'))['browser_participation_states']
            report['browser']=dict(status=value.get('status'),phase=value.get('phase'),
                failure_code=value.get('failure_code'),error_class=value.get('error_class'),peer_states=states(value),
                owned_network_paths=value.get('owned_network_paths'),
                observations=[dict(decoded_fps=o['decoded']/o['elapsed_seconds'],
                    changing_fps=o['changing']/o['elapsed_seconds'],audio=o['audio_profile']['status'],
                    rendered_fps=o['rendered']/o['elapsed_seconds'],
                    video_observer=o.get('video_observer'),native_decoded_observer=o.get('native_decoded_observer'),
                    render_marker_observation=o.get('render_marker_observation'),
                    av_sync_marker_pairs=o.get('av_sync',{}).get('marker_pairs',[]),
                    render_callback_changing_fps=o['render_callback_changing']/o['elapsed_seconds']
                        if isinstance(o.get('render_callback_changing'),(int,float)) else None,
                    presentation_counter_fps=o['presentation_counter_frames']/o['elapsed_seconds']
                        if isinstance(o.get('presentation_counter_frames'),(int,float)) else None,
                    audio_coverage_complete=o['audio_profile'].get('coverage_complete'),
                    audio_maximum_sampling_gap_seconds=o['audio_profile'].get('maximum_sampling_gap_seconds'),
                    audio_bad_unexcepted_samples=o['audio_profile'].get('bad_unexcepted_samples'),
                    audio_bad_sample_diagnostics=o['audio_profile'].get('bad_sample_diagnostics'),
                    rtp_feedback=o.get('rtp_feedback'),
                    maximum_unexcluded_motion_gap_seconds=o['motion_profile'].get('maximum_unexcluded_freshness_gap_seconds'),
                    motion=o['motion_profile']['status'],source_identity=o['source_identity']['status'],
                    recovery=o['recovery'],av_acceptance=o['av_acceptance']) for o in value.get('observations',[])])
        assert report['exit_code']==0 and not report.get('timed_out') and not report.get('cleanup_required_sigterm')
        assert len(report['test_results'])==1 and '1 passed; 0 failed; 0 ignored;' in report['test_results'][0]
        assert report.get('burst') and all(report['burst']['dropped'][d]>0
            and report['final_counters'][d]['packets']>0
            and report['final_counters'][d]['drops']>report['burst']['dropped'][d] for d in netem.handles)
        report['passed']=True
    except Exception as error:
        text=str(getattr(error,'stderr',''))
        report['failure']=dict(stage=stage,error_class=type(error).__name__,
            network_error=next((name for name,pattern in [
                ('capability','Operation not permitted'),('loopback-device','Cannot find device'),
                ('netem-unavailable','Specified qdisc kind is unknown'),
                ('distribution-unavailable','No distribution data')] if pattern in text),None))
        raise
    finally:
        if burst_task and not burst_task.done():
            burst_task.cancel();await asyncio.gather(burst_task,return_exceptions=True)
        if child is not None and child.returncode is None:
            report['cleanup_required_sigterm']=True
            os.killpg(child.pid,signal.SIGTERM)
            try:await asyncio.wait_for(child.wait(),5)
            except asyncio.TimeoutError:
                report['cleanup_required_sigkill']=True
                os.killpg(child.pid,signal.SIGKILL);await child.wait()
        for sock in held:sock.close()
        if netem is not None:
            try:
                netem.close();report['owned_qdiscs_removed']=True
            except Exception as error:
                report['netem_cleanup_failure']=type(error).__name__
                report['passed']=False
        report['changed_source_inputs']=changed_sources()
        report['passed']=report['passed'] and not report['changed_source_inputs']
        HELPERS['private_write'](output/'summary.json',json.dumps(report,indent=2)+'\n')
        print(json.dumps(report),flush=True)
    assert report['passed']


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--source-manifest',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--algorithm',choices=('EdDSA','ES256'),required=True)
    asyncio.run(run(parser.parse_args()))
