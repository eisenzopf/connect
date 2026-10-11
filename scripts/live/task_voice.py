"""Task-bound session control for the separately invoked audio rehearsal."""
import os
import re
import stat
import tomllib


def require_task_config(path):
    """Reject accidental backing credentials; the server authenticates the grant."""
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & 0o077:
            raise ValueError("Task configuration must be a private regular file")
        data = source.read(65537)
    if len(data) > 65536:
        raise ValueError("Task configuration exceeds 64 KiB")
    try:
        config = tomllib.loads(data.decode())
    except (UnicodeError, tomllib.TOMLDecodeError) as error:
        raise ValueError("Invalid task configuration") from error
    subject = config.get("credential_id", "")
    if not isinstance(subject, str) or not re.fullmatch(
            r"connect-task-grant:[0-9a-fA-F-]{36}", subject):
        raise ValueError("Export a Native task grant; backing credentials are not accepted")


class TaskVoice:
    """Use one existing prepared Session; never create a task or replacement leg."""
    def __init__(self, task_id, conversation_id, session_id, peer_id, intent):
        self.task_id = task_id
        self.conversation_id = conversation_id
        self.session_id = session_id
        self.peer_id = peer_id
        self.intent = intent

    def invoke(self, client, name, operation):
        result = client.task({"action": "invoke", "task_id": self.task_id,
                              "request_id": self.intent[name], "step_id": self.intent[name],
                              "operation": operation})
        tool = result.get("data", {})
        if (result.get("kind") != "tool" or tool.get("status") != "completed"
                or tool.get("task_id") != self.task_id):
            raise ValueError("Session control is incomplete; retain the original intent")
        return tool["outcome"]

    def bind(self, client):
        task = client.negotiate_task(self.task_id)
        if task["conversation_id"] != self.conversation_id:
            raise ValueError("Task belongs to another conversation")
        outcome = self.invoke(client, "inspect", {"op": "session_get",
                              "conversation_id": self.conversation_id,
                              "session_id": self.session_id})
        session = outcome.get("data", {})
        if (outcome.get("kind") != "session" or session.get("id") != self.session_id
                or session.get("conversation_id") != self.conversation_id
                or session.get("participant_id") != self.peer_id
                or session.get("state") != "accepted" or "audio" not in session.get("media", [])):
            raise ValueError("Expected this task's accepted audio Session")
        return session

    def finish(self, client):
        client.negotiate_task(self.task_id)
        outcome = self.invoke(client, "end", {"op": "session_end",
                              "request_id": self.intent["end"],
                              "conversation_id": self.conversation_id,
                              "session_id": self.session_id,
                              "reason": "automated-rehearsal-complete"})
        if (outcome.get("kind") != "session" or outcome.get("data", {}).get("id") != self.session_id
                or outcome.get("data", {}).get("state") != "ended"):
            raise ValueError("Session end returned a different resource")
        return client.task({"action": "recall", "task_id": self.task_id, "limit": 100})
