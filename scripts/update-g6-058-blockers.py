#!/usr/bin/env python3
"""Render the current blocker ledger without changing recorded matrix verdicts."""
import argparse
import hashlib
import html
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "docs/evidence/gate-6/g6-058-blockers.json"
PAGE = ROOT / "docs/developer/g6-058-matrix-status.html"
START = "<!-- G6-058-BLOCKERS-START -->"
END = "<!-- G6-058-BLOCKERS-END -->"


def latest_matrix_section(data):
    """Current adjudicated rows coexist with every unchanged historical table."""
    latest = data["latest_full_run"]
    evidence_root = (ROOT / "docs/evidence/gate-6").resolve()
    matrix_path = (PAGE.parent / latest["status_url"]).resolve()
    mapping_path = (evidence_root / data["latest_failure_mapping"]).resolve()
    assert matrix_path.is_relative_to(evidence_root) and mapping_path.is_relative_to(evidence_root)
    matrix = json.loads(matrix_path.read_text())
    mapping = json.loads(mapping_path.read_text())
    assert hashlib.sha256(matrix_path.read_bytes()).hexdigest() == mapping["source_evidence"]["sha256"]
    cells = matrix["cells"]
    assert len(cells) == 32 and len({cell["id"] for cell in cells}) == 32
    totals = {status: sum(row["status"] == status for cell in cells for row in cell["checks"].values())
              for status in ("passed", "failed", "blocked")}
    assert sum(totals.values()) == 432 and all(totals[k] == matrix["summary"]["checks"][k] for k in totals)
    rows = mapping["failure_mapping"]
    expected = {(cell["id"], name) for cell in cells for name, row in cell["checks"].items()
                if row["status"] in {"failed", "blocked"}}
    assert len(rows) == len(expected) and {(row["cell_id"], row["check_id"]) for row in rows} == expected
    esc = lambda value: html.escape(str(value), quote=True)
    rendered = []
    for cell in cells:
        counts = {status: sum(row["status"] == status for row in cell["checks"].values())
                  for status in ("passed", "failed", "blocked")}
        failures = [row for row in rows if row["cell_id"] == cell["id"]]
        detail = "<ul>" + "".join("<li><b>" + esc(row["status"]) + "</b> " + esc(row["check_id"])
            + " — " + esc(row["classification_role"]) + ": " + esc(row["evidence_summary"])
            + " Owner tasks: " + esc(", ".join(row["assigned_task_ids"])) + ". Causal fix confirmed: "
            + esc(row["causal_fix_confirmed"]) + ".</li>"
            for row in failures) + "</ul>"
        rendered.append("<tr><td>" + esc(cell["id"].removeprefix("G6-058/")) + "</td><td>"
            + esc(cell["status"]) + "</td><td>" + esc(counts["passed"]) + " / " + esc(counts["failed"])
            + " / " + esc(counts["blocked"]) + "</td><td><details><summary>Saved failure or missing prerequisite</summary>"
            + detail + "</details></td></tr>")
    summary = matrix["summary"]
    return ('<!-- G6-058-LATEST32-START --><section id="latest-adjudicated-matrix"><h2>Latest complete 32-cell run — '
        + esc(latest["label"]) + '</h2><p>' + esc(summary["cells"]["passed"]) + ' of 32 cells qualified. Requirement rows: '
        + esc(totals["passed"]) + ' passed, ' + esc(totals["failed"]) + ' failed, ' + esc(totals["blocked"]) + ' blocked. '
        + 'Original producer outcomes and requirement results are evaluated independently. '
        + 'Missing capture/control prerequisites are distinguished from measured quality failures. Targeted component verification does not establish a fix for every saved cell.</p>'
        + '<p><a href="' + esc(latest["status_url"]) + '">Adjudicated matrix</a> · '
        + '<a href="../evidence/gate-6/' + esc(data["latest_failure_mapping"]) + '">Exact '
        + esc(len(rows)) + ' current requirement mappings and numeric observations</a>. '
        + 'Historical mappings remain unchanged in the ledger.</p>'
        + '<table><thead><tr><th>Configuration</th><th>Cell verdict</th><th>Requirement pass / fail / blocked</th><th>Evidence and owner tasks</th></tr></thead><tbody>'
        + "".join(rendered) + '</tbody></table></section><!-- G6-058-LATEST32-END -->')


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
        diagnostic = data.get("v218_cohort56_diagnostic")
        if diagnostic:
            proof_path = (ROOT / diagnostic["evidence"]).resolve()
            assert proof_path.parent == (ROOT / "docs/evidence/gate-6").resolve()
            assert hashlib.sha256(proof_path.read_bytes()).hexdigest() == diagnostic["evidence_sha256"]
            overview += (' Postrun prerequisite attempt: ' + esc(diagnostic["cohort_passed"])
                + ' passed, ' + esc(diagnostic["cohort_failed"]) + ' failed. '
                + 'Both full native programmes passed. The expiry fixture was corrected and passed separately; '
                + 'the failed whole attempt remains recorded. <a href="../evidence/gate-6/'
                + esc(proof_path.name) + '">Current prerequisite evidence</a>. '
                + 'These results do not qualify matrix cells.')
        elif focused:
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
    latest_section = latest_matrix_section(data)
    marker_start, marker_end = "<!-- G6-058-LATEST32-START -->", "<!-- G6-058-LATEST32-END -->"
    if marker_start in result:
        assert result.count(marker_start) == result.count(marker_end) == 1
        before_latest, remainder = result.split(marker_start)
        _, after_latest = remainder.split(marker_end)
        result = before_latest + latest_section + after_latest
    else:
        result, inserted = re.subn(r'(<p id="next-full-run-status">.*?</p>)',
            lambda match: match.group(1) + latest_section, result, count=1, flags=re.DOTALL)
        assert inserted == 1, "Missing current overview before latest table"
    if args.check:
        if result != current:
            raise SystemExit("Blocker tracker is stale; run scripts/update-g6-058-blockers.py")
    else:
        PAGE.write_text(result)


if __name__ == "__main__":
    main()
