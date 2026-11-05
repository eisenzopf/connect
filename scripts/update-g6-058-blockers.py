#!/usr/bin/env python3
"""Render the current blocker ledger without changing recorded matrix verdicts."""
import argparse
import html
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "docs/evidence/gate-6/g6-058-blockers.json"
PAGE = ROOT / "docs/developer/g6-058-matrix-status.html"
START = "<!-- G6-058-BLOCKERS-START -->"
END = "<!-- G6-058-BLOCKERS-END -->"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = json.loads(DATA.read_text())
    esc = lambda value: html.escape(str(value), quote=True)
    rows = []
    ids = set()
    for item in data["blockers"]:
        assert item["id"] not in ids
        ids.add(item["id"])
        rows.append("<tr>" + "".join("<td>" + esc(value) + "</td>" for value in (
            item["id"] + " · " + item["title"], item["status"],
            item["change"], item["evidence"], item["next"],
        )) + "</tr>")
    section = (START + '<section id="blocker-progress"><h2>Current blocker progress</h2>'
        + '<p class="note">' + esc(data["run_status"]) + '</p>'
        + '<p>Updated ' + esc(data["updated"]) + '. Implemented fixes stay open until their stated runtime checks pass. '
        + 'Component verification does not change the 32-cell matrix totals. '
        + '<a href="../evidence/gate-6/g6-058-blockers.json">Blocker ledger</a></p>'
        + '<table><thead><tr><th>Blocker</th><th>Status</th><th>Change</th><th>Evidence / commit</th><th>Next verification</th></tr></thead>'
        + '<tbody>' + "".join(rows) + '</tbody></table></section>' + END)
    current = PAGE.read_text()
    if START in current:
        assert current.count(START) == current.count(END) == 1
        before, remainder = current.split(START)
        _, after = remainder.split(END)
        result = before + section + after
    else:
        anchor = '<h1>G6-058 transport matrix</h1>'
        assert current.count(anchor) == 1
        result = current.replace(anchor, anchor + "\n" + section, 1)
    if args.check:
        if result != current:
            raise SystemExit("Blocker tracker is stale; run scripts/update-g6-058-blockers.py")
    else:
        PAGE.write_text(result)


if __name__ == "__main__":
    main()
