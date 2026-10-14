#!/usr/bin/env python3
"""Run a source-linked, account-free connector contribution exercise.

Adapters implement connect_connector_api::conformance::Fixture and expose a
--contract-report <fresh-fixture-file> binary. No pass/fail input file is accepted.
Raw build output and provider fixture state stay in ignored .local/.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANDATORY = {
    "manifest-valid", "configuration-valid", "missing-secret-rejected", "raw-secret-rejected",
    "incompatible-manifest-rejected", "unsupported-video-rejected", "duplicate-registration",
    "restart-manifest-compatibility", "setup-ready", "attributable-context-reply", "message-restart",
    "message-logical-effect-once", "mutation-conflict-rejected", "independently-initiated-message",
    "uncertain-message", "uncertain-restart", "uncertain-no-repeat", "uncertain-message-reconcile", "reconciled-message-no-repeat", "signed-callback",
    "invalid-callback-auth", "tampered-callback", "final-before-acceptance", "duplicate-callback",
    "revoked-replay", "reordered-no-regression", "terminal-race-reconcile", "callback-restart",
    "voice-start-active", "duplex-generated-decoded-audio", "media-frame-bound", "voice-restart",
    "paired-teardown", "uncertain-voice-create", "unknown-voice-restart", "unknown-voice-no-repeat",
    "unknown-voice-reconcile", "cancel-uncertain-creation", "failure-after-acceptance",
    "peer-loss-bounded-cleanup",
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes(package, public_root=ROOT):
    paths = sorted(p for p in package.rglob("*") if p.is_file()
                   and (p.suffix == ".rs" or p.name in ("Cargo.toml", "Cargo.lock"))
                   and "target" not in p.parts)
    paths += sorted((public_root / "crates/connect-connector-api/src").glob("*.rs"))
    paths += [public_root / "crates/connect-connector-api/Cargo.toml", public_root / "docs/CONNECTORS.md",
              public_root / "contracts/connectors/profile-v1.md", public_root / "contracts/connectors/host-context-v1.md"]
    return {**{str(p.relative_to(public_root)) if p.is_relative_to(public_root) else str(p.relative_to(package)): digest(p)
            for p in paths}, "scripts/check-connector-contract.py": digest(Path(__file__).resolve())}


def export_kit(destination):
    if destination.exists():
        raise ValueError("export destination already exists; use a fresh isolated directory")
    api = destination / "crates/connect-connector-api"
    shutil.copytree(ROOT / "crates/connect-connector-api/src", api / "src")
    shutil.copytree(ROOT / "crates/connect-connector-api/tests", api / "tests")
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    package = workspace["package"]
    (api / "Cargo.toml").write_text(f'''[package]
name = "connect-connector-api"
version = "{package['version']}"
edition = "{package['edition']}"
rust-version = "{package['rust-version']}"
license = "{package['license']}"
[workspace]
[dependencies]
async-trait = "0.1"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
thiserror = "2"
[dev-dependencies]
tokio = {{ version = "1", features = ["full"] }}
[lints.rust]
unsafe_code = "forbid"
''')
    for relative in ["docs/CONNECTORS.md", "contracts/connectors/profile-v1.md", "contracts/connectors/host-context-v1.md", "LICENSE"]:
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, target)
    files = {str(p.relative_to(destination)): digest(p) for p in destination.rglob("*") if p.is_file()}
    (destination / "kit-source.json").write_text(json.dumps({"profile": "connect-connector-fixture-v1", "source_files": files,
        "export": "public API + guide + contract only; no store/service/routing implementation"}, indent=2) + "\n")
    print(json.dumps({"exported": str(destination), "files": len(files)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest-path", type=Path, default=ROOT / "examples/connector-template/Cargo.toml")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--export-kit", type=Path)
    parser.add_argument("--public-root", type=Path, default=ROOT, help="Selected public API/guide artifact root")
    parser.add_argument("--target-dir", type=Path, default=ROOT / ".local/gate5/connector-template-target", help="Prebuilt owned target; no hidden second compilation target")
    parser.add_argument("--work-root", type=Path, default=ROOT / ".local/gate5/connector-harness", help="Private isolated fixture/log directory")
    args = parser.parse_args()
    if args.export_kit:
        export_kit(args.export_kit.resolve())
        return 0
    manifest = args.manifest_path.resolve()
    scratch = args.work_root.resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="case-", dir=scratch) as directory:
        state = Path(directory) / "provider.json"
        command = ["cargo", "run", "--offline", "--locked", "--manifest-path", str(manifest),
                   "--target-dir", str(args.target_dir.resolve()),
                   "--", "--contract-report", str(state)]
        try:
            result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            print("Connector contract process exceeded its bounded 300 second offline budget")
            return 1
        # Build diagnostics may contain private paths; retain them locally only.
        (scratch / "latest-build.log").write_text(result.stderr)
        try:
            report = json.loads(result.stdout)
        except json.JSONDecodeError:
            print("Connector binary did not produce a JSON contract report. See .local/gate5/connector-harness/latest-build.log")
            return 1
        checks = report.get("checks", [])
        names = [c.get("name") for c in checks]
        valid = (report.get("profile") == "connect-connector-fixture-v1" and len(names) == len(set(names))
                 and MANDATORY.issubset(names) and all(c.get("passed") is True for c in checks)
                 and report.get("unsupported") == ["video", "live-provider"] and result.returncode == 0)
        redacted_command = [part.replace(str(ROOT) + "/", "") for part in command[:-1]]
        report.update({"passed": valid, "source_files": source_hashes(manifest.parent, args.public_root.resolve()),
                       "command": [*redacted_command, "<fresh-isolated-fixture-file>"],
                       "scope": "deterministic offline fixture; no provider accounts, calls, network ingress or live evidence",
                       "missing_mandatory": sorted(MANDATORY - set(names))})
        output = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(output)
        print(output)
        return 0 if valid else 1


if __name__ == "__main__":
    raise SystemExit(main())
