"""PID reuse, role separation and numeric-only process observation boundaries."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import subprocess

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location("operational_resources",ROOT/"scripts/lib/operational_process_resources.py")
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class ProcessBoundary(unittest.TestCase):
    def test_browser_startup_failure_exposes_only_closed_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            private = Path(directory)
            request = private / "input.json"
            request.write_text(json.dumps(dict(dist=str(private / "secret-path-missing"),
                credential="secret-bearer-must-not-escape", endpoint="https://private.invalid")))
            result = subprocess.run(["node", str(ROOT / "scripts/operational-browser.mjs"), str(request)],
                capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stderr, "")
            self.assertEqual(json.loads(result.stdout), dict(phase="failed", stage="browser-sdk", name="Error", code="ENOENT"))
            self.assertNotIn("secret", result.stdout)

    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.proc=Path(self.temp.name)
        for pid in (100,200,300):
            self.write(pid,0,pid,1,2,3)

    def write(self,pid,parent,group,start,cpu,rss,children=""):
        path=self.proc/str(pid);path.mkdir(exist_ok=True)
        fields=["S","0","0"]+["0"]*19
        fields[1]=str(parent);fields[2]=str(group);fields[11]=str(cpu);fields[12]="0"
        fields[19]=str(start);fields[21]=str(rss)
        (path/"stat").write_text(str(pid)+" (private name ) secret) "+" ".join(fields))
        p=path/"task"/str(pid);p.mkdir(parents=True,exist_ok=True);(p/"children").write_text(children)

    def sampler(self):
        return module.ProcessResources(dict(server=100,helper=200,browser=300),self.proc,100,4096)

    def test_separate_owned_groups_preserve_numeric_metadata_without_names(self):
        self.write(300,0,300,1,2,3,"301")
        self.write(301,300,300,4,5,7)
        sample=self.sampler();value=sample.sample()
        self.assertEqual(value["groups"]["browser"]["live_processes"],2)
        self.assertEqual(value["groups"]["browser"]["sampled_rss_bytes"],10*4096)
        self.assertEqual(value["groups"]["browser"]["observed_cpu_seconds"],.07)
        self.assertEqual(value["groups"]["helper"]["live_processes"],1)
        text=json.dumps(sample.summary())
        self.assertNotIn("secret",text);self.assertNotIn("private name",text);self.assertNotIn(str(self.proc),text)

    def test_reused_root_pid_cannot_adopt_unrelated_descendants(self):
        sample=self.sampler();sample.sample()
        self.write(300,0,300,99,900,900,"301")
        self.write(301,300,300,100,900,900)
        value=sample.sample()["groups"]["browser"]
        self.assertEqual(value["root_state"],"retired")
        self.assertEqual(value["live_processes"],0)
        self.assertEqual(value["observed_cpu_seconds"],.02)
        self.assertFalse(any(key[0]==301 for key in sample.known))

    def test_missing_or_unreadable_metadata_is_not_zero_usage_availability(self):
        sample=self.sampler()
        (self.proc/"100"/"stat").unlink()
        (self.proc/"200"/"stat").write_text("bad")
        value=sample.sample()["groups"]
        self.assertEqual(value["server"]["root_state"],"retired")
        self.assertEqual(value["helper"]["root_state"],"unavailable")
        self.assertEqual(value["helper"]["unavailable_processes"],1)

    def test_duplicate_roots_and_cross_role_children_refuse(self):
        with self.assertRaises(ValueError):
            module.ProcessResources(dict(server=100,helper=100,browser=300),self.proc,100,4096)
        sample=self.sampler()
        self.write(200,300,200,1,2,3)
        self.write(300,0,300,1,2,3,"200")
        with self.assertRaisesRegex(ValueError,"overlap"):sample.sample()

    def test_native_two_roles_preserve_descriptor_unavailability_and_pid_identity(self):
        for pid, count in [(100, 2), (200, 3)]:
            directory = self.proc / str(pid) / "fd"
            directory.mkdir()
            for index in range(count): (directory / str(index)).touch()
        sampler = module.ProcessResources(dict(server=100,helper=200),self.proc,100,4096,
            roles=("server","helper"),open_resources=True,max_samples=3)
        groups = sampler.sample()["groups"]
        self.assertEqual(set(groups),{"server","helper"})
        self.assertEqual(groups["server"]["sampled_open_descriptors"],2)
        self.assertEqual(groups["helper"]["sampled_open_descriptors"],3)
        for p in (self.proc / "200/fd").iterdir(): p.unlink()
        (self.proc / "200/fd").rmdir()
        helper = sampler.sample()["groups"]["helper"]
        self.assertEqual(helper["root_state"],"available")
        self.assertIsNone(helper["sampled_open_descriptors"])
        self.assertEqual(helper["open_resource_unavailable_processes"],1)
        self.write(200,0,200,99,999,999,"201")
        self.write(201,200,200,99,999,999)
        helper = sampler.sample()["groups"]["helper"]
        self.assertEqual(helper["root_state"],"retired")
        self.assertEqual(helper["live_processes"],0)
        self.assertFalse(any(key[0]==201 for key in sampler.known))
        self.assertEqual(sampler.summary()["groups"]["helper"]["open_resource_unavailable_samples"],1)
        with self.assertRaisesRegex(ValueError,"sample budget"): sampler.sample()
        observed=json.dumps(sampler.summary())
        self.assertNotIn("private name",observed)
        self.assertNotIn(str(self.proc),observed)

if __name__=="__main__":unittest.main()
