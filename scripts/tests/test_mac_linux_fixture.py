"""Private plumbing tests only; no VM, transport or browser qualification."""
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import tomllib
from types import SimpleNamespace
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "lib"))
from lib.mac_linux_fixture import MacLinuxFixture, archive_files, programme_source_files, rewrite_paths, unpack_verified
from lib.vm_workload_lease import VmWorkloadLease, cleanup_evidence
import mac_linux_guest

class MacLinuxPlumbing(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_projection_preserves_original_ids_credentials_urls_and_input(self):
        source = {"operation_id":"original-move", "credential":"synthetic", "endpoint":"https://192.168.1.2:443", "config":"/host/root/.local/actor.toml", "other":"/host/rooted/value", "list":["/host/root/.local/seed"]}
        value = rewrite_paths(source,Path("/host/root"),Path("/guest/root"))
        self.assertEqual(value["config"],"/guest/root/.local/actor.toml")
        self.assertEqual(value["list"],["/guest/root/.local/seed"])
        for key in ("operation_id","credential","endpoint","other"):
            self.assertEqual(value[key],source[key])
        self.assertEqual(source["config"],"/host/root/.local/actor.toml")

    def test_programme_transfer_keeps_its_exact_relative_contract_and_sdk(self):
        contract = "contracts/gate6-transport-matrix.json"
        module = "scripts/lib/transport_movement_profile.mjs"
        payloads = {contract: b'{"workload":{"movement":{"original":"pinned"}}}\n',
                    module: b'const original = new URL("../../contracts/gate6-transport-matrix.json", import.meta.url);\n',
                    "sdk/typescript/dist/index.js": b'export const original = "sdk";\n',
                    "sdk/typescript/package.json": b'{"type":"module"}\n'}
        for name, body in payloads.items():
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(body)
        tracked = (module + "\0" + contract + "\0").encode()
        with patch("lib.mac_linux_fixture.subprocess.check_output", return_value=tracked) as tracked_call:
            files = programme_source_files(self.root)
        tracked_call.assert_called_once_with(["git", "ls-files", "-z", "scripts", contract], cwd=self.root)
        self.assertEqual(set(files), set(payloads))
        with tarfile.open(fileobj=io.BytesIO(archive_files(files)), mode="r:") as archive:
            for name, body in payloads.items():
                self.assertEqual(archive.extractfile(name).read(), body)
                self.assertEqual((self.root / name).read_bytes(), body)
        # A missing contract cannot be substituted from /workspace or another
        # run. A symlink is not an ordinary sealed source dependency either.
        for tracked_value in [(module + "\0").encode(), tracked]:
            if tracked_value == tracked:
                (self.root / contract).unlink()
                outside = self.root / "another-contract.json"
                outside.write_bytes(payloads[contract])
                (self.root / contract).symlink_to(outside)
            with patch("lib.mac_linux_fixture.subprocess.check_output", return_value=tracked_value), \
                    self.assertRaisesRegex(ValueError, "Original native programme contract required"):
                programme_source_files(self.root)

    def test_verified_results_require_exact_manifest_before_writing(self):
        body=b'{"operation_id":"original"}'
        manifest={"observation.json":hashlib.sha256(body).hexdigest()}
        raw=archive_files({"observation.json":(body,False),"TRANSFER-MANIFEST.json":(json.dumps(manifest).encode(),False)})
        self.assertEqual(unpack_verified(raw,self.root/"result"),manifest)
        self.assertEqual((self.root/"result/observation.json").read_bytes(),body)
        bad=archive_files({"observation.json":(b"changed",False),"TRANSFER-MANIFEST.json":(json.dumps(manifest).encode(),False)})
        with self.assertRaisesRegex(ValueError,"digest mismatch"):
            unpack_verified(bad,self.root/"bad")
        self.assertFalse((self.root/"bad").exists())

    def test_archive_rejects_escape_and_symlink_members(self):
        with self.assertRaises(ValueError):archive_files({"../escape":(b"x",False)})
        raw=io.BytesIO()
        with tarfile.open(fileobj=raw,mode="w:") as archive:
            value=tarfile.TarInfo("observed");value.type,value.linkname=tarfile.SYMTYPE,"/elsewhere";archive.addfile(value)
        with self.assertRaises(ValueError):unpack_verified(raw.getvalue(),self.root/"bad")

    def test_config_stages_current_control_assets_without_mutating_issuer(self):
        fixture=MacLinuxFixture(self.root,self.root/"owner",{})
        actor=self.root/".local/actor";actor.mkdir(parents=True)
        for name in ("trust.der","key.der","server.pub"):(actor/name).write_bytes(name.encode())
        source=actor/"sdk.toml"
        source.write_text('[control]\ncredential="synthetic"\ndevice_id="device-original"\ntrust_certificate="trust.der"\nsigning_private_key="key.der"\nserver_public_key="server.pub"\n[application]\ncredential="admin-local-only"\n')
        before=source.read_bytes();staged={};fixture.stage=staged.update
        self.assertEqual(fixture.stage_native_config(source),fixture.remote_root/".local/actor/sdk.toml")
        value=tomllib.loads(staged[".local/actor/sdk.toml"][0].decode())
        self.assertNotIn("application",value);self.assertEqual(value["control"]["device_id"],"device-original")
        self.assertEqual(set(staged),{".local/actor/"+name for name in ("sdk.toml","trust.der","key.der","server.pub")})
        self.assertEqual(source.read_bytes(),before)

    def test_clock_translation_retains_raw_domains_and_rejects_stale(self):
        fixture=MacLinuxFixture(self.root,self.root/"owner",{})
        fixture.alignments=[dict(host_before=1000,host_after=1000.2,guest_monotonic_seconds=500)]
        self.assertAlmostEqual(fixture.host_time(499.5),999.6)
        for when in (501,469,float("nan")):
            with self.assertRaises(ValueError):fixture.host_time(when)

    def test_native_sources_create_mapped_parent_without_reusing_leaf_or_rewriting_manifest(self):
        fixture=MacLinuxFixture(self.root,self.root/"owner",{})
        fixture.remote_root=self.root/"guest";fixture.remote_root.mkdir(mode=0o700)
        output=self.root/".local/matrix/cells/macos-arm64/remote/native-quic/EdDSA/private/media/av-sources"
        target=fixture.mapped(output)
        self.assertFalse(target.parent.exists())
        manifest=b'{\n  "format":"synthetic-source-custody", "sources": []\n}\n'
        preparation=[];owned=[]
        def prepare(path):
            preparation.append(path)
            self.assertEqual(path,target)
            self.assertTrue(path.parent.is_dir(),"Guest artifact parents must exist before source preparation")
            path.mkdir(mode=0o700)
            (path/"manifest.json").write_bytes(manifest)
            return json.loads(manifest)
        def call(command,**kwargs):
            if command[:2]==["python3","-c"]:
                return json.dumps({"total":20*1024**3,"used":1024**3,"free":19*1024**3}).encode()
            self.assertEqual(command,["cat",target/"manifest.json"])
            return (target/"manifest.json").read_bytes()
        def owned_call(command,*,timeout):
            self.assertEqual(command[:2],["python3","-c"]);self.assertEqual(timeout,1200)
            self.assertEqual(command[3:],[fixture.remote_root,target]);owned.append(command)
            # Execute only the generated preparation glue with an in-memory
            # codec producer. No VM, subprocess, encoder or source workload.
            with patch.dict(sys.modules,{"lib.transport_av":SimpleNamespace(prepare=prepare)}), \
                    patch.object(sys,"argv",["-c",*map(str,command[3:])]), \
                    patch.object(sys,"path",list(sys.path)),patch.object(sys,"stdout",io.StringIO()) as out:
                exec(command[2],{"__name__":"__main__"})
                return out.getvalue().encode()
        fixture.call=call;fixture.owned_call=owned_call
        self.assertEqual(fixture.prepare_sources(output),json.loads(manifest))
        self.assertEqual(preparation,[target]);self.assertEqual(len(owned),1)
        self.assertEqual((output/"manifest.json").read_bytes(),manifest)
        self.assertEqual((target/"manifest.json").read_bytes(),manifest)
        self.assertEqual(target.stat().st_mode&0o777,0o700)
        # A second preparation must fail before replacing either original
        # manifest; creating parents must not make the source leaf reusable.
        with self.assertRaises(FileExistsError):
            fixture.prepare_sources(output)
        self.assertEqual((output/"manifest.json").read_bytes(),manifest)
        self.assertEqual((target/"manifest.json").read_bytes(),manifest)

    def test_revoke_bridge_preserves_original_request_and_refuses_response_change(self):
        output=self.root/"probe";output.mkdir();request={"kind":"revoke","operation_id":"original-revoke"}
        (output/"revoke-request.json").write_text(json.dumps(request))
        inbound={"directory":"probe","responses":{"revoke-response.json":{"revoked":True}}}
        with patch.object(sys,"stdin",io.StringIO(json.dumps(inbound))),patch.object(sys,"stdout",io.StringIO()) as out:mac_linux_guest.exchange(self.root)
        self.assertEqual(json.loads(out.getvalue())["requests"]["revoke-request.json"],request)
        inbound["responses"]["revoke-response.json"]["revoked"]=False
        with patch.object(sys,"stdin",io.StringIO(json.dumps(inbound))),self.assertRaisesRegex(ValueError,"cannot be changed"):mac_linux_guest.exchange(self.root)

    def nested_resource_fixture(self):
        host = self.root / "bridge-host"
        guest = self.root / "bridge-guest"
        host.mkdir()
        guest.mkdir()
        output = guest / "browser"
        output.mkdir()
        fixture = MacLinuxFixture(host, host / "owner", {}, resource_cycles=3)
        fixture.output = host / "browser"
        fixture.output.mkdir()
        fixture.output_relative = "browser"
        fixture.remote_root = guest
        def exchange(command, *, data, timeout):
            self.assertEqual(command[-2:], ["exchange", guest])
            self.assertEqual(timeout, 5)
            with patch.object(sys, "stdin", io.StringIO(data.decode())), patch.object(sys, "stdout", io.StringIO()) as out:
                mac_linux_guest.exchange(guest)
                return out.getvalue().encode()
        fixture.call = exchange
        return fixture, guest, output

    def test_nested_resource_bridge_round_trips_three_cycles_and_preserves_original_requests(self):
        fixture, guest, output = self.nested_resource_fixture()
        for cycle in (1, 2, 3):
            for phase in ("baseline", "terminal", "retired"):
                relative = Path("cycles") / ("cycle-" + str(cycle)) / ("resources-" + phase + ".json")
                request = output / relative
                request.parent.mkdir(parents=True, exist_ok=True)
                original = {"phase": phase, "monotonic_seconds": 10.25 * cycle}
                request.write_text(json.dumps(original))
                observed = fixture.poll()
                self.assertEqual(observed["requests"][relative.as_posix()], original)
                self.assertEqual(json.loads((fixture.output / relative).read_text()), original)
                response = relative.with_name("resources-" + phase + "-response.json")
                result = {"passed": False, "original_cycle": cycle}
                (fixture.output / response).write_text(json.dumps(result))
                fixture.poll()
                self.assertEqual(json.loads((output / response).read_text()), result)
                self.assertEqual(json.loads(request.read_text()), original)
        changed = fixture.output / "cycles/cycle-1/resources-baseline-response.json"
        changed.write_text(json.dumps({"passed": True}))
        with self.assertRaisesRegex(ValueError, "cannot be changed"):
            fixture.poll()

    def test_nested_resource_bridge_rejects_symlink_bounds_and_unowned_paths(self):
        fixture, guest, output = self.nested_resource_fixture()
        nested = output / "cycles/cycle-1"
        nested.mkdir(parents=True)
        request = nested / "resources-baseline.json"
        request.write_text("x" * 4097)
        with self.assertRaisesRegex(ValueError, "Bounded original request"):
            fixture.poll()
        request.unlink()
        nested.rmdir()
        outside = self.root / "other"
        outside.mkdir()
        nested.symlink_to(outside, target_is_directory=True)
        (outside / "resources-baseline.json").write_text('{}')
        with self.assertRaisesRegex(ValueError, "Bounded original request"):
            fixture.poll()
        nested.unlink()
        payload = {"directory": "browser", "responses": {"cycles/cycle-4/resources-baseline-response.json": {"passed": True}}}
        with patch.object(sys, "stdin", io.StringIO(json.dumps(payload))), self.assertRaisesRegex(ValueError, "Bounded original response"):
            mac_linux_guest.exchange(guest)
        self.assertFalse((output / "cycles/cycle-4").exists())
        fixture.call = lambda *args, **kwargs: json.dumps({"monotonic_seconds": 0, "requests": {"../escape": {}}, "state": None}).encode()
        with self.assertRaisesRegex(ValueError, "Bounded original cross-host request"):
            fixture.poll()
        self.assertFalse((fixture.output.parent / "escape").exists())

    def test_actual_browser_resource_bridge_completes_all_nine_remote_handshakes(self):
        import subprocess
        import time
        fixture, guest, output = self.nested_resource_fixture()
        bridge = Path(__file__).resolve().parents[1] / "lib/matrix_resource_bridge.mjs"
        script = """
import {pathToFileURL} from 'node:url';
const {matrixResourceBridge} = await import(pathToFileURL(process.argv[1]));
const observe = matrixResourceBridge(process.argv[2], {cycles: 3});
for (let cycle = 1; cycle <= 3; cycle++) {
    for (const phase of ['baseline', 'terminal', 'retired']) {
        const result = await observe(phase, Number(process.hrtime.bigint()) / 1e9, cycle);
        if (result.passed !== false) throw Error('Original negative verdict changed');
    }
}
console.log(JSON.stringify({handshakes: 9}));
"""
        child = subprocess.Popen(["node", "--input-type=module", "-e", script, str(bridge), str(output)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        served = set()
        cutoff = time.monotonic() + 5
        try:
            while child.poll() is None and time.monotonic() < cutoff:
                fixture.poll()
                for request in fixture.output.glob("cycles/cycle-[123]/resources-*.json"):
                    if request.name.endswith("-response.json"):
                        continue
                    relative = request.relative_to(fixture.output)
                    if relative in served:
                        continue
                    value = json.loads(request.read_text())
                    self.assertIn(value["phase"], ("baseline", "terminal", "retired"))
                    response = request.with_name(request.stem + "-response.json")
                    response.write_text(json.dumps({"passed": False}))
                    served.add(relative)
                fixture.poll()
                time.sleep(.005)
            self.assertIsNotNone(child.poll(), "Actual browser helper did not receive the original nested responses")
            stdout, stderr = child.communicate(timeout=1)
            self.assertEqual(child.returncode, 0, stderr)
            self.assertEqual(json.loads(stdout), {"handshakes": 9})
            self.assertEqual(len(served), 9)
        finally:
            if child.poll() is None:
                child.terminate()
            child.communicate(timeout=2)

    def test_three_cycle_preflight_scales_only_original_queue_and_host_wait(self):
        from lib import mac_linux_fixture as module
        for cycles in [1, 3]:
            with self.subTest(cycles=cycles):
                fixture = MacLinuxFixture(self.root, self.root / ("lease-" + str(cycles)), {}, resource_cycles=cycles)
                lease = SimpleNamespace(pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(b"original-vm-lease-acquired\n"), stderr=io.BytesIO(), poll=lambda: None)
                class Select:
                    def __enter__(self):return self
                    def __exit__(self, *args):pass
                    def register(self, *args):pass
                    def select(self, timeout):waits.append(timeout);return [True]
                waits = []
                with patch.object(module.subprocess, "Popen", return_value=lease) as opened, patch.object(module.selectors, "DefaultSelector", Select):
                    fixture.acquire_lease()
                args = opened.call_args.args[0]
                self.assertEqual(args[-1], str(1200 * cycles));self.assertEqual(waits, [1200 * cycles + 10])
                self.assertIn("VmWorkloadLease(timeout=int(sys.argv[2]))", args[-3])
                self.assertTrue(fixture.lease_attempt["acquired"])
        for value in [0, 2, 4, True, "3"]:
            with self.assertRaises(ValueError):MacLinuxFixture(self.root, self.root / "invalid", {}, resource_cycles=value)

    def test_failed_pre_acquire_queue_records_only_original_ssh_join_and_no_guest_launch(self):
        from lib import mac_linux_fixture as module
        fixture = MacLinuxFixture(self.root, self.root / "queue-failure", {}, resource_cycles=3)
        stderr = SimpleNamespace(fileno=lambda: 42, read=lambda limit: b"TimeoutError: Existing ARM VM workload lease deadline\n", close=lambda: None)
        lease = SimpleNamespace(pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=stderr, poll=lambda: 1, wait=lambda timeout: 1)
        class Select:
            def __enter__(self):return self
            def __exit__(self, *args):pass
            def register(self, *args):pass
            def select(self, timeout):return [True]
        with patch.object(module.subprocess, "Popen", return_value=lease), patch.object(module.selectors, "DefaultSelector", Select), patch.object(module.os, "set_blocking"), patch.object(module.os, "killpg", side_effect=ProcessLookupError):
            with self.assertRaisesRegex(ValueError, "lease unavailable"):fixture.acquire_lease()
            fixture.call = lambda *args, **kwargs: self.fail("No private guest source or workload exists before acquisition")
            result = fixture.close()
        self.assertFalse(result["passed"]);self.assertFalse(result["original_joined"])
        self.assertTrue(result["preflight_only"]);self.assertTrue(result["original_lease_ssh_joined"])
        for name in ["guest_job_started", "guest_sources_staged", "guest_namespace_started"]:self.assertFalse(result[name])
        self.assertEqual(result["lease_acquisition"]["original_ssh_exit_code"], 1)
        self.assertIn("TimeoutError", (fixture.directory / "lease-acquisition.stderr").read_text())
        self.assertEqual(fixture.inputs, {});self.assertIsNone(fixture.owner)

    def test_unjoined_original_queue_process_never_becomes_guest_cleanup_success(self):
        from lib import mac_linux_fixture as module
        fixture = MacLinuxFixture(self.root, self.root / "queue-unjoined", {}, resource_cycles=3)
        fixture.lease_attempt = dict(acquired=False, started_unix=1, started_monotonic_seconds=1)
        fixture.lease = SimpleNamespace(pid=123, poll=lambda: None,
            stderr=SimpleNamespace(fileno=lambda: 42, read=lambda limit: b""))
        with patch.object(module.os, "set_blocking"):
            result = fixture.close()
        self.assertFalse(result["passed"]);self.assertFalse(result["original_joined"])
        self.assertFalse(result["original_lease_ssh_joined"])
        self.assertEqual(result["cleanup_failures"], ["original-lease-ssh-join-unconfirmed"])

    def transfer_fixture(self, cycles):
        host = self.root / ("host-" + str(cycles));host.mkdir()
        guest = self.root / ("guest-" + str(cycles));guest.mkdir()
        output_relative = "output"
        remote_output = guest / output_relative;remote_output.mkdir()
        (remote_output / "packet-capture").mkdir()
        (remote_output / "packet-capture/media.pcap").write_bytes(b"x" * 40000)
        (remote_output / "guest-owner.json").write_text(json.dumps({"original_joined": True, "forced": False}))
        fixture = MacLinuxFixture(host, host / "bridge", {})
        fixture.run_id = guest.name;fixture.remote_root = guest;fixture.revision = "synthetic-source"
        def stage(files):
            for name, (body, executable) in files.items():
                (guest / name).write_bytes(body)
                fixture.inputs[name] = hashlib.sha256(body).hexdigest()
        fixture.stage = stage
        owner = SimpleNamespace(pid=1, poll=lambda: 0, wait=lambda timeout: 0)
        with patch("lib.mac_linux_fixture.subprocess.Popen", return_value=owner):
            fixture.launch(["node", "synthetic-owned-programme"], host / output_relative,
                host / "original-ssh.log", resource_cycles=cycles)
        self.addCleanup(fixture.log.close)
        fixture.verify_inputs = lambda: True
        fixture.call = lambda command, **kwargs: self.guest_collect(guest, command)
        return fixture, guest

    @staticmethod
    def guest_collect(guest, command):
        if "collect" not in command:return b"true"
        output = io.BytesIO()
        with patch.object(sys, "stdout", SimpleNamespace(buffer=output)):
            mac_linux_guest.collect(guest, str(command[-1]))
        return output.getvalue()

    def test_owned_three_cycle_output_bound_round_trips_through_original_close_and_collect(self):
        with patch("lib.mac_linux_fixture.LIMIT", 32768), patch.object(mac_linux_guest, "LIMIT", 32768):
            fixture, guest = self.transfer_fixture(3)
            result = fixture.close(release_lease=False)
            self.assertTrue(result["passed"]);self.assertTrue(result["original_joined"])
            self.assertEqual((fixture.output / "packet-capture/media.pcap").stat().st_size, 40000)
            profile = json.loads((fixture.output / "TRANSFER-PROFILE.json").read_text())
            self.assertEqual(profile, fixture.owned_output_profile)
            old, _ = self.transfer_fixture(1)
            self.assertFalse(old.close(release_lease=False)["passed"])
            self.assertFalse((old.output / "packet-capture/media.pcap").exists())

    def test_owned_transfer_rejects_unrelated_directory_escalation_and_invalid_cycles(self):
        with patch("lib.mac_linux_fixture.LIMIT", 32768), patch.object(mac_linux_guest, "LIMIT", 32768):
            fixture, guest = self.transfer_fixture(3)
            unrelated = guest / "other";unrelated.mkdir();(unrelated / "large").write_bytes(b"x" * 40000)
            with self.assertRaisesRegex(ValueError, "Private transfer bound"):
                self.guest_collect(guest, ["collect", "other"])
            for cycles in [0, 2, 4, True, "3"]:
                with self.subTest(cycles=cycles):
                    job = dict(run_id=guest.name, output="output", resource_cycles=cycles)
                    (guest / "job.json").write_text(json.dumps(job))
                    with self.assertRaisesRegex(ValueError, "collection profile"):
                        self.guest_collect(guest, ["collect", "output"])
                    profile = dict(fixture.owned_output_profile, resource_cycles=cycles)
                    with self.assertRaisesRegex(ValueError, "collection profile"):
                        unpack_verified(b"", self.root / "invalid", owned_job=profile)

    def test_owned_output_collection_binds_original_job_digest_before_writing(self):
        with patch("lib.mac_linux_fixture.LIMIT", 32768), patch.object(mac_linux_guest, "LIMIT", 32768):
            fixture, guest = self.transfer_fixture(3)
            changed = dict(run_id=guest.name, output="output", resource_cycles=3, source_revision="changed")
            (guest / "job.json").write_text(json.dumps(changed))
            self.assertFalse(fixture.close(release_lease=False)["passed"])
            self.assertFalse((fixture.output / "packet-capture/media.pcap").exists())

    def test_guest_collection_omits_ephemeral_profile_symlinks(self):
        (self.root/"trusted-profile").mkdir();(self.root/"trusted-profile/SingletonLock").symlink_to("missing-process-lock")
        (self.root/"browser-observation.json").write_text('{}')
        self.assertEqual([p.name for p in mac_linux_guest.regular_files(self.root)],["browser-observation.json"])

    def test_lease_never_uses_elapsed_time_as_unknown_owner_join(self):
        (self.root/".owners").mkdir()
        intent=self.root/".owners/command-original-intent.json";intent.write_text('{}')
        self.assertFalse(mac_linux_guest.lease_owners_joined(self.root))
        receipt=intent.with_name("command-original-receipt.json")
        receipt.write_text(json.dumps({"original_joined":True,"original_pid":123}))
        with patch.object(mac_linux_guest,"group_alive",return_value=True):
            self.assertFalse(mac_linux_guest.lease_owners_joined(self.root))
        with patch.object(mac_linux_guest,"group_alive",return_value=False):
            self.assertTrue(mac_linux_guest.lease_owners_joined(self.root))
        (self.root/"job-intent.json").write_text('{}')
        with patch.object(mac_linux_guest,"group_alive",return_value=False):
            self.assertFalse(mac_linux_guest.lease_owners_joined(self.root))

    def test_failed_media_does_not_hold_vm_when_original_owners_are_proven_joined(self):
        report={"format":"connect/transport-fixture-rehearsal/1","passed":False,
            "native_programme_started":True,"servers":[{"original_joined":True}],
            "native_process_profile":{"original_supervisor_joined":True,"original_workers_joined":True},
            "packet_capture":{"observed":False,"owner_created":False}}
        self.assertTrue(cleanup_evidence(report)["confirmed"])
        report["native_process_profile"]["original_workers_joined"]=False
        self.assertFalse(cleanup_evidence(report)["confirmed"])
        self.assertFalse(cleanup_evidence(None)["confirmed"])

    def test_authority_launch_attempt_requires_join_even_before_executed_marker(self):
        report={"format":"connect/transport-fixture-rehearsal/1","native_programme_started":False,
            "servers":[],"current_authority":{}}
        cases=[
            ({"executed":False,"launcher_attempted":True},False),
            ({"executed":False,"process_owner":{}},False),
            ({"executed":False,"launcher_attempted":False,"process_owner":{}},False),
            ({"executed":False},False),
            ({"executed":False,"launcher_attempted":False},True),
            ({"executed":False,"launcher_attempted":True,"process_owner":{
                "original_supervisor_joined":True,"original_workers_joined":True}},True),
        ]
        for kind in ("expiry","revocation"):
            for value,expected in cases:
                with self.subTest(kind=kind,value=value):
                    report["current_authority"]={kind:value}
                    self.assertEqual(cleanup_evidence(report)["authority_joined"],expected)
                    self.assertEqual(cleanup_evidence(report)["confirmed"],expected)

    def test_lease_excludes_second_workload_and_releases_original_lock(self):
        directory=self.root/"lease"
        with VmWorkloadLease(directory=directory):
            with self.assertRaises(TimeoutError):
                with VmWorkloadLease(directory=directory,timeout=.01):self.fail("Second workload entered")
        with VmWorkloadLease(directory=directory,timeout=.01):pass

if __name__=="__main__":unittest.main()
