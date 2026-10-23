"""Explicit task negotiation under backing credentials or narrow Native grants."""
import re
from .errors import ConnectError
from .task_types import TASK_PROFILE, TaskCommand, TaskOutcome


class TaskOperations:
    _task_profile = False

    def _observe_outcome(self, outcome):
        if outcome["kind"] == "capabilities":
            self._task_profile = TASK_PROFILE in outcome["data"].get("extensions", [])
        return outcome

    def _require_task_profile(self, operation, task_binding=None):
        if task_binding is not None:
            request = operation.get("request", {})
            command = request.get("command", {})
            if (
                not isinstance(task_binding, str)
                or not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", task_binding)
                or operation.get("op") != "task"
                or request.get("task_profile") != TASK_PROFILE
                or command != {"action": "get", "task_id": task_binding}
            ):
                raise ConnectError("invalid-request", "Task bootstrap must read its exact bound task")
            return
        if operation.get("op") == "task" and not self._task_profile:
            raise ConnectError("unsupported-profile", "Discover connect-task/1 through capabilities before task requests")

    def negotiate_tasks(self, address: str, conversation_id: str | None = None) -> None:
        """Discovery never expands current credential, task or recipient authority."""
        outcome = self.request({"op": "capabilities", "address": address, "conversation_id": conversation_id})
        self._observe_outcome(outcome)
        if not self._task_profile:
            raise ConnectError("unsupported-profile", "Server does not advertise connect-task/1")

    def negotiate_task(self, task_id: str):
        """Read one explicit task; no resolve permission or grant issuance is needed."""
        operation = {"op": "task", "request": {"task_profile": TASK_PROFILE, "command": {"action": "get", "task_id": task_id}}}
        outcome = self.request(operation, task_binding=task_id)
        if outcome.get("kind") != "task" or outcome["data"].get("kind") != "record" or outcome["data"]["data"].get("id") != task_id:
            raise ConnectError("protocol", "Task bootstrap returned a different task")
        self._task_profile = True
        return outcome["data"]["data"]

    def task(self, command: TaskCommand) -> TaskOutcome:
        """Keep IDs/inputs across uncertainty. Pending is not completion."""
        operation = {"op": "task", "request": {"task_profile": TASK_PROFILE, "command": command}}
        self._require_task_profile(operation)
        return self.request(operation)["data"]


def exchange_timeout(base, operation):
    """Bound the additional server wait without extending other exchanges."""
    if operation.get("op") in ("conversation_events","session_observations"):
        wait=operation.get("query",{}).get("wait_ms",0)
        return base+(min(wait,30000)/1000 if type(wait) is int and wait>=0 else 0)
    if operation.get("op") != "task":
        return base
    command = operation.get("request", {}).get("command", {})
    wait = command.get("wait_ms", 0) if command.get("action") in ("inbox", "wait") else 0
    return base + (min(wait, 10000) / 1000 if type(wait) is int and wait >= 0 else 0)
