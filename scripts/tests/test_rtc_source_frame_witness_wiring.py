"""Exercise actual source argv branches only; never import/run a producer."""
import ast
import copy
from pathlib import Path
from types import SimpleNamespace
import unittest
ROOT=Path(__file__).resolve().parents[2]
def tree(path):return ast.parse((ROOT/path).read_text())
def execute(nodes,values):
 module=ast.fix_missing_locations(ast.Module(body=copy.deepcopy(nodes),type_ignores=[]))
 exec(compile(module,'<actual-source-frame-wiring>','exec'),values)
 return values
class Wiring(unittest.TestCase):
 def test_both_host_worker_argv_explicitly_record_the_prospective_profile(self):
  run=next(n for n in tree('scripts/lib/transport_matrix_coordinator.py').body if isinstance(n,ast.FunctionDef)and n.name=='run')
  declared=next(n for n in run.body if isinstance(n,ast.Assign)and any(isinstance(t,ast.Name)and t.id=='worker' for t in n.targets))
  forwarded=next(n for n in run.body if isinstance(n,ast.AugAssign)and ast.unparse(n.target)=='worker'and '--rtc-source-frame-witness' in ast.unparse(n.value))
  values=execute([declared,forwarded],{'revision':'original-source'})
  self.assertEqual(values['worker'],['--phase','worker','--stage','full','--expected-revision','original-source','--rtc-source-frame-witness'])
 def test_actual_direct_executor_only_selects_firefox_quality_in_both_network_profiles(self):
  node=next(n for n in ast.walk(tree('scripts/run-gate6-transport-matrix.py')) if isinstance(n,ast.If)
   and 'firefox-wss-webrtc' in ast.unparse(n.test)and '--rtc-source-frame-witness' in ast.unparse(n))
  for transport in ['native-quic','chrome-webtransport','chrome-wss-webrtc','firefox-wss-webrtc']:
   for stage in ['admission','programme','quality']:
    for network in ['local','remote-profile-fixture']:
     argv=['original'];execute([node],{'cell':{'transport':transport,'network':network,'execution':{'stage':stage}},'sys':SimpleNamespace(argv=argv)})
     self.assertEqual(argv,['original']+(['--rtc-source-frame-witness']if transport=='firefox-wss-webrtc'and stage=='quality'else []))
if __name__=='__main__':unittest.main()
