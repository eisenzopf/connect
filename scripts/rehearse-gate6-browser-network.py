#!/usr/bin/env python3
"""Owned loopback port-preserving NAT, browser UDP loss/jitter and burst recovery.

Uses the existing impairment observer/policy. No clean-profile, distinct-network,
provider or account qualification. All raw state stays in a fresh .local folder.
"""
import argparse
import asyncio
from contextlib import AsyncExitStack
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import signal
import socket
import time

ROOT = Path(__file__).resolve().parents[1]
PROXY = runpy.run_path(str(ROOT / "scripts/video/native-processes.py"))["DatagramImpairment"]
TURN_HELPERS = runpy.run_path(str(ROOT / "scripts/rehearse-gate6-turn-relay.py"))


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


async def run(args):
    binary = args.binary.resolve(strict=True)
    manifest = args.source_manifest.resolve(strict=True)
    sources = json.loads(manifest.read_text())
    assert len(sources) > 100 and all(not Path(name).is_absolute() and ".." not in Path(name).parts
        and re.fullmatch("[0-9a-f]{64}", value) for name, value in sources.items())
    def changed_sources():
        return [name for name, value in sources.items()
                if not (ROOT / name).is_file() or digest(ROOT / name) != value]
    assert not changed_sources(), "Current inputs differ from the original producer"
    output = args.output.resolve()
    assert output.is_relative_to(ROOT / ".local")
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    browser = output / "browser"
    browser.mkdir(mode=0o700)
    test = "canonical_webrtc_browser::canonical_chrome_owned_udp_impairment_" + args.algorithm.lower()
    report = dict(scope=__doc__, passed=False, test=test, source_count=len(sources),
        source_manifest_sha256=digest(manifest), binary_sha256=digest(binary),
        runner_sha256=digest(__file__), profile="impairment", configured_random_drop_probability=.03,
        configured_delay_ms=[0, 100], configured_burst_seconds=3, proxies=[])
    held = []
    child = None
    try:
        start, held = TURN_HELPERS["reserve_range"](8)
        async with AsyncExitStack() as owners:
            proxies = [await owners.enter_async_context(PROXY(("127.0.0.1", port),
                seed=511 + index, bursts=((20, 3),), listen=("127.0.0.2", port)))
                for index, port in enumerate(range(start, start + 8))]
            # One actual impairment interval for all allocated RTP/RTCP ports.
            began, began_unix = time.monotonic(), time.time()
            for proxy in proxies:
                proxy.start, proxy.start_unix = began, began_unix
            fixture = output / "network-fixture.json"
            TURN_HELPERS["private_write"](fixture, json.dumps(dict(port_start=start, port_end=start + 7,
                window_ready=str(browser / "window-ready.json"), bursts=[dict(
                    start_unix_seconds=began_unix + 20, end_unix_seconds=began_unix + 23,
                    duration_seconds=3.0)])) + "\n")
            env = dict(os.environ, CONNECT_GATE6_BROWSER_NETWORK_FIXTURE=str(fixture),
                CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS=str(browser))
            for sock in held:
                sock.close()
            command = [str(binary), test, "--exact", "--ignored", "--test-threads=1"]
            with (output / "test.log").open("xb") as log:
                child = await asyncio.create_subprocess_exec(*command, cwd=ROOT, env=env,
                    stdout=log, stderr=asyncio.subprocess.STDOUT, start_new_session=True)
                try:
                    await asyncio.wait_for(child.wait(), 120)
                except asyncio.TimeoutError:
                    report["timed_out"] = True
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        await asyncio.wait_for(child.wait(), 5)
                    except asyncio.TimeoutError:
                        report["forced_kill"] = True
                        os.killpg(child.pid, signal.SIGKILL)
                        await child.wait()
                report["exit_code"] = child.returncode
            # Capture original sockets before retiring their owned transports.
            addresses = [proxy.address for proxy in proxies]
            addresses += [backend.get_extra_info("sockname") for proxy in proxies for backend in proxy.backends.values()]
            report["proxies"] = [proxy.report() for proxy in proxies]
        await asyncio.sleep(.05)
        reclaimed = []
        try:
            for address in addresses:
                sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                reclaimed.append(sock)
                sock.bind(address)
        finally:
            for sock in reclaimed:
                sock.close()
        report["original_proxy_addresses_reclaimed"] = len(addresses)
        report["owned_proxies_closed"] = len(proxies)
        raw = (output / "test.log").read_text(errors="replace")
        report["test_log_sha256"] = digest(output / "test.log")
        report["test_results"] = re.findall(r"^test result:.*$", raw, re.M)
        report['panic_locations']=[dict(file=name,line=int(line),column=int(column))
            for name,line,column in re.findall(r'panicked at (crates/[A-Za-z0-9_/.-]+\.rs):(\d+):(\d+)',raw)[:8]]
        result_path=browser/'browser-result.jsonl'
        if result_path.is_file():
            value=json.loads(result_path.read_text())
            states=runpy.run_path(str(ROOT/'scripts/check-gate6-browser-media.py'))['browser_participation_states']
            report['browser_failure']=dict(status=value.get('status'),phase=value.get('phase'),
                failure_code=value.get('failure_code'),error_class=value.get('error_class'),peer_states=states(value))
        active = [proxy for proxy in report["proxies"]
                  if all(proxy["directions"][direction]["forwarded"] > 0 for direction in ("to_server", "to_client"))]
        report["active_bidirectional_proxies"] = len(active)
        assert len(active) >= 2, "Both browser peers must traverse the owned proxy pool"
        for direction in ("to_server", "to_client"):
            assert sum(p["directions"][direction].get("random_dropped", 0) for p in active) > 0
            assert sum(p["directions"][direction].get("stall_dropped", 0) for p in active) > 0
        assert all(not p["errors"] for p in report["proxies"])
        assert report["exit_code"] == 0 and not report.get("timed_out") and not report.get("forced_kill")
        assert len(report["test_results"]) == 1 and "1 passed; 0 failed; 0 ignored;" in report["test_results"][0]
        report["passed"] = True
    finally:
        for sock in held:
            sock.close()
        if child is not None and child.returncode is None:
            report["cleanup_required_sigterm"] = True
            os.killpg(child.pid, signal.SIGTERM)
            try:
                await asyncio.wait_for(child.wait(), 5)
            except asyncio.TimeoutError:
                report["cleanup_required_sigkill"] = True
                os.killpg(child.pid, signal.SIGKILL)
                await child.wait()
        report["changed_source_inputs"] = changed_sources()
        report["passed"] = report["passed"] and not report["changed_source_inputs"]
        TURN_HELPERS["private_write"](output / "summary.json", json.dumps(report, indent=2) + "\n")
        print(json.dumps(report), flush=True)
    assert report["passed"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--algorithm", choices=("EdDSA", "ES256"), required=True)
    asyncio.run(run(parser.parse_args()))
