"""Independent observations of actual host processes and their MCP event results."""
import json
from pathlib import Path
import subprocess


def snapshot(jobs, codex):
    """Read PID metadata while every corresponding real host is gated alive."""
    pids = [int(job['shim'].with_suffix('.pid').read_text()) for job in jobs]
    assert len(set(pids)) == len(jobs)
    rows = subprocess.check_output(['ps', '-o', 'pid=,pgid=,rss=,comm=', '-p', ','.join(map(str, pids))],
                                   text=True, timeout=3).splitlines()
    observations = []
    for row in rows:
        pid, group, rss, command = row.strip().split(maxsplit=3)
        assert Path(command).resolve() == codex, 'Recorded child is not the selected Codex executable'
        observations.append({'pid': int(pid), 'process_group': int(group), 'rss_bytes': int(rss) * 1024})
    assert {row['pid'] for row in observations} == set(pids), 'A host ended before simultaneous sampling'
    return sorted(observations, key=lambda row: row['pid'])


def exited(observed):
    """Read only: a surviving leader or inherited child fails this measurement."""
    groups = {row['process_group'] for row in observed}
    rows = subprocess.check_output(['ps', '-axo', 'pid=,pgid=,stat='], text=True, timeout=3).splitlines()
    alive = [int(pid) for pid, group, state in (row.split() for row in rows)
             if int(group) in groups and not state.startswith('Z')]
    assert not alive, f'Owned host groups still contain live processes: {alive}'


def verify_turn(job, warm):
    """Completion prose cannot substitute for successful, correctly bound tools."""
    directory = job['directory'] / 'host'
    state = json.loads((directory / 'host-state.json').read_text())
    path = directory / f"host-{state['run_id']}-{state['attempts']}.jsonl"
    assert path.stat().st_size <= 4 * 1024 * 1024
    events = [json.loads(line) for line in path.read_text().splitlines()]
    calls = [event['item'] for event in events if event['type'] == 'item.completed'
             and event['item']['type'] == 'mcp_tool_call']
    assert [c['tool'] for c in calls] == ['connect_task_status', 'connect_recall', 'connect_send']
    for call in calls:
        assert call['server'] == 'connect_task' and call['status'] == 'completed' and call['error'] is None
        assert call['result'].get('is_error') is not True and call['result'].get('isError') is not True
    status, recall, send = [call['result']['structured_content'] for call in calls]
    task = job['task']
    assert status['data']['id'] == task['id'] and status['data']['conversation_id'] == job['cid']
    source = next(s for s in recall['data']['sources'] if s['source_id'] == job['message']['message_id'])
    assert source['state'] == 'available' and json.loads(source['body'])['body'] == job['body']
    assert source['provenance'].split(';')[0] == 'conversation-journal:' + task['initiating_event_id']
    assert send['kind'] == 'tool' and send['data']['status'] == 'completed'
    assert send['data']['task_id'] == task['id']
    receipt = send['data']['outcome']
    assert receipt['kind'] == 'accepted' and receipt['data']['conversation_id'] == job['cid']
    assert receipt['data']['request_id'] == 'incoming-answer'
    if warm:
        assert send == job['tool_result'], 'Warm replay did not return the original task effect'
    job['tool_result'] = send
