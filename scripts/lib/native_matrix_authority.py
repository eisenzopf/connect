"""Isolated native current-authority cases, independent of the AV verdict.

Only production public issuance/revocation and the public Node SDK are used.
Every intent and original operation is saved before submission. Expiry is the
issued credential's actual wall-clock boundary, never a rewritten config date.
"""
import json
import re
from pathlib import Path
import subprocess
import sys
import time
import tomllib
import uuid

from .native_matrix_process import NativeOwner
from .transport_actors import OPERATIONS, exact, installation_files
from .transport_fixture import publish, toml_bytes
from .transport_matrix import digest


def authority_verdict(value, kind):
    fresh, live = value.get("fresh_original_refusal", {}), value.get("live_original_refusal", {})
    return (value.get("executed") is True and value.get("kind") == kind
        and value.get("selected_transport") == "quic" and value.get("mutation_submissions") == 1
        and value.get("current_original_read_before_boundary") is True
        and value.get("original_reference_preserved") is True
        and isinstance(value.get("original_reference_sha256"), str)
        and re.fullmatch(r"[0-9a-f]{64}", value.get("original_reference_sha256", "")) is not None
        and value.get("original_owners_joined") is True and value.get("original_helpers_joined") is True
        and value.get("opened_client_owners") == value.get("closed_client_owners") == 2
        and type(value.get("observed_helpers")) is int and value["observed_helpers"] >= 1
        and fresh.get("kind") == "authentication" and fresh.get("certainty") in {"read-only", "refused", "not-submitted"}
        and live.get("kind") in {"authentication", "cancelled", "transport-uncertain", "application"}
        and live.get("certainty") in {"read-only", "refused"}
        and (live.get("kind") != "application" or live.get("certainty") == "refused"
             and live.get("problem", {}).get("code") in {401, 403})
        and value.get("original_expiry_elapsed" if kind == "expiry" else "original_credential_revoked") is True
        and value.get("original_config_unchanged") is True and value.get("executable_unchanged") is True
        and value.get("original_issuance_unchanged") is True
        and value.get("process_owner", {}).get("original_supervisor_joined") is True
        and value.get("process_owner", {}).get("original_workers_joined") is True
        and value.get("process_owner", {}).get("forced") is False
        and value.get("process_owner", {}).get("exit_code") == 0)


def run_native_authority(*, root, directory, instance, admin, binary, env, execute,
                         source_revision, signing_algorithm, network_profile, wrap=None, client_platform=None,
                         owner_factory=NativeOwner, client_executable=None, client_binary_sha256=None):
    directory = Path(directory)
    directory.mkdir(mode=0o700)
    results = {}
    client_platform = client_platform or ("macos-arm64" if sys.platform == "darwin" else "ubuntu24-arm64")
    server_binary_hash = digest(binary)
    if (client_executable is None) != (client_binary_sha256 is None):
        raise ValueError("Remote native executable requires its original digest")
    selected_executable = binary if client_executable is None else client_executable
    binary_hash = server_binary_hash if client_binary_sha256 is None else client_binary_sha256
    for kind in ("expiry", "revocation"):
        case = directory / kind
        case.mkdir(mode=0o700)
        owner = None
        value = dict(passed=False, executed=False, launcher_attempted=False, kind=kind)
        issued_before = None
        try:
            # Remote source/config staging must happen before the short-lived
            # credential is issued. This hook creates no actor or operation.
            prepare = getattr(owner_factory, "prepare", None)
            if prepare is not None:
                prepare(case)
            # Separate synthetic identities keep the main four-participant
            # conversation inventory unchanged, even if this AV cell fails.
            token = uuid.uuid4().hex
            plan = dict(identity_operation_id="matrix-authority-identity-" + token,
                credential_operation_id="matrix-authority-credential-" + token,
                operation_id="matrix-authority-original-" + token,
                revoke_operation_id="matrix-authority-revoke-" + token,
                alias="matrix-native-authority-" + token)
            exact(case / "plan.json", (json.dumps(plan, sort_keys=True) + "\n").encode())
            identity = json.loads(execute("native-authority-identity-" + kind,
                ["provision", "--config", admin], dict(type="identity", operation_id=plan["identity_operation_id"],
                    alias=plan["alias"], kind="human", display_name="Synthetic native authority probe")))["result"]["Ok"]
            expires = int(time.time()) + (12 if kind == "expiry" else 120)
            intent = dict(operation_id=plan["credential_operation_id"], identity_id=identity["identity_id"],
                operations=OPERATIONS, recipients=[], expires_at=expires)
            exact(case / "issue-intent.json", (json.dumps(intent, sort_keys=True) + "\n").encode())
            installation = case / "installation"
            execute("native-authority-issue-" + kind, ["issue", "--config", admin,
                "--native-template", Path(instance) / "operator.toml", "--request", case / "issue-intent.json",
                "--output", installation, "--signing-algorithm", signing_algorithm])
            issued = json.loads((installation / "issued.json").read_text())
            native = tomllib.loads((installation / "client.toml").read_text())
            if (issued["expires_at"] != expires or issued["operation_id"] != plan["credential_operation_id"]
                    or native["signing_algorithm"] != signing_algorithm or native["server_algorithm"] != signing_algorithm):
                raise ValueError("Original native authority issuance differs")
            for key in ("trust_certificate", "signing_private_key", "server_public_key"):
                native[key] = str(installation / native[key])
            application = tomllib.loads((installation / "application.toml").read_text())
            publish(case / "sdk.toml", toml_bytes(dict(tenant_id="tnt_default", control=native, application=application)))
            issued_before = installation_files(installation)
            publish(case / "input.json", (json.dumps(dict(format="connect/native-matrix-authority-input/1",
                kind=kind, config=str(case / "sdk.toml"), executable=str(selected_executable), binary_sha256=binary_hash,
                source_revision=source_revision, signing_algorithm=signing_algorithm, network_profile=network_profile,
                operation_id=plan["operation_id"], revoke_operation_id=plan["revoke_operation_id"], expires_at=expires)) + "\n").encode())
            owner = owner_factory(["node", "scripts/check-transport-native-authority.mjs", case / "input.json", case / "observation.json"],
                case / "owner", case / "process.log", cwd=root, env=env, wrap=wrap)
            value["launcher_attempted"] = True
            cutoff = time.monotonic() + 45
            revoked = False
            while owner.poll() is None:
                if not owner.alive():
                    raise ValueError("Original authority owner exited without completion")
                request = case / "revoke-request.json"
                if request.exists() and not revoked:
                    if kind != "revocation" or request.is_symlink() or request.stat().st_size > 1024:
                        raise ValueError("Original revocation request bound")
                    if json.loads(request.read_text()) != dict(kind="revoke", operation_id=plan["revoke_operation_id"]):
                        raise ValueError("Original revocation request changed")
                    revoked = True  # No retry with another intent on uncertainty.
                    response = json.loads(execute("native-authority-revoke", ["revoke", "--config", admin,
                        "--operation-id", plan["revoke_operation_id"], issued["credential_id"]]))["result"]["Ok"]
                    publish(case / "revoke-response.json", json.dumps(dict(revoked=response["revoked"])).encode())
                if time.monotonic() >= cutoff:
                    raise ValueError("Original native authority deadline")
                time.sleep(.02)
            path = case / "observation.json"
            if path.is_symlink() or path.stat().st_size > 16384:
                raise ValueError("Native authority observation bound")
            value = json.loads(path.read_text())
            if (value.get("source_revision") != source_revision or value.get("binary_sha256") != binary_hash
                    or value.get("signing_algorithm") != signing_algorithm or value.get("network_profile") != network_profile
                    or value.get("host", {}).get("platform") != client_platform):
                raise ValueError("Native authority observation changed source or cohort")
            value["original_issuance_unchanged"] = installation_files(installation) == issued_before
            value["server_binary_sha256"] = server_binary_hash
            value["report_sha256"] = digest(path)
        except (OSError, ValueError, KeyError, AssertionError, subprocess.SubprocessError) as error:
            value.update(passed=False, failure=dict(name=type(error).__name__))
        finally:
            if owner is not None:
                try:
                    value["process_owner"] = owner.close()
                except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                    value["process_owner"] = dict(original_supervisor_joined=False, original_workers_joined=False,
                        forced=True, failure=type(error).__name__)
            value["passed"] = value.get("passed") is True and authority_verdict(value, kind)
            results[kind] = value
    (directory / "authority-observation.json").write_text(json.dumps(results, indent=2) + "\n")
    return results
