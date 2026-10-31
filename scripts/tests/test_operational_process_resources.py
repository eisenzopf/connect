"""PID reuse, role separation and numeric-only process observation boundaries."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location("operational_resources",ROOT/"scripts/lib/operational_process_resources.py")
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class ProcessBoundary(unittest.TestCase):
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

if __name__=="__main__":unittest.main()
