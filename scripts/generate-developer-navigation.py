#!/usr/bin/env python3
"""Render the shared developer navigation, vocabulary and reference index."""
import argparse
import html
import json
import os
from pathlib import Path
import re
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / 'docs'


def relative(page, href):
    path, separator, fragment = href.partition('#')
    target = (DOCS / path).resolve()
    assert target.is_relative_to(ROOT), href
    assert target.is_file(), href
    if separator:
        assert target.suffix == '.html', href
        assert re.search(r'\bid=["\']' + re.escape(fragment) + r'["\']', target.read_text()), href
    return os.path.relpath(target, page.parent) + (separator + fragment if separator else '')


def block(source, name, body):
    start, end = f'<!-- CONNECT:{name}:START -->', f'<!-- CONNECT:{name}:END -->'
    pattern = re.escape(start) + r'.*?' + re.escape(end)
    assert len(re.findall(pattern, source, re.S)) == 1, name
    return re.sub(pattern, lambda _: start + '\n' + body + '\n' + end, source, flags=re.S)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    model = json.loads((DOCS / 'navigation.json').read_text())
    assert model['profile'] == 'connect-developer-navigation/1'
    ids = [row['id'] for row in model['routes']]
    assert len(ids) == len(set(ids))
    assert {'install', 'first-message', 'voice', 'handoff', 'hosting', 'reference', 'contribute'} <= set(ids)
    pages = [DOCS / path for path in ['index.html', 'start/index.html', 'concepts/index.html',
        'hosting/index.html', 'tutorials/parley.html', 'reference/index.html', 'engineering/index.html']]
    changed = []
    for page in pages + [ROOT / 'README.md', DOCS / 'README.md']:
        source = page.read_text()
        if page.suffix == '.html':
            links = '\n'.join(f'<li><a href="{html.escape(relative(page, row["href"]))}">{html.escape(row["label"])}</a></li>' for row in model['routes'])
            rendered = '<nav class="dev-nav" aria-label="Developer documentation"><div class="dev-shell"><ul>\n' + links + '\n</ul></div></nav>'
        else:
            rendered = ' · '.join(f'[{row["label"]}]({relative(page, row["href"])})' for row in model['routes'])
        output = block(source, 'NAV', rendered)
        if page == DOCS / 'concepts/index.html':
            rows = '\n'.join(f'<tr><th scope="row">{html.escape(row["name"])}</th><td>{html.escape(row["meaning"])}</td></tr>' for row in model['terms'])
            output = block(output, 'TERMS', '<table><caption>Shared Connect terminology</caption><tbody>\n' + rows + '\n</tbody></table>')
        if page == DOCS / 'reference/index.html':
            rows = '\n'.join(f'<li><a href="{html.escape(relative(page, row["href"]))}">{html.escape(row["label"])}</a> — {html.escape(row["scope"])}</li>' for row in model['references'])
            output = block(output, 'REFERENCES', '<ul>\n' + rows + '\n</ul>')
        if output != source:
            changed.append(str(page.relative_to(ROOT)))
            if not args.check:
                page.write_text(output)
    # Only inspect public documentation targets; no network or private files.
    links = 0
    for page in pages:
        for href in re.findall(r'\bhref="([^"]+)"', page.read_text()):
            value = urlsplit(html.unescape(href))
            if value.scheme or value.netloc:
                continue
            target = (page.parent / value.path).resolve() if value.path else page
            assert target.is_relative_to(ROOT) and target.is_file(), (page.relative_to(ROOT), href)
            if value.fragment and target.suffix == '.html':
                assert re.search(r'\bid=["\']' + re.escape(value.fragment) + r'["\']', target.read_text()), (page.relative_to(ROOT), href)
            links += 1
    print(json.dumps({'check': args.check, 'routes': len(ids), 'html_pages': len(pages),
        'local_links_checked': links, 'changed': changed, 'scope': 'Local navigation/target integrity; no runtime or external-link qualification.'}))
    if args.check and changed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
