"""Expired artifact receipts, source references, and durable terminal task records."""
import base64
import hashlib
import json
import time

from .mcp_load_http import burst, encode, success
from .retention_setup import call, LIVE_BYTES


def upload(jobs, report):
    expiry = int(time.time()) + 12
    with report.phase('thirty-two-expiring-files-and-bounded-chunks'):
        results = burst(lambda i: call(jobs[i], 'connect_artifact_begin', {
            'request_id': f'begin-{i}', 'step_id': 'file', 'name': f'file-{i}.bin',
            'media_type': 'application/octet-stream', 'size': len(jobs[i]['bytes']),
            'sha256': hashlib.sha256(jobs[i]['bytes']).hexdigest(), 'expires_at': expiry}), report)
        for job, result in zip(jobs, results):
            job['artifact'] = success(result)['structuredContent']['data']['outcome']['data']
        for offset in (0, 32768):
            results = burst(lambda i: call(jobs[i], 'connect_artifact_append', {
                'request_id': f'append-{i}-{offset}', 'step_id': 'file', 'artifact_id': jobs[i]['artifact']['id'],
                'offset': offset, 'data': base64.b64encode(jobs[i]['bytes'][offset:offset + 32768]).decode()}), report)
            for result in results:
                success(result)
        results = burst(lambda i: call(jobs[i], 'connect_artifact_finish', {
            'request_id': f'finish-{i}', 'step_id': 'file', 'artifact_id': jobs[i]['artifact']['id']}), report)
        for result in results:
            assert success(result)['structuredContent']['data']['outcome']['data']['state'] == 'ready'
        for job in jobs:
            job['read'] = {'request_id': f'read-{job["index"]}', 'step_id': 'read', 'artifact_id': job['artifact']['id'],
                           'offset': 0, 'limit': 32768}
        results = burst(lambda i: call(jobs[i], 'connect_artifact_get', jobs[i]['read']), report)
        for job, response in zip(jobs, results):
            chunk = success(response)['structuredContent']['data']['outcome']['data']
            assert base64.b64decode(chunk['data']) == job['bytes'][:32768]
            assert chunk['offset'] == 0 and chunk['eof'] is False
        report.checkpoint(ready_expiring_files=32, file_bytes=65536, verified_chunks=32, chunk_bytes=32768)
    return expiry


def forbidden(response):
    assert response[0] == 200
    value = json.loads(response[1])['result']
    assert value['isError'] is True and value['structuredContent']['code'] == 'forbidden'


def expired_files(jobs, report):
    responses = burst(lambda i: call(jobs[i], 'connect_artifact_get', jobs[i]['read']), report)
    for job, response in zip(jobs, responses):
        forbidden(response)
        assert base64.b64encode(job['bytes'][:32768]) not in response[1]
    responses = burst(lambda i: call(jobs[i], 'connect_recall', {'limit': 100}), report)
    for job, response in zip(jobs, responses):
        sources = success(response)['structuredContent']['data']['sources']
        artifact_sources = [source for source in sources if source['source_id'] == job['artifact']['id']]
        assert artifact_sources, 'Expired source references disappeared'
        assert all(source['state'] == 'expired' and source['body'] is None
                   and 'bytes-require-artifact-get' in source['provenance'] for source in artifact_sources)
        assert any(source['source_id'] == job['message']['message_id'] and source['state'] == 'available' for source in sources)


def expire_under_load(jobs, expiry, report):
    with report.phase('artifact-expiry-denies-cached-bytes-and-retains-provenance'):
        counts = {'available': 0, 'expired': 0}
        while time.time() < expiry + .05:
            responses = burst(lambda i: call(jobs[i], 'connect_artifact_get', jobs[i]['read']), report)
            for job, response in zip(jobs, responses):
                result = json.loads(response[1]).get('result', {})
                if result.get('isError') is True:
                    forbidden(response)
                    counts['expired'] += 1
                else:
                    chunk = success(response)['structuredContent']['data']['outcome']['data']
                    assert base64.b64decode(chunk['data']) == job['bytes'][:32768]
                    counts['available'] += 1
            time.sleep(.05)
        expired_files(jobs, report)
        assert counts['available'] >= 32
        report.checkpoint(artifact_expiry_responses=counts, expired_receipt_replays_denied=32,
                          expired_sources_retained=32, message_sources_available=32)


def cleanup(state, jobs, live, owner, report):
    blob_directory = state / 'state.blobs'
    assert not any((blob_directory / (job['artifact']['id'] + '.blob')).exists() for job in jobs)
    assert (blob_directory / (live['id'] + '.blob')).is_file()
    chunk = owner.artifact_get(jobs[0]['task']['conversation_id'], live['id'])
    assert base64.b64decode(chunk['data']) == LIVE_BYTES
    report.checkpoint(expired_blobs_removed_on_restart=32, live_control_blob_preserved=True)


def task_expiry(jobs, task_expiry_at, report):
    with report.phase('task-expiry-under-load-denies-further-tools'):
        counts = {200: 0, 401: 0}
        while time.time() < task_expiry_at + .05:
            responses = burst(lambda i: jobs[i]['endpoint'].request(encode('connect_task_status', {})), report)
            for job, response in zip(jobs, responses):
                assert response[0] in counts
                counts[response[0]] += 1
                if response[0] == 200:
                    assert success(response)['structuredContent']['data']['id'] == job['task']['id']
            time.sleep(.05)
        responses = burst(lambda i: jobs[i]['endpoint'].request(encode('connect_task_status', {})), report)
        assert all(response[0] == 401 for response in responses)
        report.checkpoint(task_expiry_responses=counts, expired_task_tools_denied=32)


def terminal_records(owner, jobs):
    for job in jobs:
        owner.negotiate_task(job['task']['id'])
        record = owner.task({'action': 'get', 'task_id': job['task']['id']})['data']
        assert record['id'] == job['task']['id'] and record['state'] == 'expired'
        assert record['conversation_id'] == job['task']['conversation_id']
        assert record['scope'] == job['task']['scope'] and record['expires_at'] == job['task']['expires_at']
        messages = owner.history(record['conversation_id'])['messages']
        assert len(messages) == 1 and messages[0]['id'] == job['message']['message_id']
        if 'terminal' in job:
            assert record == job['terminal'], 'Expired task changed after restart'
        job['terminal'] = record
