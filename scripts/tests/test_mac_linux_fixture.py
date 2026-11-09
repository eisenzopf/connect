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
from unittest.mock import Mock, patch
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

    def test_nested_authority_bridge_round_trips_three_original_sequences_and_refuses_rewrite(self):
        fixture, guest, output = self.nested_resource_fixture()
        for cycle in (1, 2, 3):
            for sequence, kind in enumerate(("expiry", "revocation", "revoke"), 1):
                relative = Path("cycles") / ("cycle-" + str(cycle)) / ("authority-" + str(sequence) + "-request.json")
                request = output / relative;request.parent.mkdir(parents=True, exist_ok=True)
                original = {"sequence": sequence, "kind": kind};request.write_text(json.dumps(original))
                observed = fixture.poll()
                self.assertEqual(observed["requests"][relative.as_posix()], original)
                self.assertEqual(json.loads((fixture.output / relative).read_text()), original)
                response = relative.with_name("authority-" + str(sequence) + "-response.json")
                result = {"original_kind": kind, "original_sequence": sequence, "original_cycle": cycle, "passed": False}
                (fixture.output / response).write_text(json.dumps(result));fixture.poll()
                self.assertEqual(json.loads((output / response).read_text()), result)
                self.assertEqual(json.loads(request.read_text()), original)
        changed = fixture.output / "cycles/cycle-1/authority-1-response.json"
        changed.write_text(json.dumps({"passed": True}))
        with self.assertRaisesRegex(ValueError, "cannot be changed"):fixture.poll()

    def test_nested_authority_bridge_keeps_original_size_symlink_and_fixed_path_bounds(self):
        fixture, guest, output = self.nested_resource_fixture()
        nested = output / "cycles/cycle-1";nested.mkdir(parents=True)
        request = nested / "authority-1-request.json";request.write_text("x" * 4097)
        with self.assertRaisesRegex(ValueError, "Bounded original request"):fixture.poll()
        request.unlink();nested.rmdir()
        outside = self.root / "authority-outside";outside.mkdir();nested.symlink_to(outside, target_is_directory=True)
        (outside / "authority-1-request.json").write_text('{}')
        with self.assertRaisesRegex(ValueError, "Bounded original request"):fixture.poll()
        nested.unlink()
        for relative in ("cycles/cycle-4/authority-1-response.json", "cycles/cycle-0/authority-1-response.json", "cycles/cycle-1/authority-4-response.json", "../authority-1-response.json"):
            payload = {"directory": "browser", "responses": {relative: {"passed": False}}}
            with patch.object(sys, "stdin", io.StringIO(json.dumps(payload))), self.assertRaisesRegex(ValueError, "Bounded original response"):
                mac_linux_guest.exchange(guest)
        self.assertFalse((output / "cycles/cycle-4").exists())
        fixture.call = lambda *args, **kwargs: json.dumps({"monotonic_seconds": 0, "requests": {"cycles/cycle-1/authority-4-request.json": {}}, "state": None}).encode()
        with self.assertRaisesRegex(ValueError, "Bounded original cross-host request"):fixture.poll()
        self.assertFalse((fixture.output / "cycles/cycle-1/authority-4-request.json").exists())

    def test_actual_browser_authority_bridge_completes_all_nine_remote_handshakes(self):
        import subprocess
        import time
        fixture, guest, output = self.nested_resource_fixture()
        bridge = Path(__file__).resolve().parents[1] / "lib/matrix_authority_bridge.mjs"
        script = """
import {pathToFileURL} from 'node:url';
const {matrixAuthorityBridge} = await import(pathToFileURL(process.argv[1]));
const provision = matrixAuthorityBridge(process.argv[2], {cycles: 3});
for (let cycle = 1; cycle <= 3; cycle++) {
    for (const [index, kind] of ['expiry', 'revocation', 'revoke'].entries()) {
        const result = await provision(kind, cycle);
        if (result.original_kind !== kind || result.original_sequence !== index + 1 || result.original_cycle !== cycle || result.passed !== false)
            throw Error('Original authority request/negative response changed');
    }
}
console.log(JSON.stringify({handshakes: 9}));
"""
        child = subprocess.Popen(["node", "--input-type=module", "-e", script, str(bridge), str(output)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        served = set();cutoff = time.monotonic() + 5
        try:
            while child.poll() is None and time.monotonic() < cutoff:
                fixture.poll()
                for request in fixture.output.glob("cycles/cycle-[123]/authority-[123]-request.json"):
                    relative = request.relative_to(fixture.output)
                    if relative in served:continue
                    value = json.loads(request.read_text())
                    self.assertIn(value["kind"], ("expiry", "revocation", "revoke"))
                    self.assertIn(value["sequence"], (1, 2, 3))
                    response = request.with_name(request.name.replace("-request.json", "-response.json"))
                    response.write_text(json.dumps({"original_kind": value["kind"], "original_sequence": value["sequence"], "original_cycle": int(relative.parts[1].removeprefix("cycle-")), "passed": False}))
                    served.add(relative)
                fixture.poll();time.sleep(.005)
            self.assertIsNotNone(child.poll(), "Actual browser authority helper did not receive original nested responses")
            stdout, stderr = child.communicate(timeout=1)
            self.assertEqual(child.returncode, 0, stderr)
            self.assertEqual(json.loads(stdout), {"handshakes": 9});self.assertEqual(len(served), 9)
            self.assertTrue(child.stdout.closed);self.assertTrue(child.stderr.closed)
        finally:
            if child.poll() is None:child.terminate()
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
        self.assertEqual(result["lease_cleanup"]["original_ssh_exit_code"], 1)
        self.assertFalse(result["lease_cleanup"]["forced"])
        self.assertTrue(lease.stdin.closed and lease.stdout.closed)
        self.assertIn("TimeoutError", (fixture.directory / "lease-acquisition.stderr").read_text())
        self.assertEqual(fixture.inputs, {});self.assertIsNone(fixture.owner)

    def test_unjoined_original_queue_process_never_becomes_guest_cleanup_success(self):
        from lib import mac_linux_fixture as module
        fixture = MacLinuxFixture(self.root, self.root / "queue-unjoined", {}, resource_cycles=3)
        fixture.lease_attempt = dict(acquired=False, started_unix=1, started_monotonic_seconds=1)
        fixture.lease = SimpleNamespace(
            pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=io.BytesIO(),
            poll=lambda: None,
            wait=Mock(side_effect=module.subprocess.TimeoutExpired("owned-queue", 5)),
            send_signal=Mock(side_effect=PermissionError("synthetic owned-child refusal")),
        )
        with patch.object(module.os, "getpgid", return_value=123), \
                patch.object(module.os, "killpg", side_effect=PermissionError("synthetic owned-group refusal")):
            result = fixture.close()
        self.assertFalse(result["passed"]);self.assertFalse(result["original_joined"])
        self.assertFalse(result["original_lease_ssh_joined"])
        self.assertEqual(result["cleanup_failures"], ["original-lease-ssh-join-unconfirmed"])
        self.assertEqual(fixture.lease.wait.call_count, 3)
        self.assertIsNone(result["lease_cleanup"]["original_ssh_exit_code"])
        self.assertTrue(result["forced"])
        self.assertTrue(result["lease_cleanup"]["stop_errors"])
        self.assertTrue(fixture.lease.stdin.closed and fixture.lease.stdout.closed and fixture.lease.stderr.closed)

    def test_interrupted_pre_acquire_closes_only_original_queue_and_preserves_history(self):
        from lib import mac_linux_fixture as module
        fixture = MacLinuxFixture(self.root, self.root / "interrupted-queue", {})
        state = {"code": None}
        lease = SimpleNamespace(
            pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=io.BytesIO(),
            poll=lambda: state["code"],
            wait=Mock(side_effect=[module.subprocess.TimeoutExpired("owned-queue", 5), -15]),
            send_signal=Mock(),
        )
        class Select:
            def __enter__(self):
                return self
            def __exit__(self, *args):
                pass
            def register(self, *args):
                pass
            def select(self, timeout):
                raise KeyboardInterrupt
        def group_signal(pid, selected):
            self.assertEqual(pid, 123)
            if selected == 0:
                if state["code"] is None:
                    return
                raise ProcessLookupError
            self.assertEqual(selected, module.signal.SIGTERM)
            state["code"] = -15
        fixture.call = lambda *args, **kwargs: self.fail("No guest command exists before acquisition")
        with patch.object(module.subprocess, "Popen", return_value=lease), \
                patch.object(module.selectors, "DefaultSelector", Select), \
                patch.object(module.os, "getpgid", return_value=123), \
                patch.object(module.os, "killpg", side_effect=group_signal) as signalled:
            with self.assertRaises(KeyboardInterrupt):
                fixture.acquire_lease()
            result = fixture.close()
            self.assertTrue(lease.stdin.closed, "Interrupted acquisition left the original queue pipe open")
            self.assertGreaterEqual(lease.wait.call_count, 2)
            self.assertTrue(result["original_lease_ssh_joined"])
            self.assertTrue(result["forced"])
            history = (fixture.directory / "lease-acquisition.json").read_bytes()
            current = (fixture.directory / "lease-cleanup.json").read_bytes()
            self.assertIs(fixture.close(), result)
            fixture.release_lease()
            self.assertEqual((fixture.directory / "lease-acquisition.json").read_bytes(), history)
            self.assertEqual((fixture.directory / "lease-cleanup.json").read_bytes(), current)
            self.assertEqual([call.args for call in signalled.call_args_list if call.args[1]], [(123, module.signal.SIGTERM)])
        self.assertTrue(lease.stdout.closed and lease.stderr.closed)
        self.assertIsNone(result["lease_acquisition"]["original_ssh_exit_code"])
        self.assertEqual(result["lease_cleanup"]["original_ssh_exit_code"], -15)
        self.assertFalse(result["passed"])
        self.assertFalse(result["original_joined"])
        self.assertFalse(result["guest_job_started"])
        self.assertNotIn("all_guest_owners_joined", result)
        self.assertNotIn("original_vm_lease_joined", result)

    def test_pre_acquire_eof_reaps_original_queue_without_signals_or_guest_success(self):
        from lib import mac_linux_fixture as module
        fixture = MacLinuxFixture(self.root, self.root / "queue-eof", {})
        fixture.lease_attempt = dict(acquired=False, started_unix=1, started_monotonic_seconds=1)
        state = {"code": None}
        def wait(timeout):
            state["code"] = 0
            return 0
        lease = SimpleNamespace(
            pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=io.BytesIO(),
            poll=lambda: state["code"], wait=Mock(side_effect=wait), send_signal=Mock(),
        )
        fixture.lease = lease
        def group_signal(pid, selected):
            self.assertEqual((pid, selected), (123, 0))
            if state["code"] is not None:
                raise ProcessLookupError
        with patch.object(module.os, "killpg", side_effect=group_signal):
            result = fixture.close()
            self.assertTrue(lease.stdin.closed, "Failed acquisition never closed its original pipe")
            self.assertTrue(result["original_lease_ssh_joined"])
            self.assertFalse(result["forced"])
            self.assertIs(fixture.close(), result)
        lease.wait.assert_called_once_with(timeout=5)
        lease.send_signal.assert_not_called()
        self.assertTrue(lease.stdout.closed and lease.stderr.closed)
        self.assertIsNone(result["lease_acquisition"]["original_ssh_exit_code"])
        self.assertEqual(result["lease_cleanup"]["original_ssh_exit_code"], 0)
        self.assertFalse(result["original_joined"])
        self.assertFalse(result["passed"])

    def test_pre_acquire_group_permission_error_still_reaps_original_child(self):
        from lib import mac_linux_fixture as module
        fixture = MacLinuxFixture(self.root, self.root / "queue-permission", {})
        fixture.lease_attempt = dict(acquired=False, started_unix=1, started_monotonic_seconds=1)
        state = {"code": None}
        def direct_signal(selected):
            self.assertEqual(selected, module.signal.SIGTERM)
            state["code"] = -15
        lease = SimpleNamespace(
            pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=io.BytesIO(),
            poll=lambda: state["code"],
            wait=Mock(side_effect=[module.subprocess.TimeoutExpired("owned-queue", 5), -15]),
            send_signal=Mock(side_effect=direct_signal),
        )
        fixture.lease = lease
        def group_signal(pid, selected):
            self.assertEqual(pid, 123)
            if selected == 0 and state["code"] is not None:
                raise ProcessLookupError
            if selected:
                raise PermissionError("synthetic exact-group permission refusal")
        with patch.object(module.os, "getpgid", return_value=123), \
                patch.object(module.os, "killpg", side_effect=group_signal):
            result = fixture.close()
        self.assertGreaterEqual(lease.wait.call_count, 2, "Group refusal skipped original Popen wait")
        lease.send_signal.assert_called_once_with(module.signal.SIGTERM)
        self.assertTrue(result["original_lease_ssh_joined"])
        self.assertEqual(result["lease_cleanup"]["original_ssh_exit_code"], -15)
        self.assertTrue(result["lease_cleanup"]["stop_errors"])
        self.assertFalse(result["passed"])
        self.assertTrue(lease.stdin.closed and lease.stdout.closed and lease.stderr.closed)
        # Losing the history write must not skip original-child cleanup either.
        fixture = MacLinuxFixture(self.root, self.root / "queue-history-error", {})
        fixture.lease_attempt = dict(acquired=False, started_unix=1, started_monotonic_seconds=1)
        fixture.lease = SimpleNamespace(
            pid=124, stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=io.BytesIO(),
            poll=lambda: 1, wait=Mock(return_value=1), send_signal=Mock(),
        )
        with patch.object(fixture, "_record_failed_lease", side_effect=OSError("synthetic history failure")), \
                patch.object(module.os, "killpg", side_effect=ProcessLookupError):
            with self.assertRaisesRegex(OSError, "synthetic history failure"):
                fixture.close()
        fixture.lease.wait.assert_called_once_with(timeout=5)
        fixture.lease.send_signal.assert_not_called()
        self.assertTrue(fixture.lease.stdin.closed and fixture.lease.stdout.closed and fixture.lease.stderr.closed)

    def test_pre_acquire_exited_leader_with_remaining_group_stays_unconfirmed(self):
        from lib import mac_linux_fixture as module
        fixture = MacLinuxFixture(self.root, self.root / "queue-exited-leader", {})
        fixture.lease_attempt = dict(acquired=False, started_unix=1, started_monotonic_seconds=1)
        fixture.lease = SimpleNamespace(
            pid=123, stdin=io.BytesIO(), stdout=io.BytesIO(), stderr=io.BytesIO(),
            poll=lambda: 1, wait=Mock(return_value=1), send_signal=Mock(),
        )
        # Model the Colima leader already reaped while its SSH group remains.
        # Group presence alone supplies no retained kernel identity for a kill.
        def group_signal(pid, selected):
            self.assertEqual((pid, selected), (123, 0))
        with patch.object(module.os, "killpg", side_effect=group_signal) as signalled, \
                patch.object(module.os, "getpgid") as identified:
            result = fixture.close()
            self.assertIs(fixture.close(), result)
        identified.assert_not_called()
        self.assertEqual([call.args for call in signalled.call_args_list], [(123, 0), (123, 0)])
        fixture.lease.wait.assert_called_once_with(timeout=5)
        fixture.lease.send_signal.assert_not_called()
        self.assertFalse(result["original_lease_ssh_joined"])
        self.assertFalse(result["original_joined"])
        self.assertFalse(result["passed"])
        self.assertEqual(result["cleanup_failures"], ["original-lease-ssh-join-unconfirmed"])
        self.assertIn("original-leader-exited-group-still-present", result["lease_cleanup"]["stop_errors"])
        self.assertEqual(result["lease_acquisition"]["original_ssh_exit_code"], 1)
        self.assertFalse(result["lease_acquisition"]["original_ssh_group_joined"])
        self.assertTrue(fixture.lease.stdin.closed and fixture.lease.stdout.closed and fixture.lease.stderr.closed)

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

    @staticmethod
    def original_failed_remote_owner():
        return dict(format="connect/mac-linux-owner/1", run_id="mac-remote-original", passed=False,
            original_joined=True, forced=False, cleanup_failures=[], client_inputs_unchanged=True,
            all_guest_owners_joined=True, original_vm_lease_joined=True,
            guest=dict(format="connect/mac-linux-original-job/1", run_id="mac-remote-original",
                original_pid=12345, original_start_ticks=9876, original_joined=True, forced=False, exit_code=1,
                resources=dict(groups=dict(browser=dict(live_processes=17)))))

    def test_remote_browser_failed_programme_projects_only_original_joined_custody(self):
        import copy
        from lib.mac_linux_fixture import remote_browser_owner_joined
        original = self.original_failed_remote_owner();before = copy.deepcopy(original)
        self.assertTrue(remote_browser_owner_joined(original))
        self.assertEqual(original, before, "Do not rewrite original failed verdicts or last prejoin samples")
        self.assertFalse(original["passed"]);self.assertEqual(original["guest"]["exit_code"], 1)
        self.assertEqual(original["guest"]["resources"]["groups"]["browser"]["live_processes"], 17,
            "The final original group join, not an old process sample, proves custody")

    def test_remote_browser_missing_foreign_forced_or_unconfirmed_custody_never_uses_passed(self):
        import copy
        from lib.mac_linux_fixture import remote_browser_owner_joined
        changes = [
            (("original_joined",), False), (("original_joined",), "true"),
            (("all_guest_owners_joined",), False), (("original_vm_lease_joined",), False),
            (("forced",), True), (("cleanup_failures",), ["original-guest-owners-unconfirmed"]),
            (("cleanup_failures",), None), (("client_inputs_unchanged",), False),
            (("guest", "run_id"), "foreign-original"), (("guest", "original_joined"), False),
            (("guest", "forced"), True), (("guest", "original_pid"), True),
            (("guest", "original_pid"), 1), (("guest", "original_start_ticks"), 0),
            (("guest", "exit_code"), None), (("guest", "exit_code"), True),
            (("format",), "unknown"), (("guest", "format"), "unknown"), (("run_id",), "")]
        for keys, value in changes:
            with self.subTest(keys=keys, value=value):
                receipt = copy.deepcopy(self.original_failed_remote_owner());receipt["passed"] = True
                target = receipt
                for key in keys[:-1]:target = target[key]
                target[keys[-1]] = value
                self.assertFalse(remote_browser_owner_joined(receipt))
        for receipt in (None, {}, {"passed": True}, {"guest": None}):
            self.assertFalse(remote_browser_owner_joined(receipt))

    @classmethod
    def original_failed_remote_native_report(cls):
        owner = cls.original_failed_remote_owner()
        owner["guest"].update(source_revision="a" * 40,
            capture_owner_created=True, capture_original_joined=True)
        return dict(format="connect/transport-fixture-rehearsal/1", source_revision="a" * 40,
            passed=False, native_programme_started=True,
            servers=[dict(original_joined=True)],
            native_process_profile=dict(format="connect/mac-linux-native-process-resources/1",
                collection_complete=True, original_owner=owner),
            remote_client_owner=owner, packet_capture=dict(owner_created=True, original_joined=True))

    def test_remote_native_nested_custody_accepts_natural_nonzero_without_rewriting_verdict(self):
        import copy
        report = self.original_failed_remote_native_report()
        before = copy.deepcopy(report)
        self.assertNotIn("original_supervisor_joined", report["native_process_profile"])
        self.assertNotIn("original_workers_joined", report["native_process_profile"])
        evidence = cleanup_evidence(report)
        self.assertTrue(evidence["clients_joined"])
        self.assertTrue(evidence["confirmed"])
        self.assertEqual(report, before)
        self.assertFalse(report["passed"])
        self.assertEqual(report["remote_client_owner"]["guest"]["exit_code"], 1)

    def test_remote_native_nested_custody_refuses_missing_forced_foreign_or_unknown_proof(self):
        import copy
        changes = [
            (("format",), "unknown"), (("collection_complete",), False),
            (("forced",), True), (("forced_cleanup",), True),
            (("original_owner", "format"), "unknown"),
            (("original_owner", "original_joined"), False),
            (("original_owner", "all_guest_owners_joined"), False),
            (("original_owner", "original_vm_lease_joined"), False),
            (("original_owner", "forced"), True),
            (("original_owner", "client_inputs_unchanged"), False),
            (("original_owner", "cleanup_failures"), ["unknown-original-owner"]),
            (("original_owner", "guest", "format"), "unknown"),
            (("original_owner", "guest", "original_joined"), False),
            (("original_owner", "guest", "forced"), True),
            (("original_owner", "guest", "original_pid"), True),
            (("original_owner", "guest", "original_start_ticks"), 0),
            (("original_owner", "guest", "exit_code"), None),
            (("original_owner", "guest", "capture_original_joined"), False),
            (("original_owner", "guest", "source_revision"), "b" * 40),
        ]
        for keys, value in changes:
            with self.subTest(keys=keys):
                report = self.original_failed_remote_native_report()
                profile = report["native_process_profile"]
                profile.update(original_supervisor_joined=True, original_workers_joined=True)
                target = profile
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                self.assertFalse(cleanup_evidence(report)["confirmed"])
        for missing in ("original_owner", "collection_complete"):
            report = self.original_failed_remote_native_report()
            del report["native_process_profile"][missing]
            self.assertFalse(cleanup_evidence(report)["confirmed"])
        for missing in ("original_pid", "original_start_ticks", "source_revision", "capture_original_joined"):
            report = self.original_failed_remote_native_report()
            del report["native_process_profile"]["original_owner"]["guest"][missing]
            self.assertFalse(cleanup_evidence(report)["confirmed"])
        report = self.original_failed_remote_native_report()
        report["remote_client_owner"] = copy.deepcopy(report["remote_client_owner"])
        report["remote_client_owner"]["run_id"] = "foreign-original"
        self.assertFalse(cleanup_evidence(report)["confirmed"])
        report = self.original_failed_remote_native_report()
        report["native_programme_started"] = False
        report["native_process_profile"]["original_owner"]["forced"] = True
        self.assertFalse(cleanup_evidence(report)["confirmed"], "unentered marker cannot erase an actual owner")
        for unknown in (None, {}, dict(format="unknown", original_supervisor_joined=True,
                                      original_workers_joined=True)):
            report = self.original_failed_remote_native_report()
            report["native_programme_started"] = False
            report["native_process_profile"] = unknown
            self.assertFalse(cleanup_evidence(report)["confirmed"])

    def test_lease_excludes_second_workload_and_releases_original_lock(self):
        directory=self.root/"lease"
        with VmWorkloadLease(directory=directory):
            with self.assertRaises(TimeoutError):
                with VmWorkloadLease(directory=directory,timeout=.01):self.fail("Second workload entered")
        with VmWorkloadLease(directory=directory,timeout=.01):pass

if __name__=="__main__":unittest.main()
