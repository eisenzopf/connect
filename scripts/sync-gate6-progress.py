#!/usr/bin/env python3
"""Refresh Gate 6 documentation progress without promoting acceptance to a pass."""
import collections
import datetime
import json
import pathlib
import re
from zoneinfo import ZoneInfo

ROOT = pathlib.Path(__file__).resolve().parents[1]


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    today = datetime.datetime.now(ZoneInfo('America/Phoenix')).date().isoformat()
    source = (ROOT / 'docs/connect-gate-6-implementation.html').read_text()
    plan = json.loads(re.search(r'<script id="plan-data" type="application/json">(.*?)</script>', source, re.S)[1])
    tasks = {item['id']: item for item in plan['tasks']}
    counts = collections.Counter(item['status'] for item in tasks.values())
    completed = counts['done']
    progress = f"{completed}/105 tasks complete, {counts['in-progress']} in progress; 28 acceptance criteria"
    acceptance_path = ROOT / 'docs/gate-6-acceptance.json'
    acceptance = json.loads(acceptance_path.read_text())
    for criterion in acceptance['acceptance']:
        items = [tasks[key] for key in criterion['implementation_items']]
        done = sum(item['status'] == 'done' for item in items)
        if criterion['status'] == 'not-started' and any(item['status'] != 'not-started' for item in items):
            criterion['status'] = 'in-progress'
        if criterion['status'] == 'in-progress':
            criterion['observation'] = f'{done}/{len(items)} mapped implementation tasks complete. Acceptance remains unqualified until its full required evidence passes.'
        criterion['implementation_evidence'] = sorted({
            part.strip() for item in items if item['status'] == 'done'
            for part in item['evidence'].split(';') if part.strip()
        })
    acceptance['updated'] = today
    save(acceptance_path, acceptance)
    roadmap_path = ROOT / 'docs/roadmap.json'
    roadmap = json.loads(roadmap_path.read_text())
    gate = next(item for item in roadmap['gates'] if item['id'] == 6)
    gate['completed_task_count'] = completed
    gate['in_progress_task_count'] = counts['in-progress']
    for milestone in gate['milestones']:
        items = [item for item in tasks.values() if item['phase'] == milestone['phase']]
        milestone['status'] = 'done' if all(item['status'] == 'done' for item in items) else ('in-progress' if any(item['status'] != 'not-started' for item in items) else 'not-started')
    roadmap['updated'] = today
    save(roadmap_path, roadmap)
    for relative in ('docs/index.html', 'docs/engineering/index.html'):
        path = ROOT / relative
        text = path.read_text()
        text = re.sub(r'implementation authorized · \d+/105 tasks', f'implementation in progress · {completed}/105 tasks complete', text)
        text = re.sub(r'implementation in progress · \d+/105 tasks complete', f'implementation in progress · {completed}/105 tasks complete', text)
        path.write_text(text)
    path = ROOT / 'docs/connect-roadmap.html'
    text = path.read_text()
    text = text.replace('<strong>105 not-started tasks and 28 acceptance criteria</strong>', f'<strong id="gate6-progress">{progress}</strong>')
    text = re.sub(r'(<strong id="gate6-progress">).*?(</strong>)', lambda m: m[1] + progress + m[2], text)
    text = text.replace('the UCTP planning inventory covers all 49 current envelope types and assigns the remaining normative audit.', 'the source audit covers 99 sections, 207 clause units and all 49 baseline envelopes; the target schema covers 65 envelope types. Runtime conformance remains open.')
    path.write_text(text)
    path = ROOT / 'docs/connect-gate-6-plan.html'
    text = path.read_text().replace('Gate 6 draft: complete', 'Gate 6 implementation: complete')
    text = text.replace('49 envelope types, 15 normative areas and 9 proposed semantic additions', '49 baseline envelope types, 99 source sections, 207 clause units and 65 target envelope types')
    text = text.replace('It is a planning inventory, not conformance evidence.', 'The audited inventory and schema vectors are recorded; runtime conformance remains unqualified.')
    text = text.replace('All criteria are <strong>not started</strong>.', 'Implementation is in progress; <strong>no acceptance criterion is yet qualified</strong>.')
    for criterion in acceptance['acceptance']:
        pattern = r'(<details id="' + criterion['id'] + r'">.*?</details>)'
        match = re.search(pattern, text, re.S)
        if match and criterion['status'] == 'in-progress':
            card = match[1].replace('<span class="status-draft">Not started</span>', '<span class="status-draft">In progress · unqualified</span>')
            text = text[:match.start()] + card + text[match.end():]
    path.write_text(text)
    print(json.dumps({'tasks': dict(counts), 'acceptance_qualified': sum(item['status'] == 'done' for item in acceptance['acceptance'])}))


if __name__ == '__main__':
    main()
