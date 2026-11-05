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
from lib.mac_linux_fixture import MacLinuxFixture, archive_files, rewrite_paths, unpack_verified
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
