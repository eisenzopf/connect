"""Typed Connect task inputs/results; secrets and verified bindings are not inputs."""
from typing import Any, Literal, NotRequired, TypedDict, Union

TASK_PROFILE = "connect-task/1"
TaskState = Literal["ready", "running", "waiting", "completed", "cancelled", "failed", "expired"]
EffectState = Literal["prepared", "dispatched", "succeeded", "failed", "uncertain", "cancelled"]
TaskPermission = Literal[
    "resolve", "capabilities", "send", "history", "context_view", "context_deliveries",
    "inbox", "request_status", "artifact_begin", "artifact_append", "artifact_finish",
    "artifact_get", "artifact_delete", "session_invite", "session_accept", "session_reject",
    "session_cancel", "session_end", "session_get", "call_start", "call_status", "call_end",
    "sip_dial", "sip_status", "sip_end",
]

class TaskSmsEnrollment(TypedDict):
    source: str
    grant_id: str
    purpose: str

class TaskScope(TypedDict):
    operations: list[TaskPermission]
    recipients: list[str]
    sms_enrollments: NotRequired[list[TaskSmsEnrollment]]

class TaskDefinition(TypedDict):
    conversation_id: str
    scope: TaskScope
    expires_at: int
    initiating_event_id: NotRequired[str | None]

class TaskRecord(TypedDict):
    id: str
    owner_id: str
    conversation_id: str
    scope: TaskScope
    state: TaskState
    revision: int
    expires_at: int
    initiating_event_id: str | None
    created_at: int
    updated_at: int
    reason: str | None

class TaskStepClaim(TypedDict):
    task_id: str
    step_id: str
    worker_id: str
    revision: int
    expires_at: int

class TaskEffect(TypedDict):
    id: str
    task_id: str
    step_id: str
    request_id: str
    state: EffectState
    outcome: dict[str, Any] | None
    error: NotRequired[dict[str, str] | None]

class TaskReply(TypedDict):
    event_id: str
    conversation_id: str
    task_id: str | None
    reply_to: str | None
    assigned_by: str | None
    consumed: bool

class TaskSource(TypedDict):
    source_id: str
    kind: str
    actor_id: str
    created_at: int | None
    provenance: str
    state: Literal["available", "deleted", "expired", "unavailable"]
    body: str | None

class TaskGet(TypedDict):
    action: Literal["get"]
    task_id: str

class TaskCreate(TypedDict):
    action: Literal["create"]
    request_id: str
    definition: TaskDefinition

class TaskCancel(TypedDict):
    action: Literal["cancel"]
    request_id: str
    task_id: str
    reason: NotRequired[str | None]

class TaskTransition(TypedDict):
    action: Literal["transition"]
    request_id: str
    task_id: str
    expected_revision: int
    state: TaskState
    reason: NotRequired[str | None]

class TaskReplies(TypedDict):
    action: Literal["replies"]
    conversation_id: str
    after: NotRequired[str | None]
    limit: int

class TaskAssignReply(TypedDict):
    action: Literal["assign_reply"]
    request_id: str
    task_id: str
    event_id: str
    reply_to: NotRequired[str | None]

class TaskReplyCommand(TypedDict):
    action: Literal["reply"]
    request_id: str
    conversation_id: str
    reply_to: str
    body: str

class TaskWait(TypedDict):
    action: Literal["inbox", "wait"]
    task_id: str
    after: NotRequired[str | None]
    limit: int
    wait_ms: int

class TaskConsumeReply(TypedDict):
    action: Literal["consume_reply"]
    request_id: str
    task_id: str
    event_id: str
    expected_revision: int

class TaskInvoke(TypedDict):
    action: Literal["invoke"]
    request_id: str
    task_id: str
    step_id: str
    operation: dict[str, Any]
    reply_to: NotRequired[str | None]
    wait_for_reply: NotRequired[bool]

class TaskRecall(TypedDict):
    action: Literal["recall"]
    task_id: str
    after: NotRequired[str | None]
    limit: int

class TaskEffectGet(TypedDict):
    action: Literal["effect_get"]
    task_id: str
    effect_id: str

class TaskClaim(TypedDict):
    action: Literal["claim"]
    request_id: str
    task_id: str
    step_id: str
    worker_id: str
    expected_revision: NotRequired[int | None]

class TaskClaimGet(TypedDict):
    action: Literal["claim_get"]
    task_id: str
    step_id: str

class TaskReleaseClaim(TypedDict):
    action: Literal["release_claim"]
    request_id: str
    task_id: str
    step_id: str
    worker_id: str
    expected_revision: int

TaskCommand = Union[TaskGet, TaskCreate, TaskCancel, TaskTransition, TaskReplies,
                    TaskAssignReply, TaskReplyCommand, TaskWait, TaskConsumeReply,
                    TaskInvoke, TaskRecall, TaskEffectGet, TaskClaim, TaskClaimGet,
                    TaskReleaseClaim]

class TaskEvent(TypedDict):
    id: str
    task_id: str
    actor_id: str
    sequence: int
    created_at: int
    data: dict[str, Any]

class _RecordOutcome(TypedDict):
    kind: Literal["record"]
    data: TaskRecord

class _ReplyPage(TypedDict):
    items: list[TaskReply]
    next: str | None

class _RepliesOutcome(TypedDict):
    kind: Literal["replies"]
    data: _ReplyPage

class _InboxPage(TypedDict):
    events: list[TaskEvent]
    next: str | None

class _InboxOutcome(TypedDict):
    kind: Literal["inbox"]
    data: _InboxPage

class _RecallPage(TypedDict):
    sources: list[TaskSource]
    next: str | None
    omitted: int

class _RecallOutcome(TypedDict):
    kind: Literal["recall"]
    data: _RecallPage

class _EffectOutcome(TypedDict):
    kind: Literal["effect"]
    data: TaskEffect

class _ClaimOutcome(TypedDict):
    kind: Literal["claim"]
    data: TaskStepClaim | None

class _CompletedTool(TypedDict):
    status: Literal["completed"]
    task_id: str
    outcome: dict[str, Any]

class _PendingTool(TypedDict):
    status: Literal["pending"]
    task_id: str
    effect_id: str | None

class _UncertainTool(TypedDict):
    status: Literal["uncertain"]
    task_id: str
    effect_id: str

TaskToolResult = Union[_CompletedTool, _PendingTool, _UncertainTool]

class _ToolOutcome(TypedDict):
    kind: Literal["tool"]
    data: TaskToolResult

class _ReplyAccepted(TypedDict):
    request_id: str
    message_id: str
    reply: TaskReply

class _ReplyAcceptedOutcome(TypedDict):
    kind: Literal["reply_accepted"]
    data: _ReplyAccepted

TaskOutcome = Union[_RecordOutcome, _RepliesOutcome, _InboxOutcome, _RecallOutcome,
                    _EffectOutcome, _ClaimOutcome, _ToolOutcome, _ReplyAcceptedOutcome]
