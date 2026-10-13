"""Full-size MCP framing, saturated body deadlines, and honest pending cursors."""
import json
import time

from .mcp_load_http import ADMISSIONS, BODY_BYTES, burst, encode, success


def framing(endpoint, task, report):
    status = encode('connect_task_status', {})
    maximum = status + b' ' * (BODY_BYTES - len(status))
    with report.phase('maximum-framing-and-overflow-bursts'):
        for _ in range(3):
            for result in burst(lambda _: endpoint.request(maximum), report):
                assert success(result)['structuredContent']['data']['id'] == task['id']
        for chunked in (False, True):
            for result in burst(lambda _: endpoint.request(maximum + b' ', chunked), report):
                assert result[0] == 413, result[0]
        assert success(endpoint.request(status))['structuredContent']['data']['id'] == task['id']
        report.checkpoint(maximum_jsonrpc_body_bytes=BODY_BYTES, accepted_maximum_bodies=96,
                          oversized_content_length_denied=32, oversized_chunked_denied=32)
    return maximum


def slow_bodies(endpoint, maximum, task, report):
    status = encode('connect_task_status', {})
    for wave in range(2):
        sockets = []
        with report.phase(f'saturated-near-limit-body-deadline-{wave + 1}'):
            try:
                deadline = time.monotonic() + 2
                for _ in range(ADMISSIONS):
                    sockets.append(endpoint.stalled(maximum, admission_deadline=deadline))
                assert time.monotonic() < deadline, 'Never saturated all 32 admission slots'
                # Every stalled body is one byte short. A separate rejected
                # request proves all admission permits are actually occupied.
                while True:
                    response = endpoint.request(status)
                    if response[0] == 503:
                        break
                    assert success(response)['structuredContent']['data']['id'] == task['id']
                    assert time.monotonic() < deadline, 'Never saturated all 32 admission slots'
                    time.sleep(.01)
                durations = []
                for connection, started in sockets:
                    response = connection.getresponse()
                    value = response.read(4096)
                    elapsed = time.monotonic() - started
                    assert response.status == 504, response.status
                    assert 9.5 <= elapsed < 11, elapsed
                    assert endpoint.token.encode() not in value
                    durations.append(elapsed)
                    report.latency(elapsed)
                assert success(endpoint.request(status))['structuredContent']['data']['id'] == task['id']
                report.checkpoint(**{f'stalled_wave_{wave + 1}': {
                    'admitted': ADMISSIONS, 'held_body_bytes_each': BODY_BYTES - 1,
                    'admission_confirmations': ADMISSIONS,
                    'admission_confirmation': '100-continue before each incomplete body',
                    'excess_status': 503, 'timeouts': len(durations), 'deadline_status': 504,
                    'recovered_status': 200}})
            finally:
                for connection, _ in sockets:
                    connection.close()


def pending_waits(endpoint, report):
    initial = success(endpoint.request(encode('connect_task_wait', {'limit': 100, 'wait_ms': 0})))
    cursor = initial['structuredContent']['data']['next']
    arguments = {'after': cursor, 'limit': 100, 'wait_ms': 10000}
    with report.phase('thirty-two-maximum-waits-and-cursor-recovery'):
        started = time.monotonic()
        results = burst(lambda _: endpoint.request(encode('connect_task_wait', arguments)), report)
        elapsed = time.monotonic() - started
        assert 9 <= elapsed < 11, elapsed
        for response in results:
            result = success(response)
            assert result['_meta']['connect']['pending'] is True
            data = result['structuredContent']['data']
            assert data['events'] == [] and data['next'] == cursor
        resumed = success(endpoint.request(encode('connect_task_wait', {**arguments, 'wait_ms': 0})))
        assert resumed['structuredContent']['data'] == data
        report.checkpoint(maximum_waits=32, returned_pending_cursors=32, original_cursor_reusable=True)


def recall(endpoint, source_ids, report):
    with report.phase('bounded-tool-history-under-bursts'):
        cursor, found, sizes = None, set(), []
        for _ in range(100):
            responses = burst(lambda _: endpoint.request(encode('connect_recall', {'after': cursor, 'limit': 100})), report)
            pages = [success(response)['structuredContent'] for response in responses]
            assert all(page == pages[0] for page in pages), 'Unchanged history yielded different projections'
            page = pages[0]
            size = len(json.dumps(page, ensure_ascii=False, separators=(',', ':')).encode())
            assert size <= 32 * 1024
            sizes.append(size)
            assert len(page['data']['sources']) <= 100
            for source in page['data']['sources']:
                if source['source_id'] in source_ids:
                    assert source['state'] == 'available' and source['provenance'].startswith('conversation-journal:')
                    found.add(source['source_id'])
            if found == source_ids:
                break
            next_cursor = page['data']['next']
            assert next_cursor is not None and next_cursor != cursor, 'Recall stopped before all sources'
            cursor = next_cursor
        assert found == source_ids
        report.checkpoint(recall_sources_found=len(found), recall_pages=len(sizes),
                          largest_structured_projection_bytes=max(sizes), recall_burst_width=ADMISSIONS)
