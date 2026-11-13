"""Independent late native controls, after proven original programme drain."""
import subprocess
import os
import signal
from .native_conversation_visibility import require_exact_conversation_visibility

FAILURE_LIMIT = 32
CONTROL_ERRORS = (AssertionError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError)
LATE_MARKER = "revoked-primary-device-refuses-while-independent-replacement-and-other-devices-remain-usable"

class OriginalControlOwnerUnjoined(RuntimeError):
    """A newly created original CLI owner has no confirmed terminal join."""


def terminate_original_control_child(pid, *, namespace, env):
    """A refused fallback kill must remain an unconfirmed original owner."""
    if namespace is not None:
        namespace.client_forced = True
    try:
        os.killpg(pid, signal.SIGKILL)
    except PermissionError:
        subprocess.run(["sudo", "-n", "/bin/kill", "-KILL", "--", "-" + str(pid)],
            env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=5, check=True)
    except ProcessLookupError:
        pass


def wait_original_control_child(child, terminate, report, phase):
    """Retain the existing 30s/5s bounds and distinguish failed owner cleanup."""
    custody = report.setdefault("public_control_owner_cleanup", dict(
        confirmed=True, owners_started=0, owners_joined=0))
    custody["owners_started"] += 1
    custody["confirmed"] = False
    joined = False
    try:
        result = child.wait(timeout=30)
        joined = True
        return result
    finally:
        if not joined:
            cleanup_error = None
            try:
                terminate()
            except Exception as error:
                cleanup_error = error
            # Even failed termination must attempt the same original child wait.
            try:
                child.wait(timeout=5)
                joined = True
            except Exception as error:
                cleanup_error = cleanup_error or error
            if cleanup_error is not None:
                custody.setdefault("unconfirmed_owner", dict(original_pid=child.pid,
                    phase=phase, cleanup_error=type(cleanup_error).__name__))
                raise OriginalControlOwnerUnjoined("Original public command cleanup is unconfirmed") from cleanup_error
        if joined:
            custody["owners_joined"] += 1
            custody["confirmed"] = (custody["owners_started"] == custody["owners_joined"]
                and "unconfirmed_owner" not in custody)



def record_native_failure(report, phase, error=None, *, line=None):
    row = dict(phase=phase, name=type(error).__name__ if error is not None else "AssertionError")
    if line is not None:
        row["line"] = line
    report.setdefault("failure", row)
    failures = report.setdefault("additional_failures", [])
    if len(failures) < FAILURE_LIMIT:
        failures.append(row)
    else:
        report["additional_failure_overflow"] = report.get("additional_failure_overflow", 0) + 1
    report["passed"] = False



def merge_native_failures(report, deferred):
    """Adjudicate already recorded late controls after ordinary media verdicts."""
    for original in deferred.get("additional_failures", []):
        row = dict(original)
        report.setdefault("failure", row)
        failures = report.setdefault("additional_failures", [])
        if len(failures) < FAILURE_LIMIT:
            failures.append(row)
        else:
            report["additional_failure_overflow"] = report.get("additional_failure_overflow", 0) + 1
        report["passed"] = False
    if deferred.get("additional_failure_overflow", 0):
        report["additional_failure_overflow"] = report.get("additional_failure_overflow", 0) + deferred["additional_failure_overflow"]


def ordinary_native_check(report, phase, condition):
    if condition is not True:
        record_native_failure(report, phase)
        return False
    return True


def original_native_owners_joined(report, media, cycles):
    """Reuse actual original Worker/retirement witnesses; quality is independent."""
    from .transport_matrix_verdict import native_cycle_join_verified
    control = report.get("conversation_control", {})
    if control.get("all_observed_helpers_joined") is not True:
        return False
    if not media:
        return True
    try:
        if cycles == 3:
            controls = report["native_cycle_controls"]
            resources = report["idle_resources"]["cycles"]
            receipts = control["resource_cycles"]
            return (len(controls) == len(resources) == len(receipts) == 3
                and all(c["index"] == r["index"] == receipt["index"] == index
                    and c["programme_worker"] == receipt["programme_worker"] == r["original_worker_receipt"]
                    and native_cycle_join_verified(c, r)
                    for index, (c, r, receipt) in enumerate(zip(controls, resources, receipts), 1)))
        return (cycles == 1 and control.get("programme_worker") == report["idle_resources"]["retirement"].get("programme_worker")
            and native_cycle_join_verified(control, dict(original_programme_owner=report["idle_resources"].get("programme_owner"))))
    except (KeyError, TypeError, ValueError):
        return False


def late_controls_not_run(reason):
    return dict(attempted=False, status="not-run", reason=reason,
        all_required_calls_attempted=False, steps=[])


def run_late_native_controls(report, *, owners_joined, owners, expected,
        before_read, revoke_primary, primary_refused, after_read, configs_unchanged):
    if owners_joined is not True:
        return late_controls_not_run("original-programme-owners-not-joined")
    owners = sorted(owners)
    if not owners or not expected:
        return late_controls_not_run("original-create-lineage-or-independent-owners-unavailable")
    steps, before = [], {}
    progress = dict(attempted=True, status="incomplete", all_required_calls_attempted=False, steps=steps)
    report["late_native_controls"] = progress

    def attempt(phase, call, validate):
        try:
            value = call()
            if validate(value) is not True:
                raise AssertionError("Original independent native control failed")
        except OriginalControlOwnerUnjoined:
            steps.append(dict(phase=phase, status="owner-unconfirmed"))
            progress["reason"] = "original-public-command-owner-unconfirmed"
            raise
        except CONTROL_ERRORS as error:
            steps.append(dict(phase=phase, status="failed", name=type(error).__name__))
            record_native_failure(report, phase, error)
            return False, None
        steps.append(dict(phase=phase, status="passed"))
        return True, value

    def exact(owner, rows):
        require_exact_conversation_visibility({owner: rows}, expected, {owner})
        return True

    for owner in owners:
        ok, rows = attempt(owner + "-before-revocation", lambda owner=owner: before_read(owner),
            lambda rows, owner=owner: exact(owner, rows))
        if ok:
            before[owner] = rows
    all_before = len(before) == len(owners)
    revoked, _ = attempt("revoke-primary-device", revoke_primary, lambda value: value is True)
    refused, _ = attempt("revoked-primary-native-refusal", primary_refused, lambda value: value is True)
    all_after = True
    for owner in owners:
        def unchanged(rows, owner=owner):
            exact(owner, rows)
            return owner in before and rows == before[owner]
        ok, _ = attempt(owner + "-after-revocation", lambda owner=owner: after_read(owner), unchanged)
        all_after &= ok
    unchanged_configs, _ = attempt("original-actor-configs-unchanged", configs_unchanged, lambda value: value is True)
    passed = all_before and revoked and refused and all_after and unchanged_configs
    return dict(attempted=True, status="passed" if passed else "failed",
        all_required_calls_attempted=len(steps) == 2 * len(owners) + 3,
        all_before_exact=all_before, primary_revoked=revoked, revoked_primary_refused=refused,
        all_after_same=all_after, configs_unchanged=unchanged_configs, steps=steps)
