#!/usr/bin/env python3
"""Current reader admission and writer exclusion regressions on local ARM Linux."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import runpy
import subprocess
import time
import traceback

from lib.package_environment import fixture_environment

ROOT = Path(__file__).resolve().parents[1]
CASES = [
    "authority_fences::tests::resource_preparation::same_scope_read_admission_coexists_and_keeps_reserved_writer_exclusive",
    "authority_fences::tests::resource_preparation::concurrent_cold_readers_coalesce_only_the_current_owned_resource",
    "authority_fences::tests::resource_preparation::blocked_shared_reader_keeps_original_deadline_and_releases_partial_scopes",
    "authority_fences::tests::writer_intent_stops_new_readers_held_proof_reuses_without_reentry_and_other_identity_proceeds",
    "authority_fences::tests::cancellation_deadline_and_partial_acquisition_release_every_scope",
    "authority_fences::tests::actual_child_process_revocation_waits_for_parent_delivery_guard_and_admin_open_preserves_runtime",
    "authority_fences::tests::actual_child_death_releases_kernel_reader_without_persistent_phantom_authority",
    "authority_fences::tests::immediate_admission_refuses_queued_writer_and_releases_partial_scopes",
    "authority_fences::tests::immediate_admission_refuses_reader_map_contention_without_waiting",
    "authority_fences::tests::admissions::resource_description_returns_unlocked_only_after_the_final_owned_reader",
    "authority_fences::tests::admissions::cached_admission_preserves_queued_writer_and_releases_partial_read_scopes",
    "authority_fences::tests::resource_preparation::preparing_one_resource_keeps_unrelated_scope_admission_live_and_own_writer_excluded",
    "authority_fences::tests::resource_preparation::busy_registry_after_resource_preparation_refuses_and_releases_its_original_file_lock",
]
SNAPSHOT = "peer_authority::connection_media::snapshot::tests::"
OBSERVER_CASES = [
    SNAPSHOT + "observer_cache::changed_facts_with_covered_scopes_keep_one_fresh_admission",
    SNAPSHOT + "packet_sql_observers_are_read_only_and_release_after_saturation",
    SNAPSHOT + "observer_custody::original_observer_contention_refuses_without_substitution_and_releases_the_reader",
    SNAPSHOT + "observer_custody::an_earlier_observer_becoming_free_cannot_replace_the_original_snapshot_observer",
    SNAPSHOT + "observer_custody::original_observer_reacquisition_rechecks_external_data_and_schema_versions",
    SNAPSHOT + "observer_cache::cached_current_observer_needs_only_one_fresh_file_admission",
    SNAPSHOT + "observer_cache::another_observers_old_scope_names_cannot_cover_new_current_metadata",
    SNAPSHOT + "optimistic_metadata_uses_only_fresh_scope_coverage_after_a_version_change",
    SNAPSHOT + "packet_metadata_memo_invalidates_external_own_clock_connection_and_source_changes",
    SNAPSHOT + "packet_metadata_memo_contention_and_failed_capture_keep_no_cached_grant",
    SNAPSHOT + "erased_packet_projection_memo_skips_only_unchanged_facts_and_finishes_every_grant",
    SNAPSHOT + "compound_packet_projection_clock_edge_revalidates_under_original_interval_and_expiry",
    SNAPSHOT + "compound_packet_projection_releases_every_reader_when_final_construction_crosses_a_commit",
]


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--suite", choices=("file-readers", "sql-observers"), default="file-readers")
    args = parser.parse_args()
    cases = OBSERVER_CASES if args.suite == "sql-observers" else CASES
    selection = SNAPSHOT if args.suite == "sql-observers" else "authority_fences::tests::"
    assert platform.system() == "Linux" and platform.machine() == "aarch64"
    assert os.environ.get("CONNECT_GATE6_OWNED_LINUX_VM") == "connect-gate6"
    assert not os.environ.get("CI") and not os.environ.get("GITHUB_ACTIONS")
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    output = args.output.resolve()
    assert output.is_relative_to(ROOT / ".local")
    os.umask(0o077);output.mkdir(parents=True, exist_ok=False)
    env = fixture_environment();env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
    bounded = runpy.run_path(str(ROOT / "scripts/check-linux-package-build.py"))["run_bounded"]
    sources = runpy.run_path(str(ROOT / "scripts/check-gate6-linux-hosting.py"))["sources"]
    state = dict(format="connect/current-reader-admission/1", source_revision=subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), controller_pid=os.getpid(),
        stage="preparing", source_guard_active=False, passed=False, suite=args.suite, cases=cases, results=[],
        scope=("Current SQL observer saturation, exact observer custody, snapshot resealing and memo scope validity; no full media or idle-resource pass."
            if args.suite == "sql-observers" else
            "Current Store admission, resource ownership, writer exclusion and cross-process lifetime; no full media or idle-resource pass."))
    def save(): (output / "state.json").write_text(json.dumps(state, indent=2) + "\n")
    save();before=None;binary=None;started=time.monotonic()
    try:
        with (output / "prepare.log").open("x") as log:
            subprocess.run(["bash", "scripts/prepare-dependencies.sh"], cwd=ROOT, env=env,
                stdout=log, stderr=subprocess.STDOUT, timeout=180, check=True)
        before=sources();(output / "source-inputs.json").write_text(json.dumps(before, sort_keys=True) + "\n")
        state.update(stage="compiling", source_guard_active=True);save()
        log=output / "compile.log"
        code=bounded(["cargo", "test", "--release", "--locked", "-p", "connect-store", "--lib",
            "--no-run", "--message-format=json"], log, env, 3600)
        state["compile_exit_code"]=code;state["compile_log_sha256"]=digest(log);save()
        assert code == 0
        binaries=[]
        for line in log.read_text().splitlines():
            try: value=json.loads(line)
            except ValueError: continue
            if (value.get("reason")=="compiler-artifact" and value.get("executable")
                    and value["target"]["name"]=="connect_store" and value["profile"]["test"]):
                binaries.append(Path(value["executable"]))
        assert len(binaries)==1
        binary=binaries[0];state["binary_sha256"]=digest(binary)
        log=output / "test-list.log"
        code=bounded([str(binary), selection, "--list"], log, env, 10)
        state["test_list_exit_code"]=code;state["test_list_sha256"]=digest(log)
        inventory=log.read_text().splitlines()
        assert code == 0 and all(inventory.count(case + ": test") == 1 for case in cases), "All exact cases must be compiled"
        state.update(stage="regressions");save()
        for index, case in enumerate(cases):
            log=output / f"case-{index:02}.log"
            code=bounded([str(binary), case, "--exact", "--nocapture", "--test-threads=1"], log, env, 60)
            passed=code==0 and "1 passed; 0 failed; 0 ignored" in log.read_text()
            state["results"].append(dict(case=case, exit_code=code, passed=passed, log_sha256=digest(log)))
            save()
        state["passed"]=all(case["passed"] for case in state["results"]) and len(state["results"])==len(cases)
    except BaseException as error:
        (output / "failure.log").write_text(traceback.format_exc());state["failure"]=type(error).__name__
    finally:
        state["source_inputs_unchanged"]=before is not None and sources()==before
        state["executable_unchanged"]=binary is not None and digest(binary)==state["binary_sha256"]
        state["passed"] &= state["source_inputs_unchanged"] and state["executable_unchanged"]
        state.update(stage="complete", source_guard_active=False, elapsed_seconds=time.monotonic()-started);save()
    print(json.dumps(state), flush=True)
    return 0 if state["passed"] else 1


if __name__ == "__main__": raise SystemExit(main())
