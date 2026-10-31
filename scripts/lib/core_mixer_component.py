"""Run the actual guarded mixer tests from their pinned upstream workspace."""
import hashlib
import json
import re


def run(root, execute):
    source = root / '.deps/rvoip/crates/foundation/rvoip-core/src/staged_streams/tests/guarded_mixer.rs'
    declared = re.findall(r'#\[tokio::test\]\s*async fn (\w+)', source.read_text())
    assert len(declared) == len(set(declared)) and len(declared) >= 30
    result = dict(passed=False, status='unrun', full_task_qualified=False,
        acceptance_qualified=False, scope='Actual Core guarded mixer parent-module functions, including contiguous capture across empty input and real discarded PCM/missed quanta. No network/browser quality qualification.')
    text = execute('compile-core-mixer-clock', ['cargo', 'test', '--locked',
        '--manifest-path', '.deps/rvoip/Cargo.toml', '--jobs', '2', '-p', 'rvoip-core',
        '--lib', '--features', 'opus', '--no-run', '--message-format=json'], 3600, defer_failure=True)
    if text is None:
        return result
    rows = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
    assert any(row.get('reason') == 'build-finished' and row.get('success') for row in rows)
    binaries = [row['executable'] for row in rows if row.get('reason') == 'compiler-artifact'
        and row['target']['name'] == 'rvoip_core' and row['profile']['test'] and row.get('executable')]
    assert len(binaries) == 1
    binary = binaries[0]
    with open(binary, 'rb') as stream:
        result['binary_sha256'] = hashlib.file_digest(stream, 'sha256').hexdigest()
    prefix = 'staged_streams::tests::guarded_mixer::'
    inventory = execute('core-mixer-clock-inventory', [binary, prefix, '--list'], 10)
    observed = {line.removesuffix(': test') for line in inventory.splitlines() if line.endswith(': test')}
    expected = {prefix + name for name in declared}
    assert expected <= observed and all(name.startswith(prefix) for name in observed)
    # Run each real parent-module function with exact selection. Submodule
    # functions have their own separate inventories and are not claimed here.
    result['results'] = []
    for name in declared:
        text = execute('core-mixer-clock-' + name, [binary, prefix + name,
            '--exact', '--test-threads=1'], 30, defer_failure=True)
        passed = text is not None and re.findall(r'^test result: ok\. (\d+) passed; 0 failed;', text, re.M) == ['1']
        passed = passed and name + ' ... ok' in (text or '')
        result['results'].append(dict(name=name, passed=passed))
    result['passed'] = all(row['passed'] for row in result['results'])
    result['status'] = 'passed' if result['passed'] else 'failed'
    return result
