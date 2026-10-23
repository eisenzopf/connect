#!/usr/bin/env python3
"""Check Gate 6 task/acceptance/doc integrity; this does not qualify behavior."""
import collections
import html
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    source = (ROOT / 'docs/connect-gate-6-implementation.html').read_text()
    plan = json.loads(re.search(r'<script id="plan-data" type="application/json">(.*?)</script>', source, re.S)[1])
    tasks = {task['id']: task for task in plan['tasks']}
    assert len(tasks) == len(plan['tasks']) == 105
    assert set(tasks) == {f'G6-{number:03}' for number in range(1, 106)}
    active, visited = set(), set()

    def visit(key):
        assert key not in active, f'Dependency cycle: {key}'
        if key in visited:
            return
        active.add(key)
        for dependency in tasks[key]['depends']:
            assert dependency in tasks, (key, dependency)
            visit(dependency)
        active.remove(key)
        visited.add(key)

    for task in tasks.values():
        visit(task['id'])
        if task['status'] == 'done':
            assert task['owner'].strip() and task['evidence'].strip(), task['id']
            assert all(tasks[key]['status'] == 'done' for key in task['depends']), task['id']
            for reference in task['evidence'].split(';'):
                assert (ROOT / reference.strip()).exists(), (task['id'], reference)
        card = re.search(r'<details class="task" id="' + task['id'] + r'"[^>]*>', source)[0]
        assert f'data-status="{task["status"]}"' in card, task['id']
        # The editor restores embedded data. A newer static card alone must
        # not silently lose current evidence/notes when the plan is opened.
        saved_card = re.search(r'<details class="task" id="' + task['id'] + r'".*?</details>', source, re.S)[0]
        for field in ('notes', 'evidence'):
            value = re.search(r'<textarea data-field="' + field + r'"[^>]*>(.*?)</textarea>', saved_card, re.S)[1]
            assert html.unescape(value) == task[field], (task['id'], field, 'static and embedded plan data differ')
    acceptance = json.loads((ROOT / 'docs/gate-6-acceptance.json').read_text())
    criteria = {item['id']: item for item in acceptance['acceptance']}
    assert len(criteria) == 28 and len(acceptance['requirements']) == 17
    for item in criteria.values():
        assert item['implementation_items']
        for key in item['implementation_items']:
            assert item['id'] in tasks[key]['acceptance'].split(), (item['id'], key)
        if item['status'] == 'done':
            assert item['evidence'], item['id']
            assert all(tasks[key]['status'] == 'done' for key in item['implementation_items']), item['id']
    for task in tasks.values():
        for key in task['acceptance'].split():
            assert task['id'] in criteria[key]['implementation_items'], (task['id'], key)
    for requirement in acceptance['requirements']:
        assert requirement['acceptance']
        for key in requirement['acceptance']:
            assert requirement['id'] in criteria[key]['requirements']
    roadmap = json.loads((ROOT / 'docs/roadmap.json').read_text())
    gate = next(item for item in roadmap['gates'] if item['id'] == 6)
    counts = collections.Counter(task['status'] for task in tasks.values())
    assert gate['completed_task_count'] == counts['done']
    assert gate['in_progress_task_count'] == counts['in-progress']
    assert f"{counts['done']}/105 tasks complete" in (ROOT / 'docs/index.html').read_text()
    print(json.dumps({'scope': 'Plan integrity only; no runtime qualification.', 'tasks': dict(counts),
                      'criteria': len(criteria), 'requirements': len(acceptance['requirements']),
                      'done_evidence_paths_exist': True, 'dependency_graph': 'acyclic'}))


if __name__ == '__main__':
    main()
