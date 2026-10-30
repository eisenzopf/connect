"""Parallel fixture children share a deadline and retain every original owner."""
import os
from pathlib import Path
import runpy
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
cohort=runpy.run_path(str(ROOT/'sdk/tests/check_python_helper.py'))['bounded_cohort']


class PythonComponentCohort(unittest.TestCase):
    def setUp(self):
        base=ROOT/'.local/gate-6/python-cohort-unit';base.mkdir(parents=True,exist_ok=True,mode=0o700)
        self.temporary=tempfile.TemporaryDirectory(dir=base)
        self.addCleanup(self.temporary.cleanup)
        self.output=Path(self.temporary.name)

    def run_pair(self,sources,budget):
        return cohort([[sys.executable,'-c',source] for source in sources],os.environ,
            [self.output/f'{i}.log' for i in range(len(sources))],budget)

    def test_independent_children_can_observe_each_other_before_either_exits(self):
        sources=[]
        for index in range(2):
            sources.append(f'''from pathlib import Path
import time
root=Path({str(self.output)!r});(root/{str(index)!r}).touch()
until=time.monotonic()+2
while not (root/{str(1-index)!r}).exists():
 assert time.monotonic()<until
 time.sleep(.005)
''')
        result=self.run_pair(sources,3)
        self.assertEqual(result['exit_code'],0)
        self.assertEqual(result['worker_exit_codes'],[0,0])
        self.assertFalse(result['timed_out'] or result['forced_cleanup'])
        self.assertTrue(result['original_processes_joined'] and result['groups_drained'])

    def test_a_sibling_does_not_renew_the_shared_deadline_and_owners_are_joined(self):
        result=self.run_pair(['import time;time.sleep(5)','pass'],.2)
        self.assertTrue(result['timed_out'])
        self.assertNotEqual(result['exit_code'],0)
        self.assertTrue(result['original_processes_joined'] and result['groups_drained'])
        self.assertFalse(result['forced_cleanup'])
        self.assertLess(result['seconds'],2)

    def test_one_failed_child_cannot_be_hidden_by_a_passing_sibling(self):
        result=self.run_pair(['raise SystemExit(7)','pass'],3)
        self.assertEqual(result['exit_code'],7)
        self.assertFalse(result['timed_out'] or result['forced_cleanup'])
        self.assertTrue(result['original_processes_joined'] and result['groups_drained'])
