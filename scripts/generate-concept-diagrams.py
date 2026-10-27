#!/usr/bin/env python3
"""Render contract-traced concept figures/examples; optionally validate wire JSON."""
import argparse
import copy
import hashlib
import html
import json
import os
from pathlib import Path
import re
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
DIRECTORY = ROOT / 'docs/concepts'
PAGE = DIRECTORY / 'index.html'
MODEL = DIRECTORY / 'lifecycle.json'


def escape(value):
    return html.escape(str(value))


def heading_slug(value):
    return re.sub(r'[^\w\- ]', '', value.lower()).replace(' ', '-')


def source_link(source):
    path = (ROOT / source['path']).resolve()
    assert path.is_relative_to(ROOT) and path.is_file(), source
    headings = re.findall(r'^#{1,6} (.+?)\s*$', path.read_text(), re.M)
    assert source['heading'] in headings, source
    href = os.path.relpath(path, DIRECTORY) + '#' + heading_slug(source['heading'])
    return f'<a href="{escape(href)}">{escape(path.name)} / {escape(source["heading"])}</a>'


def sources(values):
    return ' · '.join(source_link(value) for value in values)


def svg_start(figure, height):
    return [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 {height}" role="img" aria-labelledby="title description">',
        f'<title id="title">{escape(figure["title"])}</title><desc id="description">{escape(figure["description"])}</desc>',
        '<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10Z" fill="#62ddff"/></marker></defs>',
        '<style>text{fill:#edf5ff;font:17px system-ui,sans-serif}rect{fill:#102033;stroke:#62ddff;stroke-width:1.5}.edge{stroke:#62ddff;stroke-width:2;fill:none;marker-end:url(#arrow)}.life{stroke:#58728d;stroke-dasharray:5 5}.note{fill:#b5c5d8;font-size:15px}</style>',
        f'<path d="M0 0H960V{height}H0Z" fill="#07111d"/>']


def render(figure):
    if figure['kind'] == 'states':
        result = svg_start(figure, 350)
        nodes = {node['id']: node for node in figure['nodes']}
        for start, end in figure['edges']:
            a, b = nodes[start], nodes[end]
            if a['y'] == b['y']:
                x1, y1, x2, y2 = a['x'] + 150, a['y'] + 27, b['x'], b['y'] + 27
            elif a['y'] < b['y']:
                offset = -10 if [end, start] in figure['edges'] else 0
                x1, y1, x2, y2 = a['x'] + 75 + offset, a['y'] + 54, b['x'] + 75 + offset, b['y']
                if a['x'] != b['x']:
                    x1, x2 = a['x'] + 125, b['x'] + 25
            else:
                offset = 10 if [end, start] in figure['edges'] else 0
                x1, y1, x2, y2 = a['x'] + 75 + offset, a['y'], b['x'] + 75 + offset, b['y'] + 54
                if a['x'] != b['x']:
                    x1, x2 = a['x'] + 125, b['x'] + 25
            result.append(f'<path class="edge" data-from="{start}" data-to="{end}" d="M{x1} {y1}L{x2} {y2}"/>')
        for node in nodes.values():
            result.append(f'<g data-state="{node["id"]}"><rect x="{node["x"]}" y="{node["y"]}" width="150" height="54" rx="8"/><text x="{node["x"] + 75}" y="{node["y"] + 33}" text-anchor="middle">{node["id"]}</text></g>')
        result.append('<text class="note" x="20" y="325">Selected legal edges. Safe retry and attributed reconciliation require current authority.</text>')
    elif figure['kind'] == 'sequence':
        height = 160 + len(figure['steps']) * 74
        result = svg_start(figure, height)
        for index, actor in enumerate(figure['actors']):
            x = 125 + index * 240
            result += [f'<rect x="{x - 104}" y="24" width="208" height="48" rx="7"/>',
                f'<text x="{x}" y="54" text-anchor="middle">{escape(actor)}</text>',
                f'<path class="life" d="M{x} 72V{height - 30}"/>']
        for index, step in enumerate(figure['steps']):
            x1, x2, y = 125 + step['from'] * 240, 125 + step['to'] * 240, 140 + index * 74
            style = ' stroke-dasharray="6 4"' if step.get('observation') else ''
            if x1 == x2:
                path = f'M{x1} {y}h60v22h-60'
                text = f'<text x="{x1 + 72}" y="{y - 10}">{escape(step["label"])}</text>'
            else:
                path = f'M{x1} {y}H{x2}'
                text = f'<text x="{(x1 + x2) // 2}" y="{y - 12}" text-anchor="middle">{escape(step["label"])}</text>'
            result += [text, f'<path class="edge"{style} d="{path}"/>']
    else:
        assert figure['kind'] == 'lifetime'
        result = svg_start(figure, 280)
        result += ['<text x="30" y="30">Persistent Conversation con_example</text>',
            '<rect x="25" y="48" width="910" height="50" rx="8"/>',
            '<text x="480" y="80" text-anchor="middle">Messages and authorized history continue across Session boundaries</text>',
            '<rect x="35" y="135" width="365" height="65" rx="8"/>',
            '<text x="217" y="161" text-anchor="middle">Session A</text><text x="217" y="187" text-anchor="middle" class="note">Invite → accept → actual media → end</text>',
            '<rect x="445" y="135" width="300" height="65" rx="8"/>',
            '<text x="595" y="161" text-anchor="middle">Session B</text><text x="595" y="187" text-anchor="middle" class="note">Independent live interaction</text>',
            '<text x="835" y="160" text-anchor="middle">Private</text><text x="835" y="186" text-anchor="middle">follow-up</text>',
            '<path class="edge" d="M35 232H930"/><text class="note" x="35" y="262">Illustrative time → ending voice is separate from an authorized Conversation close.</text>']
    return '\n'.join(result + ['</svg>', ''])


def replace_block(text, name, content):
    start, end = f'<!-- CONNECT:{name}:START -->', f'<!-- CONNECT:{name}:END -->'
    pattern = re.escape(start) + '.*?' + re.escape(end)
    assert len(re.findall(pattern, text, re.S)) == 1, name
    return re.sub(pattern, lambda _: start + '\n' + content + '\n' + end, text, flags=re.S)


def verify_example_links(examples):
    consult, handoff, move = (examples[name] for name in ('consult', 'handoff', 'move'))
    scope = lambda frame: (frame['cid'], frame['sid'])
    for name in ('return', 'handoff', 'move', 'move_status', 'move_failed', 'move_unknown'):
        assert scope(examples[name]) == scope(consult), (name, 'Conversation/Session changed')
    for name in ('return', 'handoff', 'move'):
        assert examples[name]['payload']['retained_connid'] == consult['payload']['retained_connid'], (name, 'remote changed')
    assert handoff['connid'] == consult['connid'], 'handoff primary changed'
    assert move['connid'] == handoff['payload']['candidate_connid'], 'move source must remain the handoff target Connection'
    assert {handoff['payload']['source_participant'], handoff['payload']['target_participant']} <= set(consult['payload']['private_participants'])
    assert examples['move_status']['payload']['operation_id'] == move['operation_id']
    for name in ('move_failed', 'move_unknown'):
        observed = examples[name]
        assert observed['connid'] == move['connid'] and observed['operation_id'] == move['operation_id']
        assert observed['payload']['operation_id'] == move['operation_id']
    assert examples['follow_up']['cid'] == consult['cid']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--validate', action='store_true', help='Also validate examples using docs/requirements.txt')
    args = parser.parse_args()
    data = json.loads(MODEL.read_text())
    assert data['profile'] == 'connect-concepts/1'
    schema = json.loads((ROOT / data['schema']).read_text())
    vocabulary = json.loads((ROOT / 'docs/navigation.json').read_text())
    assert set(data['term_sources']) == {item['name'] for item in vocabulary['terms']}
    states = set(schema['$defs']['OperationState']['enum'])
    allowed = set()
    state_pattern = r'\b(' + '|'.join(sorted(states)) + r')\b'
    for start, end in re.findall(r'^\| ([^|]+) \| ([^|]+) \|$', (ROOT / 'contracts/connect-operations.md').read_text(), re.M):
        starts = re.findall(state_pattern, start)
        ends = re.findall(state_pattern, end)
        allowed.update((a, b) for a in starts for b in ends)
    for refused in [('unknown', 'preparing'), ('completed', 'ready'), ('failed', 'unknown')]:
        assert refused not in allowed
    examples = {item['id']: item['frame'] for item in data['examples']}
    assert len(examples) == len(data['examples'])
    verify_example_links(examples)
    text = PAGE.read_text()
    changed = []
    for figure in data['figures']:
        if figure['kind'] == 'states':
            assert {node['id'] for node in figure['nodes']} == states
            assert all(tuple(edge) in allowed for edge in figure['edges'])
        for command in figure.get('commands', []):
            assert command in schema['properties']['type']['enum']
        for step in figure.get('steps', []):
            assert 0 <= step['from'] < len(figure['actors']) and 0 <= step['to'] < len(figure['actors'])
            for example in ([step['example']] if 'example' in step else step.get('alternatives', [])):
                assert example in examples
        svg = render(figure)
        ET.fromstring(svg)
        path = DIRECTORY / 'diagrams' / (figure['id'] + '.svg')
        if not path.exists() or path.read_text() != svg:
            changed.append(str(path.relative_to(ROOT)))
            if not args.check:
                path.parent.mkdir(exist_ok=True)
                path.write_text(svg)
        lines = [f'<figure><div class="dev-diagram" tabindex="0" role="region" aria-label="{escape(figure["title"])}; scroll horizontally if needed"><img src="diagrams/{figure["id"]}.svg" alt="{escape(figure["description"])}"></div>',
            f'<figcaption>{escape(figure["description"])} Sources: {sources(figure["sources"])}.</figcaption></figure>']
        if figure.get('steps'):
            lines.append('<ol>' + ''.join(f'<li>{escape(step["label"])}</li>' for step in figure['steps']) + '</ol>')
        text = replace_block(text, 'FIGURE:' + figure['id'], '\n'.join(lines))
    for name, frame in examples.items():
        text = replace_block(text, 'EXAMPLE:' + name, '<pre><code>' + escape(json.dumps(frame, indent=2)) + '</code></pre>')
    traces = [('Term: ' + term, values) for term, values in data['term_sources'].items()]
    traces += [('Section: ' + section, values) for section, values in data['section_sources'].items()]
    for section in data['section_sources']:
        assert f'id="{section}"' in text, section
    rows = ''.join(f'<tr><th scope="row">{escape(name)}</th><td>{sources(values)}</td></tr>' for name, values in traces)
    text = replace_block(text, 'CONCEPT-SOURCES', '<table><caption>Contract trace for every shared term and concept section</caption><tbody>' + rows + '</tbody></table>')
    if text != PAGE.read_text():
        changed.append(str(PAGE.relative_to(ROOT)))
        if not args.check:
            PAGE.write_text(text)
    validation = 'not run; use --validate'
    if args.validate:
        from jsonschema import Draft202012Validator, FormatChecker
        from importlib.metadata import version
        Draft202012Validator.check_schema(schema)
        checker = FormatChecker()
        if 'date-time' not in checker.checkers:
            raise SystemExit('Date-time validation unavailable; install docs/requirements.txt in the selected documentation environment.')
        validator = Draft202012Validator(schema, format_checker=checker)
        for frame in examples.values():
            validator.validate(frame)
        negatives = []
        bad = copy.deepcopy(examples['consult']); bad['payload']['private_participants'] = ['prt_owner']; negatives.append(bad)
        bad = copy.deepcopy(examples['move']); bad['payload']['candidate_connid'] = 'conn_phone'; negatives.append(bad)
        bad = copy.deepcopy(examples['move']); del bad['expected_revision']; negatives.append(bad)
        bad = copy.deepcopy(examples['move_failed']); bad['payload']['result'] = {}; negatives.append(bad)
        bad = copy.deepcopy(examples['move_unknown']); bad['payload']['state'] = 'ringing'; negatives.append(bad)
        bad = copy.deepcopy(examples['follow_up']); bad['payload']['to'] = []; negatives.append(bad)
        bad = copy.deepcopy(examples['handoff']); del bad['payload']['role_scope']; negatives.append(bad)
        bad = copy.deepcopy(examples['consult']); bad['ts'] = 'not-a-time'; negatives.append(bad)
        assert all(not validator.is_valid(frame) for frame in negatives)
        validation = {'positive_examples': len(examples), 'malformed_examples_refused': len(negatives),
            'jsonschema': version('jsonschema'), 'date_time_checker_installed': True}
    paths = {MODEL, ROOT / data['schema'], ROOT / 'contracts/connect-operations.md'}
    for _, values in traces:
        paths.update(ROOT / value['path'] for value in values)
    for figure in data['figures']:
        paths.update(ROOT / value['path'] for value in figure['sources'])
    hashes = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(paths)}
    print(json.dumps({'check': args.check, 'figures': len(data['figures']), 'terms': len(data['term_sources']),
        'selected_operation_edges': len(next(f for f in data['figures'] if f['kind'] == 'states')['edges']),
        'synthetic_scope_and_lineage_checked': True,
        'schema_validation': validation, 'changed': changed, 'source_sha256': hashes,
        'scope': 'Documentation syntax, trace and diagram generation; no command/media/provider execution.'}))
    if args.check and changed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
