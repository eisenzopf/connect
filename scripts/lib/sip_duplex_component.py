"""Run the exact prepared production SIP data selector in a small test target."""
import hashlib
import json
import re


def run(root, execute):
    base = root / '.deps/rvoip/crates/sip/rvoip-sip/src/media_stream'
    source = base / 'guarded/duplex.rs'
    bridge = (base / 'guarded.rs').read_text()
    assert bridge.count('mod duplex;') == bridge.count('duplex::next(') == 1
    declared = re.findall(r'#\[tokio::test\]\s*async fn (\w+)', source.read_text())
    assert len(declared) == len(set(declared)) == 3
    result = dict(passed=False, status='unrun', source_module_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
        scope='Exact prepared production SIP queue selector: bounded bidirectional service with both queues ready, single-direction FIFO and cancellation-safe idle waits. No RTP, telephone, provider or full G6-054 qualification.',
        full_task_qualified=False, acceptance_qualified=False)
    text = execute('compile-sip-duplex-selector', ['cargo', 'test', '--offline', '--locked',
        '--jobs', '2', '-p', 'connect-media-conformance', '--test', 'gate6_sip_duplex',
        '--no-run', '--message-format=json'], 3600, defer_failure=True)
    if text is None:
        return result
    rows = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
    assert any(row.get('reason') == 'build-finished' and row.get('success') for row in rows)
    binaries = [row['executable'] for row in rows if row.get('reason') == 'compiler-artifact'
        and row['target']['name'] == 'gate6_sip_duplex' and row['profile']['test'] and row.get('executable')]
    assert len(binaries) == 1
    with open(binaries[0], 'rb') as stream:
        result['binary_sha256'] = hashlib.file_digest(stream, 'sha256').hexdigest()
    prefix = 'production_duplex::tests::'
    inventory = execute('sip-duplex-selector-inventory', [binaries[0], '--list'], 10)
    assert {line.removesuffix(': test') for line in inventory.splitlines() if line.endswith(': test')} == {prefix + name for name in declared}
    text = execute('sip-duplex-selector-contracts', [binaries[0], '--test-threads=1'], 30, defer_failure=True)
    result['results'] = [dict(name=name, passed=name + ' ... ok' in (text or '')) for name in declared]
    result['passed'] = text is not None and all(row['passed'] for row in result['results'])
    result['passed'] = result['passed'] and re.findall(r'^test result: ok\. (\d+) passed; 0 failed;', text or '', re.M) == ['3']
    result['status'] = 'passed' if result['passed'] else 'failed'
    return result
