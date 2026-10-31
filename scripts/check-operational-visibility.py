#!/usr/bin/env python3
"""Bounded synthetic G6-057 component checks; raw logs/state stay private."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import time
from lib.package_environment import fixture_environment

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".local/gate6-operational"
EVIDENCE = BASE / "evidence"
PRIVATE = BASE / "private"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def cacheable_steps(steps):
    # A later compile/runtime failure must not discard already completed Cargo
    # outputs. This grants cache reuse only; Cargo still validates/rebuilds every
    # incomplete target and no behavior result is promoted to a pass.
    return any(step.get("exit_code") in (0, 101) and not step.get("compiler_errors")
        and any(re.fullmatch("[0-9a-f]{64}", item.get("sha256", ""))
            for item in step.get("executables", [])) for step in steps)


def generate_reference():
    spec = importlib.util.spec_from_file_location("application_reference", ROOT / "scripts/generate-application-http-openapi.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    generated = EVIDENCE / "generated"
    generated.mkdir(parents=True, exist_ok=True)
    module.TARGET = generated / "openapi-application-http.json"
    module.CATALOG = generated / "application-http-dispatch.json"
    module.main()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("schema", "runtime", "typecheck"), required=True)
    parser.add_argument("--case", choices=("store-authority", "service-observation", "permit-lifetime", "mcp-capacity", "public-client", "hosting-limits", "operation-overload"))
    parser.add_argument("--aggregate", action="store_true")
    parser.add_argument("--generate-reference", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    os.umask(0o077)
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    PRIVATE.mkdir(parents=True, exist_ok=True)
    if args.generate_reference:
        # Generator owns its own CLI; don't forward the component runner's flags.
        import sys
        sys.argv = [str(ROOT / "scripts/generate-application-http-openapi.py")]
        generate_reference()
        return
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    before = sources()
    assert not args.case or args.phase == "runtime"
    assert not args.aggregate or (args.phase == "runtime" and not args.case)
    key = args.phase + ("-" + args.case if args.case else "")
    source_path = EVIDENCE / ("source-inputs-" + key + ".json")
    source_path.write_text(json.dumps(before, sort_keys=True) + "\n")
    report = dict(format="connect/gate6-operational-component/1", phase=args.phase, selected_case=args.case,
        source_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=digest(source_path), source_count=len(before), steps=[], passed=False,
        whole_gate_qualified=False, task_complete=False,
        scope="Local synthetic operational component only; no provider/model/media or whole G6-057 qualification.")
    env = fixture_environment()
    env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    prefix = ["cargo", "test", "--release", "--locked", "--message-format=json"]
    commands = [
        ("protocol-shape", prefix + ["-p", "connect-protocol", "--lib", "operational::tests", "--", "--test-threads=1"], 1200, 1),
        ("application-reference", ["python3", "scripts/check-operational-visibility.py", "--phase", "schema", "--generate-reference"], 1200, None),
    ] if args.phase == "schema" else [
        ("store-authority", prefix + ["-p", "connect-store", "--lib", "operational::tests", "--", "--test-threads=1"], 1800, 2),
        ("service-observation", prefix + ["-p", "connect-service", "--lib", "readiness::tests", "--", "--test-threads=1"], 3600, 3),
        ("permit-lifetime", prefix + ["-p", "connect-service", "--lib", "resources::tests", "--", "--test-threads=1"], 1200, 2),
        ("mcp-capacity", prefix + ["-p", "connect-mcp", "--lib", "capacity::tests", "--", "--test-threads=1"], 1800, 2),
        ("public-client", prefix + ["-p", "connect-installation-conformance", "--test", "gate6_operational_visibility", "--", "--test-threads=1"], 3600, 3),
        ("hosting-limits", prefix + ["-p", "connect-installation-conformance", "--test", "gate6_hosting_limits", "--", "--test-threads=1"], 1800, 2),
        ("operation-overload", prefix + ["-p", "connect-store", "--lib", "transitions::tests::shared_capacity_keeps_room_for_a_terminal_cancel_decision_and_its_causal_event", "--", "--test-threads=1"], 1200, 1),
    ]
    if args.phase == "typecheck":
        commands = [("integration-fixture-types", ["cargo", "check", "--release", "--locked", "--message-format=json",
            "-p", "connect-installation-conformance", "--test", "gate6_operational_visibility",
            "--test", "gate6_hosting_limits"], 1200, None)]
        report["scope"] = "Integration fixture type checking only; no tests executed or runtime qualification."
    if args.case:
        commands = [c for c in commands if c[0] == args.case]
        assert len(commands) == 1
    try:
        if args.aggregate:
            report["cases"] = []
            for name, _, _, _ in commands:
                path = EVIDENCE / ("runtime-" + name + ".json")
                if not path.is_file():
                    report["cases"].append(dict(name=name, status="not-run"))
                    continue
                value = json.loads(path.read_text())
                inputs = EVIDENCE / ("source-inputs-runtime-" + name + ".json")
                assert (value["source_revision"] == report["source_revision"]
                    and value["phase"] == "runtime" and value["selected_case"] == name
                    and value["source_inputs_sha256"] == digest(inputs) == report["source_inputs_sha256"]
                    and value["source_inputs_unchanged"] and len(value["steps"]) == 1
                    and value["steps"][0]["name"] == name), "Component source/case differs"
                report["cases"].append(dict(name=name, report_sha256=digest(path), passed=value["passed"]))
                report["steps"].extend(value["steps"])
            report["passed"] = len(report["steps"]) == len(commands) and all(c.get("passed") for c in report["cases"])
        for name, command, budget, expected in ([] if args.aggregate else commands):
            log = PRIVATE / (name + ".log")
            start = time.monotonic()
            status = bounded(command, log, env, budget)
            lines = log.read_text().splitlines()
            step = dict(name=name, exit_code=status, elapsed_seconds=round(time.monotonic() - start, 3),
                test_results=[line for line in lines if line.startswith("test result:")],
                failed_tests=[line for line in lines if line.startswith("test ") and line.endswith(" ... FAILED")],
                compiler_errors=[], executables=[], failure_locations=[])
            for line in lines:
                location = re.search(r"panicked at ([^\s]+\.rs):(\d+):(\d+)", line)
                if location:
                    step["failure_locations"].append(dict(file=location[1], line=int(location[2]), column=int(location[3])))
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if event.get("reason") == "compiler-message" and event["message"]["level"] == "error":
                    msg = event["message"]
                    step["compiler_errors"].append(dict(code=(msg.get("code") or {}).get("code"), message=msg["message"],
                        locations=[dict(file=s["file_name"], line=s["line_start"]) for s in msg["spans"] if s["is_primary"]]))
                if event.get("reason") == "compiler-artifact" and event.get("executable") and event.get("profile", {}).get("test"):
                    step["executables"].append(dict(name=event["target"]["name"], sha256=digest(Path(event["executable"]))))
            report["steps"].append(step)
            print(json.dumps(step), flush=True)
            if status != 0:
                raise RuntimeError("Bounded operational component failed; inspect private diagnostics")
            if expected is not None:
                assert len(step["test_results"]) == 1 and re.fullmatch(
                    r"test result: ok\. " + str(expected) + r" passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out; finished in .+",
                    step["test_results"][0]), "Expected behavior cases did not execute"
                assert step["executables"] and not step["compiler_errors"]
        if not args.aggregate:
            report["passed"] = True
        if args.phase == "schema":
            report["generated"] = {p.name: digest(p) for p in (EVIDENCE / "generated").iterdir()}
    finally:
        report["source_inputs_unchanged"] = before == sources()
        report["passed"] = report["passed"] and report["source_inputs_unchanged"]
        report["cacheable_build"] = args.phase == "runtime" and report["source_inputs_unchanged"] and not args.case and cacheable_steps(report["steps"])
        report["cache_scope"] = "Completed Cargo artifacts from unchanged sources only; missing/failed compilation remains unqualified and Cargo must rebuild it."
        (EVIDENCE / (key + ".json")).write_text(json.dumps(report, indent=2) + "\n")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.write("cacheable_build=" + str(report["cacheable_build"]).lower() + "\n")
    assert report["passed"]


if __name__ == "__main__":
    main()
