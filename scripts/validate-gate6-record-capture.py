#!/usr/bin/env python3
"""Independent schema and captured-outcome checks over actual test exports."""
import json
from pathlib import Path
import subprocess
import sys
root = Path(__file__).resolve().parents[1]
directory = Path(sys.argv[1])
required = {'audience-view.json', 'typed-reference-view.json', 'actual-agent-capture.json',
            'actual-message-view-0.json', 'actual-message-view-1.json'}
assert required <= {path.name for path in directory.glob('*.json')}
subprocess.run([sys.executable, str(root / 'scripts/validate-gate6-record-view.py'), str(directory)], check=True)
for index in range(2):
    view = json.loads((directory / f'actual-message-view-{index}.json').read_text())
    messages = [item for item in view['analysis'] if item['type'] == 'message']
    assert len(messages) == (1 if index == 0 else 0)
    outcomes = [item['body'] for item in view['analysis'] if item['type'] == 'session-outcome']
    assert len(outcomes) == 1 and outcomes[0]['recorded_audio'] is False
    assert outcomes[0]['reason'] == 'failed' and outcomes[0]['capture_complete'] is False
agent = json.loads((directory / 'actual-agent-capture.json').read_text())
transcripts = [item['body']['content'] for item in agent['analysis'] if item['type'] == 'agent-observation'
               and item['body']['content']['kind'] == 'transcript'
               and item['body']['content'].get('text') == 'same-utterance']
assert len(transcripts) == 2 and [item['completeness'] for item in transcripts] == ['partial', 'final']
assert all(item['speaker_role'] == 'user' for item in transcripts)
assert 'forged-source' not in json.dumps(agent) and 'call-mock-1' not in json.dumps(agent)
print(json.dumps({'passed': True, 'required_actual_exports': sorted(required),
                  'scope': 'Actual local engine exports; schema/outcome/privacy facts, not provider delivery or host authorization.'}))
