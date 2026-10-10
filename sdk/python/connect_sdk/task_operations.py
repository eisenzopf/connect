"""Backing-credential task control; narrow model grants use the protected adapter."""
from .errors import ConnectError
from .task_types import TASK_PROFILE, TaskCommand, TaskOutcome


class TaskOperations:
    _task_profile = False

    def _observe_outcome(self, outcome):
        if outcome["kind"] == "capabilities":
            self._task_profile = TASK_PROFILE in outcome["data"].get("extensions", [])
        return outcome

    def _require_task_profile(self, operation):
        if operation.get("op") == "task" and not self._task_profile:
            raise ConnectError("unsupported-profile", "Discover connect-task/1 through capabilities before task requests")

    def negotiate_tasks(self, address: str, conversation_id: str | None = None) -> None:
        """Discovery never expands current credential, task or recipient authority."""
        outcome = self.request({"op": "capabilities", "address": address, "conversation_id": conversation_id})
        self._observe_outcome(outcome)
        if not self._task_profile:
            raise ConnectError("unsupported-profile", "Server does not advertise connect-task/1")

    def task(self, command: TaskCommand) -> TaskOutcome:
        """Keep IDs/inputs across uncertainty. Pending is not completion."""
        operation = {"op": "task", "request": {"task_profile": TASK_PROFILE, "command": command}}
        self._require_task_profile(operation)
        return self.request(operation)["data"]


def exchange_timeout(base, operation):
    """Bound the additional server wait without extending other exchanges."""
    if operation.get("op") != "task":
        return base
    command = operation.get("request", {}).get("command", {})
    wait = command.get("wait_ms", 0) if command.get("action") in ("inbox", "wait") else 0
    return base + (min(wait, 10000) / 1000 if type(wait) is int and wait >= 0 else 0)
