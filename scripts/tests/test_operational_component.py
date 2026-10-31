"""Completed-artifact cache reuse never promotes failed behavior or compilation."""
import copy
import importlib.util
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/"scripts"))
spec=importlib.util.spec_from_file_location("operational_component",ROOT/"scripts/check-operational-visibility.py")
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class CacheBoundary(unittest.TestCase):
    def completed(self,status=0):
        return dict(exit_code=status,executables=[dict(name="fixture",sha256="a"*64)],compiler_errors=[])

    def test_completed_target_survives_later_compiler_refusal_without_promoting_results(self):
        steps=[self.completed(),dict(exit_code=101,executables=[],compiler_errors=[dict(code="E0308")])]
        before=copy.deepcopy(steps)
        self.assertTrue(module.cacheable_steps(steps))
        self.assertEqual(steps,before)
        self.assertNotEqual(steps[1]["exit_code"],0)

    def test_no_completed_executable_or_invalid_digest_is_not_cache_qualification(self):
        for steps in ([],[dict(exit_code=0,executables=[],compiler_errors=[])],
                      [dict(exit_code=0,executables=[dict(sha256="bad")],compiler_errors=[])]):
            self.assertFalse(module.cacheable_steps(steps))

    def test_compile_error_is_not_hidden_by_an_executable_field(self):
        value=self.completed(101);value["compiler_errors"]=[dict(code="E0308")]
        self.assertFalse(module.cacheable_steps([value]))

    def test_runtime_failure_may_preserve_completed_compilation_but_timeout_does_not(self):
        self.assertTrue(module.cacheable_steps([self.completed(101)]))
        for status in ("timeout",-9,-15):
            self.assertFalse(module.cacheable_steps([self.completed(status)]))

if __name__=="__main__":unittest.main()
