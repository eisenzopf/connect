"""Actual RTC outgoing-handler compound SR/CNAME component, without network IO."""
import hashlib
import json
import re


def run(root, execute):
    source = root / '.deps/rvoip/crates/webrtc/rvoip-rtc/src/peer_connection/handler/interceptor/compound_source_reports_tests.rs'
    names = re.findall(r'#\[test\]\s*fn (\w+)', source.read_text())
    assert len(names) == len(set(names)) == 3
    result = dict(passed=False, status='unrun', full_task_qualified=False, acceptance_qualified=False,
        scope='Exact RTC protocol outgoing handler: known local SRs carry their signaled CNAME, groups remain distinct, existing compounds/unknown sources/reduced feedback are not relabelled. No browser AV or full G6-054 qualification.',
        test_source_sha256=hashlib.sha256(source.read_bytes()).hexdigest())
    text = execute('compile-rtc-compound-reports', ['cargo', 'test', '--locked', '--manifest-path',
        '.deps/rvoip/Cargo.toml', '--jobs', '2', '-p', 'rvoip-rtc', '--lib', '--no-run',
        '--message-format=json'], 3600, defer_failure=True)
    if text is None:
        return result
    rows = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
    assert any(row.get('reason') == 'build-finished' and row.get('success') for row in rows)
    binaries = [row['executable'] for row in rows if row.get('reason') == 'compiler-artifact'
        and row['target']['name'] == 'rtc' and row['profile']['test'] and row.get('executable')]
    assert len(binaries) == 1
    binary = binaries[0]
    with open(binary, 'rb') as stream:
        result['binary_sha256'] = hashlib.file_digest(stream, 'sha256').hexdigest()
    prefix = 'peer_connection::handler::interceptor::compound_source_reports_tests::'
    inventory = execute('rtc-compound-inventory', [binary, prefix, '--list'], 10)
    assert {line.removesuffix(': test') for line in inventory.splitlines() if line.endswith(': test')} == {prefix + name for name in names}
    result['results'] = []
    for name in names:
        text = execute('rtc-compound-' + name, [binary, prefix + name, '--exact', '--test-threads=1'],
            10, defer_failure=True)
        passed = text is not None and re.findall(r'^test result: ok\. (\d+) passed; 0 failed;', text, re.M) == ['1']
        result['results'].append(dict(name=name, passed=passed, executed=True))
    with open(binary, 'rb') as stream:
        result['binary_unchanged'] = hashlib.file_digest(stream, 'sha256').hexdigest() == result['binary_sha256']
    result['passed'] = result['binary_unchanged'] and all(row['passed'] for row in result['results'])
    result['status'] = 'passed' if result['passed'] else 'failed'
    return result
