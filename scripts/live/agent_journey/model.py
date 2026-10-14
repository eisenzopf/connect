"""Actual installed model turns and transport observation."""
import collections
import json
from pathlib import Path
import re
import sys
import time
from .common import (TURN_SECONDS, MAX_LOG, require, read_json, sha256, save,
                     write_private, ps_metadata)


def followup_invocation(calls, event_id):
    """Select the actual successful invocation, never infer it from model prose."""
    matches = [call for call in calls if call.get("tool") == "connect_task_send"
               and call.get("arguments", {}).get("reply_to") == event_id]
    require(len(matches) == 1, "one-actual-tracked-followup-call")
    call = matches[0]
    require(call.get("status") == "completed" and not call.get("error")
            and not call.get("result", {}).get("is_error")
            and not call.get("result", {}).get("isError"), "successful-followup-call")
    return {"arguments": call["arguments"],
            "structured": call["result"]["structured_content"]}


class Model:
    def model_turn(self, name, initial):
        self.phase = name
        shim, record = self.directory / (name + "-codex-exec"), self.directory / (name + "-child.json")
        shim_text = f"""#!{sys.executable}
import json,os,sys
args=sys.argv[1:]
if args == ['--version'] or args == ['login','status']:
    os.execv({str(self.codex)!r},[{str(self.codex)!r},*args])
mode='resume' if len(args)>1 and args[0:2]==['exec','resume'] else 'first'
value={{'pid':os.getpid(),'ppid':os.getppid(),'pgid':os.getpgrp(),'mode':mode,'session_id':args[2] if mode=='resume' else None,'normal_credential_present':bool(os.environ.get('CONNECT_AGENT_CREDENTIAL')),'task_grant_present':'CONNECT_TASK_GRANT' in os.environ,'provider_secret_present':any(name in os.environ for name in ('TELNYX_API_KEY','VAPI_PRIVATE_KEY','CONNECT_OPERATOR_CREDENTIAL'))}}
with open({str(record)!r},'x') as stream:json.dump(value,stream)
os.execv({str(self.codex)!r},[{str(self.codex)!r},*args])
"""
        write_private(shim, shim_text)
        shim.chmod(0o700)
        args = ["agent-access", "run-codex", "--installation", self.installation,
                "--codex-binary", shim, "--timeout-seconds", str(TURN_SECONDS)]
        if initial:
            args.extend(["--prompt-file", self.directory / "first-prompt.txt"])
        started = time.monotonic()
        result = json.loads(self.command(name, args, TURN_SECONDS + 20, model=True, child_record=record))
        require(result.get("status") == "host-turn-completed", "actual-host-turn-completed")
        require(result.get("access_mode") == "agent-credential", "actual-regular-host-mode")
        require(result.get("identity_id") == self.agent_id, "actual-selected-host-identity")
        child = read_json(record, 4096)
        require(child["mode"] == ("first" if initial else "resume"), "explicit-host-exec-mode")
        require(child["normal_credential_present"] and not child["task_grant_present"]
                and not child["provider_secret_present"], "protected-normal-only-host-environment")
        require(ps_metadata(child["pid"]) is None, "actual-host-leader-reaped")
        state = read_json(self.installation / "host-state.json")
        require(state.get("profile") == "connect-codex-agent-host/1" and state.get("completed") is True,
                "persisted-normal-host-completion")
        require(state.get("session_id") == result.get("session_id"), "saved-exact-session")
        event_path = self.installation / f"host-{state['run_id']}-{state['attempts']}.jsonl"
        require(event_path.stat().st_size <= MAX_LOG, "bounded-actual-host-events")
        events = [json.loads(line) for line in event_path.read_text().splitlines()]
        self.review_local_preparation(name, events)
        calls = [e["item"] for e in events if e.get("type") == "item.completed"
                 and e.get("item", {}).get("type") == "mcp_tool_call"]
        successful = [c for c in calls if c.get("status") == "completed" and c.get("error") is None
                      and not c.get("result", {}).get("is_error")
                      and not c.get("result", {}).get("isError")]
        failed_count = len(calls) - len(successful)
        counts = collections.Counter(c["tool"] for c in successful)
        tool_observation = {
            "calls_passed": len(successful), "calls_failed": failed_count,
            "successful_tools": dict(sorted(counts.items())),
            "event_sha256": sha256(event_path),
            "rule": "Model/schema/transport errors are retained and counted as failures; a corrected later call does not turn that failure into a pass.",
        }
        self.report.setdefault("model_tool_calls", {})[name] = tool_observation
        save(self.directory / (name + "-tool-calls.json"), tool_observation)
        require(failed_count == 0, "model-or-schema-tool-error-retained-not-a-pass")
        require(successful and all(c.get("server") == "connect_agent" for c in successful),
                "actual-ordinary-mcp-tool-calls")
        required = ({"connect_inspect", "connect_context_view", "connect_artifact_begin",
                     "connect_artifact_append", "connect_artifact_finish", "connect_task_create",
                     "connect_task_send", "connect_task_wait"} if initial
                    else {"connect_inbox", "connect_task_wait", "connect_task_send", "connect_send",
                          "connect_task_consume_reply"})
        require(required <= set(counts), "actual-required-tools-executed")
        if initial:
            inspection = next(c["result"]["structured_content"] for c in successful
                              if c["tool"] == "connect_inspect")
            require(inspection["kind"] == "identity_access"
                    and inspection["data"]["identity_id"] == self.agent_id
                    and inspection["data"]["credential_id"] == result["credential_id"]
                    and "admin" not in inspection["data"]["operations"]
                    and "*" not in inspection["data"]["operations"], "non-admin-current-identity-projection")
        if not initial:
            self.model_followup_invocation = followup_invocation(
                calls, self.reply["data"]["reply"]["event_id"])
            require(child["session_id"] == self.first_turn["session_id"] == result["session_id"],
                    "explicit-exact-saved-host-session-resume")
        observation = {"session_id": result["session_id"], "run_id": result["run_id"],
                       "seconds": round(time.monotonic() - started, 3),
                       "tools": dict(sorted(counts.items())), "failed_tool_calls": failed_count,
                       "event_sha256": sha256(event_path), "exec_shim_sha256": sha256(shim),
                       "owned_host_leader_reaped": True}
        save(self.directory / (name + "-observation.json"), observation)
        self.assert_installation_unchanged()
        return observation


    def review_local_preparation(self, name, events):
        """Retain actual local actions and fail obvious transport/key shortcuts.

        This is evidence screening, not a sandbox or a semantic proof of arbitrary
        shell code. The full private command record remains available for review.
        """
        items = [event["item"] for event in events
                 if event.get("type") == "item.completed" and isinstance(event.get("item"), dict)]
        commands = [item for item in items if item.get("type") == "command_execution"]
        changes = [item for item in items if item.get("type") == "file_change"]
        record = {"commands": [{"command": item.get("command"), "status": item.get("status"),
                                 "exit_code": item.get("exit_code")} for item in commands],
                  "file_changes": [{"status": item.get("status"), "changes": item.get("changes")}
                                   for item in changes],
                  "scope": "Actual Codex's own local skill reading/draft/digest/base64 preparation only. Every Connect operation must remain an installed MCP call.",
                  "screening": "Closed path checks and prohibited transport/secret-read markers; full command text is retained privately for independent review. This is not a shell sandbox."}
        save(self.directory / (name + "-local-preparation.json"), record)
        failed = sum(item.get("status") != "completed" or item.get("exit_code") != 0 for item in commands)
        failed += sum(item.get("status") != "completed" for item in changes)
        self.report.setdefault("local_artifact_preparation", {})[name] = {
            "commands_passed": len(commands) - sum(item.get("status") != "completed" or item.get("exit_code") != 0 for item in commands),
            "commands_or_changes_failed": failed, "file_change_items": len(changes),
            "private_record_sha256": sha256(self.directory / (name + "-local-preparation.json")),
            "screening_limit": "Text screening is not a sandbox. Review the retained private commands/draft scripts before publishing evidence.",
        }
        require(failed == 0, "failed-local-model-command-retained-not-a-pass")
        forbidden = re.compile(
            r"(?:https?://|\b(?:curl|wget|telnet|ssh|urllib|requests|httpx|socket|sqlite3|NativeClient|connect_sdk|Popen|subprocess)\b|"
            r"\b(?:getenv|environ)\b|os\.(?:system|exec|popen)|\.\.[/\\]|\.parent\b|"
            r"(?:operator|agent|model|human)\.toml|CODEX_HOME|CODEX_API_KEY|CONNECT_AGENT_CREDENTIAL|"
            r"TELNYX_API_KEY|VAPI_PRIVATE_KEY|(?:^|[\n;&|])\s*(?:[^\s]+/)?connect\s)",
            re.IGNORECASE,
        )
        for item in commands:
            command = item.get("command")
            require(isinstance(command, str) and not forbidden.search(command),
                    "local-model-command-is-not-transport-or-secret-access")
        draft_root = self.installation / "draft-work"
        for item in changes:
            for change in item.get("changes", []):
                path = Path(change["path"])
                resolved = path.resolve() if path.is_absolute() else (self.installation / path).resolve()
                require(resolved.is_relative_to(draft_root), "local-artifact-change-path")
        files = [path for path in draft_root.rglob("*") if path.is_file()] if draft_root.exists() else []
        require(len(files) <= 16, "bounded-local-artifact-preparation-files")
        observations = []
        for path in files:
            require(not path.is_symlink() and path.stat().st_size <= 32768, "bounded-local-artifact-file")
            if path.suffix in (".py", ".sh", ".js", ".mjs"):
                require(not forbidden.search(path.read_text()), "local-draft-script-is-not-transport-or-secret-access")
            observations.append({"path": str(path.relative_to(self.installation)), "sha256": sha256(path),
                                 "size": path.stat().st_size})
        self.report["local_artifact_preparation"][name]["files"] = observations


