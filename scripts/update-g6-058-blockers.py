#!/usr/bin/env python3
"""Render the current blocker ledger without changing recorded matrix verdicts."""
import argparse
import html
import json
import re
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
    tasks = {item["id"]: item for item in data.get("workplan_tasks", [])}
    workplan_rows = []
    for task_id in data.get("current_workplan_tasks", []):
        task = tasks[task_id]
        workplan_rows.append("<tr>" + "".join("<td>" + esc(value) + "</td>" for value in (
            task_id + " · " + task["title"], task["status"], task["completion"],
        )) + "</tr>")
    workplan = ('<h3 id="current-fix-workplan">Current fix batch</h3>'
        + '<table><thead><tr><th>Task</th><th>Status</th><th>Required verification</th></tr></thead>'
        + '<tbody>' + "".join(workplan_rows) + '</tbody></table>') if workplan_rows else ""
    component_rows = []
    for item in data.get("current_component_verification", []):
        component_rows.append(
            '<tr><td>' + esc(item["name"]) + '</td><td>'
            + esc(item["passed"]) + ' / ' + esc(item["planned"])
            + ' passed</td><td><a href="../evidence/gate-6/'
            + esc(item["evidence"]) + '">Verification evidence</a></td></tr>'
        )
    components = ('<h3>Targeted component verification</h3>'
        + '<p>These are separate component suites. The final combined batch and '
        + '32-cell qualification remain separate requirements.</p>'
        + '<table><thead><tr><th>Scope</th><th>Result</th><th>Evidence</th></tr></thead>'
        + '<tbody>' + "".join(component_rows) + '</tbody></table>') if component_rows else ""
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
        + components + workplan
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
    latest = data.get("latest_full_run")
    if latest:
        evidence = data[latest["evidence_key"]]
        summary = evidence["summary"]
        cells, checks = summary["cells"], summary["checks"]
        overview = ('<p id="next-full-run-status"><a href="'
            + esc(latest["status_url"]) + '">Completed ' + esc(latest["label"])
            + ' run — all 32 cell results</a>. Full-cell qualification is '
            + esc(cells["passed"]) + ' of ' + esc(summary["required_cells"])
            + '. ' + esc(cells["failed"]) + ' cells failed and '
            + esc(cells["blocked"]) + ' were blocked. Individual requirement checks: '
            + esc(checks["passed"]) + ' passed, ' + esc(checks["failed"])
            + ' failed and ' + esc(checks["blocked"]) + ' blocked.')
        focused = data.get("current_focused_verification")
        if focused:
            count = esc(focused["passed"])
            if focused.get("planned") is not None:
                count += ' of ' + esc(focused["planned"])
            overview += (' Focused follow-through checks: ' + count
                + ' passed. ' + esc(focused["next"])
                + ' These checks do not qualify matrix cells.')
        next_run = data.get("next_full_run_status")
        if next_run:
            overview += (' <a href="' + esc(next_run["url"]) + '">'
                + esc(next_run["label"]) + '</a>.')
        overview += '</p>'
        result, replaced = re.subn(r'<p id="next-full-run-status">.*?</p>',
            lambda _: overview, result, count=1, flags=re.DOTALL)
        assert replaced == 1, "Missing current matrix overview"
    if args.check:
        if result != current:
            raise SystemExit("Blocker tracker is stale; run scripts/update-g6-058-blockers.py")
    else:
        PAGE.write_text(result)


if __name__ == "__main__":
    main()
