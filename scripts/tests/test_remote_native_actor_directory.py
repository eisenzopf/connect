"""Exercise the actual source loop/actor snapshot/final guard without its CLI main."""
import ast
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[2] / 'scripts/check-transport-fixtures.py'

class RemoteNativeActorDirectory(unittest.TestCase):
    def test_remote_collection_preserves_actor_integrity_scope_and_refuses_real_changes(self):
        tree = ast.parse(SOURCE.read_text(), filename=str(SOURCE))
        def only(nodes):
            nodes = list(nodes)
            self.assertEqual(len(nodes), 1, 'actual source seam must remain unique')
            return copy.deepcopy(nodes[0])
        files = only(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'files')
        self.assertEqual(len(files.body), 1)
        self.assertIsInstance(files.body[0], ast.Return)
        actor = only(n for n in ast.walk(tree) if isinstance(n, ast.Assign)
            and len(n.targets) == 1 and isinstance(n.targets[0], ast.Name) and n.targets[0].id == 'directory'
            and isinstance(n.value, ast.BinOp) and isinstance(n.value.right, ast.Constant) and n.value.right.value == 'actors')
        installed = only(n for n in ast.walk(tree) if isinstance(n, ast.Assign)
            and len(n.targets) == 1 and isinstance(n.targets[0], ast.Name) and n.targets[0].id == 'installed'
            and isinstance(n.value, ast.Call) and isinstance(n.value.func, ast.Name) and n.value.func.id == 'files')
        loop = only(n for n in ast.walk(tree) if isinstance(n, ast.For)
            and len(n.body) == 1 and isinstance(n.body[0], ast.Expr) and isinstance(n.body[0].value, ast.Call)
            and isinstance(n.body[0].value.func, ast.Attribute) and n.body[0].value.func.attr == 'collect_directory'
            and isinstance(n.body[0].value.func.value, ast.Name) and n.body[0].value.func.value.id == 'remote')
        guard = only(n.value for n in ast.walk(tree) if isinstance(n, ast.keyword) and n.arg == 'configs_unchanged')
        self.assertIsInstance(guard, ast.Lambda)
        observations = []
        scratch = SOURCE.parents[1] / ".local"
        scratch.mkdir(exist_ok=True)
        for cycles in [1, 3]:
            with tempfile.TemporaryDirectory(prefix='connect-remote-actors-', dir=scratch) as temporary:
                base = Path(temporary); output = base / 'materialized'; private = base / 'private'; evidence = base / 'evidence'
                actors = output / 'actors'; actor_file = actors / 'participant-1/client.toml'
                actor_file.parent.mkdir(parents=True); actor_file.write_bytes(b'original fixture actor config\n'); actor_file.chmod(0o600)
                (actors / 'actors.json').write_text('{"fixture":"original"}\n'); (actors / 'actors.json').chmod(0o600)
                (private / 'av-received').mkdir(parents=True)
                (private / 'av-received/received.json').write_text('{"different":"capture inventory"}\n')
                collected = []
                class Remote:
                    def collect_directory(self, directory): collected.append(directory)
                namespace = {'output': output, 'OUTPUT': evidence, 'PRIVATE': private,
                    'resource_cycles': cycles, 'remote': Remote(),
                    'digest': lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()}
                # Execute only actual source nodes, not imports, main, CLI or remote IO.
                initial = ast.Module(body=[copy.deepcopy(n) for n in [files, actor, installed, loop]], type_ignores=[])
                exec(compile(ast.fix_missing_locations(initial), str(SOURCE), 'exec'), namespace)
                actual_guard = eval(compile(ast.fix_missing_locations(ast.Expression(body=copy.deepcopy(guard))), str(SOURCE), 'eval'), namespace)
                expected = [evidence, *([private / ('native-cycle-' + str(i)) for i in [1, 2, 3]] if cycles == 3 else [private / 'conversation-control']), private / 'av-received']
                initial_unchanged = actual_guard()
                actor_file.write_bytes(b'changed fixture actor config\n')
                bytes_refused = actual_guard() is False
                actor_file.write_bytes(b'original fixture actor config\n'); actor_file.chmod(0o644)
                mode_refused = actual_guard() is False
                actor_file.chmod(0o600)
                restored = actual_guard()
                observations.append({'cycles':cycles,'original_collections_exact':collected == expected,
                    'actor_scope_preserved':namespace['directory'] == actors,
                    'guard_target':str(namespace['directory'].relative_to(base)),
                    'unchanged_actors_accepted':initial_unchanged,
                    'actual_actor_byte_change_refused':bytes_refused,'actual_actor_mode_change_refused':mode_refused,
                    'restored_original_accepted':restored})
            self.assertFalse(base.exists(), 'all original private test files must be removed before the oracle')
        print('remote_actor_directory counter ' + json.dumps({'all_original_collections_settled':True,
            'all_private_test_directories_removed':True,'observations':observations}, sort_keys=True), flush=True)
        self.assertTrue(all(x['original_collections_exact'] for x in observations))
        self.assertTrue(all(x['actual_actor_byte_change_refused'] and x['actual_actor_mode_change_refused'] for x in observations),
            'the real integrity guard may not become an unconditional acceptance')
        self.assertTrue(all(x['actor_scope_preserved'] and x['unchanged_actors_accepted'] and x['restored_original_accepted'] for x in observations),
            'remote capture collection replaced the original actor-config integrity scope')

if __name__ == '__main__':
    unittest.main()
