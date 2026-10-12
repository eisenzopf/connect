"""Run Cargo's workspace coverage without retaining every integration executable.

Only exact executable artifacts reported by the selected successful Cargo command
are retired. Shared libraries, ordinary application binaries and failed outputs
remain available. This is a hosted-CI storage strategy, not a release profile.
"""
import datetime
import json
import pathlib
import shutil
import subprocess

from .qualification_report import save

LIBRARIES = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}


def targets(metadata):
    """Select every workspace test, doctest and default compile-only example."""
    members = set(metadata["workspace_members"])
    selected = []
    for package in metadata["packages"]:
        if package["id"] not in members:
            continue
        for target in package["targets"]:
            kinds = set(target["kind"])
            if kinds & LIBRARIES:
                if target["test"]:
                    selected.append((package["id"], package["name"], target, "test", ["--lib"]))
                if target["doctest"]:
                    selected.append((package["id"], package["name"], target, "doc", ["--doc"]))
            elif kinds in ({"bin"}, {"test"}, {"example"}):
                if target["test"]:
                    flag = {"bin": "--bin", "test": "--test", "example": "--example"}[next(iter(kinds))]
                    selected.append((package["id"], package["name"], target, "test", [flag, target["name"]]))
                elif kinds == {"example"}:
                    selected.append((package["id"], package["name"], target, "build", ["--example", target["name"]]))
            elif kinds != {"custom-build"}:
                raise ValueError(f"Unaccounted Cargo target: {package['name']} {target['name']} {kinds}")
    # Keep workspace-wide feature unification. Libraries and doctests form small
    # groups; named integration/bin/example selectors are unique in this graph.
    grouped = {}
    for target in selected:
        key = (target[3], tuple(target[4]))
        grouped.setdefault(key, []).append(target)
    return list(grouped.values())


def artifact(message, selected):
    return message.get("reason") == "compiler-artifact" and message.get("executable") and any(
        message.get("package_id") == package_id
        and message.get("target", {}).get("name") == target["name"]
        and message.get("target", {}).get("kind") == target["kind"]
        and bool(message.get("profile", {}).get("test")) == (mode == "test")
        for package_id, _, target, mode, _ in selected)


def retire(executable, target_directory, mode, kind=None):
    """Delete a completed test/example's generated files, never a normal bin."""
    path = pathlib.Path(executable)
    parent = target_directory / "debug" / ("examples" if kind == ["example"] else "deps")
    if path.is_symlink() or path.parent.resolve() != parent.resolve() or not path.is_file():
        raise ValueError(f"Unexpected Cargo executable location: {path}")
    size = path.stat().st_size
    path.unlink()
    removed = [str(path)]
    symbols = path.with_name(path.name + ".dSYM")
    if symbols.exists():
        if symbols.is_symlink() or not symbols.is_dir():
            raise ValueError(f"Unexpected Cargo symbol directory: {symbols}")
        size += sum(p.stat().st_size for p in symbols.rglob("*") if p.is_file())
        shutil.rmtree(symbols)
        removed.append(str(symbols))
    return {"paths": removed, "bytes": size}


def snapshot(root):
    usage = shutil.disk_usage(root)
    return {"free_bytes": usage.free, "used_bytes": usage.used, "total_bytes": usage.total}


def run(root, directory, all_features=False, minimum_free_bytes=4 * 1024 ** 3):
    """Preserve target failures/output; retain only a bounded set of executables."""
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=root))
    plan = targets(metadata)
    target_directory = pathlib.Path(metadata["target_directory"])
    report = {"status": "running", "scope": "explicit CI workspace coverage",
              "all_features": all_features, "target_directory": str(target_directory),
              "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "expected_targets": [{"package": p, "name": t["name"], "kind": t["kind"], "mode": m}
                                   for group in plan for _, p, t, m, _ in group], "steps": []}
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    save(directory, report)
    try:
        for selected in plan:
            _, _, target, mode, flags = selected[0]
            if any(t.get("required-features") for _, _, t, _, _ in selected) and not all_features:
                raise ValueError("Feature-gated CI coverage requires explicit --all-features")
            disk = snapshot(root)
            if disk["free_bytes"] < minimum_free_bytes:
                raise RuntimeError(f"Insufficient CI disk headroom: {disk['free_bytes']} bytes free")
            command = ["cargo", "build" if mode == "build" else "test", "--locked",
                       "--workspace", *flags, "--message-format=json-render-diagnostics"]
            if all_features:
                command.append("--all-features")
            if mode == "test":
                command += ["--", "--test-threads=1"]
            step = {"targets": [{"package": p, "target": t["name"]} for _, p, t, _, _ in selected], "mode": mode,
                    "command": command, "disk_before": disk, "status": "running"}
            report["steps"].append(step)
            save(directory, report)
            print(f"CI TARGET workspace {mode} {flags}", flush=True)
            executables = {}
            with subprocess.Popen(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, bufsize=1) as process:
                for line in process.stdout:
                    try:
                        message = json.loads(line)
                    except json.JSONDecodeError:
                        print(line, end="", flush=True)
                        continue
                    if not isinstance(message, dict) or not message.get("reason"):
                        print(line, end="", flush=True)
                    elif artifact(message, selected):
                        executables[message["executable"]] = message["target"]["kind"]
                    elif message.get("reason") == "compiler-message":
                        print(message.get("message", {}).get("rendered", line), end="", flush=True)
                step["exit_code"] = process.wait()
            step["status"] = "passed" if step["exit_code"] == 0 else "failed"
            step["disk_after_command"] = snapshot(root)
            save(directory, report)
            if step["status"] != "passed":
                raise RuntimeError(f"Cargo target failed: {flags}; outputs retained")
            if mode != "doc" and len(executables) != len(selected):
                raise ValueError(f"Expected {len(selected)} Cargo executables, observed {len(executables)}")
            step["retired"] = [retire(p, target_directory, mode, kind) for p, kind in sorted(executables.items())]
            step["disk_after_retirement"] = snapshot(root)
            save(directory, report)
        report["status"] = "passed"
    except BaseException as error:
        report.update(status="failed", error={"type": type(error).__name__, "detail": str(error)})
        raise
    finally:
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save(directory, report)
    return report
