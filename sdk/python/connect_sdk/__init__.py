"""Connect SDK: native verified UCTP/QUIC or explicit loopback HTTP/JSON."""
from .errors import ConnectError
from .native import NativeClient
from .native_audio import NativeAudioClient
from .http import HttpClient
from .record_context import ContextRecord
from .client import Client, Conversation, Participant, Operation, OperationKind, OperationKinds, OperationReference
from .sdk_types import (
    Cancellation, CapabilitySnapshot, Completion, ConversationId, ConversationRevision,
    ConversationSnapshot, DecisionReceipt, DeliveryObservation, IdentityId, IdentityKind,
    MessageAcceptance, MessageId, MutationOptions, NonEmpty, OperationId, OperationKey,
    OperationSnapshot, ParticipantId, ParticipantSnapshot, RequestOptions, Scope, SdkError,
    SdkCancelledError, SessionId, SessionRevision, TopologyRevision, StreamRevision,
    ConnectionId, StreamId, EndpointId, Cursor,
)

__all__ = ["ConnectError", "NativeClient", "NativeAudioClient", "HttpClient", "ContextRecord", "TASK_PROFILE", "TaskCommand", "TaskDefinition", "TaskOutcome", "TaskRecord", "TaskScope"]

from .task_types import TASK_PROFILE, TaskCommand, TaskDefinition, TaskOutcome, TaskRecord, TaskScope

from .membership_types import MembershipAction, MembershipRequest, MembershipOutcome, MembershipSnapshot, MembershipInvitations
__all__ += ["MembershipAction", "MembershipRequest", "MembershipOutcome", "MembershipSnapshot", "MembershipInvitations"]
__all__ += [
    "Client", "Conversation", "Participant", "Operation", "OperationKind", "OperationKinds", "OperationReference",
    "Cancellation", "CapabilitySnapshot", "Completion", "ConversationId", "ConversationRevision", "ConversationSnapshot",
    "DecisionReceipt", "DeliveryObservation", "IdentityId", "IdentityKind", "MessageAcceptance", "MessageId",
    "MutationOptions", "NonEmpty", "OperationId", "OperationKey", "OperationSnapshot", "ParticipantId",
    "ParticipantSnapshot", "RequestOptions", "Scope", "SdkError", "SdkCancelledError", "SessionId", "SessionRevision",
    "TopologyRevision", "StreamRevision", "ConnectionId", "StreamId", "EndpointId", "Cursor",
]
