"""Cross-host authority staging/replay boundaries, without any VM workloads."""
import json
from pathlib import Path
import tempfile
import tomllib
from types import SimpleNamespace
import unittest

from lib.native_matrix_remote_authority import RemoteNativeAuthorityFactory


class FakeFixture:
    instances=[]
    def __init__(self,root,directory,env):
        self.root,self.directory,self.env=root,directory,env
        directory.mkdir()
        self.remote_root=Path("/guest")/str(len(self.instances))
        self.instances.append(self)
        self.events=[];self.process_code=None;self.helpers_joined=True
        self.owner=SimpleNamespace(poll=lambda:self.process_code)
    def start(self,revision,*,image):
        self.events.append("candidate-staged-before-issuance")
        self.revision=revision;self.server_ip="192.168.1.2";self.client_ip="192.168.1.3"
        self.test_image=image
    def mapped(self,path):return self.remote_root/Path(path).relative_to(self.root)
    def stage_native_config(self,path):
        self.config=tomllib.loads(path.read_text());self.events.append("native-config-staged")
    def stage_json(self,path,value):self.input=value;self.events.append("input-staged")
    def launch(self,command,output,log,*,timeout,role):
        self.command,self.output,self.role,self.timeout=command,output,role,timeout
        self.events.append("original-guest-probe-started")
    def poll(self):self.events.append("original-bridge-poll")
    def close(self):
        self.events.append("original-guest-joined-and-collected")
        guest=dict(exit_code=0,original_joined=True)
        (self.output/"guest-owner.json").write_text(json.dumps(guest))
        (self.output/"observation.json").write_text(json.dumps(dict(original_helpers_joined=self.helpers_joined,
            original_owners_joined=self.helpers_joined,opened_client_owners=2,closed_client_owners=2,
            observed_helpers=1,binary_sha256="b"*64)))
        return dict(original_joined=True,guest=guest,forced=False,passed=True)


class RemoteNativeAuthorityTests(unittest.TestCase):
    def fixture(self,temporary):
        root=Path(temporary).resolve();case=root/"case";case.mkdir()
        parent=SimpleNamespace(root=root,env={},revision="a"*40,client_binary="/guest/main/connect",
            client_binary_sha256="b"*64,server_ip="192.168.1.2",client_ip="192.168.1.3")
        factory=RemoteNativeAuthorityFactory(parent,fixture_class=FakeFixture)
        factory.prepare(case)
        self.assertIs(FakeFixture.instances[-1].lease_parent,parent)
        self.assertEqual(FakeFixture.instances[-1].events,["candidate-staged-before-issuance"])
        config=case/"sdk.toml"
        config.write_text('tenant_id="tnt_default"\n[control]\nendpoint="192.168.1.2:5555"\n')
        value=dict(executable=parent.client_binary,binary_sha256=parent.client_binary_sha256,
            source_revision=parent.revision,network_profile="remote",config=str(config))
        source=case/"input.json";source.write_text(json.dumps(value))
        command=["node","scripts/check-transport-native-authority.mjs",source,case/"observation.json"]
        return factory,parent,case,config,source,command

    def test_guest_uses_exact_arm_executable_and_native_config_without_mac_admin_route(self):
        with tempfile.TemporaryDirectory() as temporary:
            factory,parent,case,config,source,command=self.fixture(temporary)
            original=config.read_bytes()
            owner=factory(command,case/"owner",case/"process.log",cwd=parent.root,env={})
            remote=FakeFixture.instances[-1]
            self.assertNotIn("application",remote.config)
            self.assertEqual(config.read_bytes(),original)
            self.assertEqual(remote.input["executable"],parent.client_binary)
            self.assertEqual(remote.role,"helper");self.assertEqual(remote.timeout,45)
            self.assertIsNone(owner.poll())
            remote.process_code=0
            self.assertEqual(owner.poll(),0)
            receipt=owner.close()
            self.assertTrue(receipt["original_supervisor_joined"])
            self.assertTrue(receipt["original_workers_joined"])
            self.assertFalse(receipt["forced"])
            self.assertEqual(receipt["executable_sha256"],"b"*64)
            self.assertEqual(remote.events.count("original-guest-joined-and-collected"),1)
            with self.assertRaises(ValueError):factory(command,case/"owner",case/"process.log",cwd=parent.root,env={})

    def test_changed_client_binary_is_refused_before_guest_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            factory,parent,case,config,source,command=self.fixture(temporary)
            value=json.loads(source.read_text());value["binary_sha256"]="c"*64;source.write_text(json.dumps(value))
            with self.assertRaises(ValueError):factory(command,case/"owner",case/"process.log",cwd=parent.root,env={})
            self.assertNotIn("original-guest-probe-started",FakeFixture.instances[-1].events)

    def test_optional_application_cannot_be_silently_removed_for_the_guest(self):
        with tempfile.TemporaryDirectory() as temporary:
            factory,parent,case,config,source,command=self.fixture(temporary)
            with config.open("a") as stream:
                stream.write('[application]\ncredential="synthetic-expired-http"\nexpires_at=1970-01-01T00:00:00Z\n')
            original=config.read_bytes()
            with self.assertRaisesRegex(ValueError,"must be native-only"):
                factory(command,case/"owner",case/"process.log",cwd=parent.root,env={})
            self.assertEqual(config.read_bytes(),original)
            self.assertNotIn("native-config-staged",FakeFixture.instances[-1].events)
            self.assertNotIn("original-guest-probe-started",FakeFixture.instances[-1].events)

    def test_guest_process_success_does_not_hide_unjoined_actual_sdk_helper(self):
        with tempfile.TemporaryDirectory() as temporary:
            factory,parent,case,config,source,command=self.fixture(temporary)
            owner=factory(command,case/"owner",case/"process.log",cwd=parent.root,env={})
            remote=FakeFixture.instances[-1];remote.helpers_joined=False;remote.process_code=0
            self.assertEqual(owner.poll(),0)
            self.assertFalse(owner.close()["original_workers_joined"])


if __name__=="__main__":unittest.main()
