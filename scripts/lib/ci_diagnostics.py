"""Small, redacted CI evidence bundles; raw fixture state is never uploaded."""
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

MAX_LOG_BYTES = 1024 * 1024
SCOPES = ("checks", "gate-3-checks", "gate-4-checks", "ci-workspace")


def redact(text):
    """Remove bearer/config credentials, including generated fixture secrets."""
    text = re.sub(r"(?i)(Bearer\s+)\S+", r"\1[REDACTED]", text)
    text = re.sub(r"ctg1\.[A-Za-z0-9_.-]+", "[REDACTED-GRANT]", text)
    text = re.sub(r"\b[0-9a-fA-F]{64}\b", "[REDACTED-64-HEX]", text)
    return re.sub(
        r'''(?im)((?:["']?(?:token|credential|secret|api[_-]?key|authorization)["']?)\s*[:=]\s*)("[^"\n]*"|'[^'\n]*'|[^\s,}]+)''',
        r"\1[REDACTED]", text)


def redact_value(value):
    """Keep JSON types/structure intact while sanitizing textual fields."""
    if isinstance(value, str):
        return redact(value)
    if isinstance(value, list):
        return [redact_value(item) for item in value]
    if isinstance(value, dict):
        return {key: redact_value(item) for key, item in value.items()}
    return value


def disk(root):
    usage = shutil.disk_usage(root)
    result = {"total_bytes": usage.total, "used_bytes": usage.used, "free_bytes": usage.free}
    for name in ("target", ".local/recovery-baselines"):
        path = root / name
        if path.exists():
            try:
                output = subprocess.check_output(["du", "-sk", str(path)], text=True, timeout=15)
                result[name + "_bytes"] = int(output.split()[0]) * 1024
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                result[name + "_measurement_error"] = type(error).__name__
    return result


def bundle(root, output):
    """Copy only summaries and bounded failed/active compiler/test log tails."""
    output.mkdir(parents=True, exist_ok=False)
    summary = {"scope": "account-free hosted source checks; redacted diagnostic tails",
               "disk": disk(root), "reports": []}
    for scope in SCOPES:
        paths = sorted((root / ".local" / scope).glob("*/result.json"), key=lambda p: p.stat().st_mtime)
        if not paths:
            continue
        report_path = paths[-1]
        try:
            report = json.loads(report_path.read_text())
            entry = {"scope": scope, "path": str(report_path.relative_to(root)),
                     "result": report.get("status", report.get("result")),
                     "source_revision": report.get("source_revision"),
                     "error": report.get("error"), "not_run": report.get("not_run"),
                     "steps": report.get("steps", [])}
            # No captured provider payloads, config files, prompts or fixture DBs.
            clean = redact_value(entry)
            (output / (scope + "-result.json")).write_text(json.dumps(clean, indent=2) + "\n")
            summary["reports"].append({"scope": scope, "result": entry["result"],
                                       "source_revision": entry["source_revision"],
                                       "error": clean["error"],
                                       "original_report_sha256": hashlib.sha256(report_path.read_bytes()).hexdigest()})
            selected = [step.get("log") for step in report.get("steps", [])
                        if step.get("status", step.get("result")) in ("failed", "running")]
            if report.get("active_phase"):
                selected.append(report["active_phase"] + ".log")
            for name in sorted(set(filter(None, selected))):
                path = report_path.parent / name
                if pathlib.Path(name).name != name or path.is_symlink() or not path.is_file():
                    continue
                with path.open("rb") as stream:
                    stream.seek(max(0, path.stat().st_size - MAX_LOG_BYTES))
                    text = redact(stream.read(MAX_LOG_BYTES).decode(errors="replace"))
                target = output / (scope + "-" + name)
                target.write_text(text)
                print(f"FAILED PHASE {scope}/{name}\n{text[-32 * 1024:]}", flush=True)
        except (OSError, ValueError, TypeError) as error:
            summary["reports"].append({"scope": scope, "diagnostic_error": str(error)})
    for path in sorted((root / ".local/recovery-baselines").glob("*/build.log")):
        if path.is_symlink() or not path.is_file() or not re.fullmatch(r"[0-9a-f]{40}", path.parent.name):
            continue
        with path.open("rb") as stream:
            stream.seek(max(0, path.stat().st_size - MAX_LOG_BYTES))
            tail = redact(stream.read(MAX_LOG_BYTES).decode(errors="replace"))
        (output / ("baseline-" + path.parent.name + ".log")).write_text(tail)
    summary["files_sha256"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                               for p in sorted(output.iterdir()) if p.is_file()}
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2), flush=True)
    return summary
