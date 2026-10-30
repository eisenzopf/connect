"""Connect SDK public exports, loaded when requested by the caller.

One-shot workers import only their transport dependencies. Public types remain
explicit for static checking; importing opens no connections or workers.
"""
from typing import TYPE_CHECKING

__all__ = [
    'ConnectError', 'NativeClient', 'NativeAudioClient', 'HttpClient', 'ContextRecord',
    'TASK_PROFILE', 'TaskCommand', 'TaskDefinition', 'TaskOutcome', 'TaskRecord', 'TaskScope',
    'Media', 'MediaConnection', 'MediaStream', 'MediaBinding', 'MediaNegotiation',
    'NegotiatedMediaStream', 'RtpHeader', 'RtpPacket', 'SenderClockReport', 'ClockPacket',
    'MediaGap', 'MediaPacket', 'LocalMediaSubmission', 'MediaStats', 'MembershipAction',
    'MembershipRequest', 'MembershipOutcome', 'MembershipSnapshot', 'MembershipInvitations',
    'Client', 'Conversation', 'Participant', 'Operation', 'OperationKind', 'OperationKinds',
    'OperationReference', 'Cancellation', 'CapabilitySnapshot', 'Completion', 'ConversationId',
    'ConversationRevision', 'ConversationSnapshot', 'DecisionReceipt', 'DeliveryObservation',
    'IdentityId', 'IdentityKind', 'MessageAcceptance', 'MessageId', 'MutationOptions', 'NonEmpty',
    'OperationId', 'OperationKey', 'OperationSnapshot', 'ParticipantId', 'ParticipantSnapshot',
    'RequestOptions', 'Scope', 'SdkError', 'SdkCancelledError', 'SessionId', 'SessionRevision',
    'TopologyRevision', 'StreamRevision', 'ConnectionId', 'StreamId', 'EndpointId', 'Cursor',
    'Session', 'SessionDecision', 'Sessions', 'SessionSnapshot', 'ParticipantLeave', 'RoleChange',
    'MediumUpgrade', 'CodecRenegotiate', 'Medium', 'InviteMedium', 'SessionUpdate', 'Connection',
    'Connections', 'Stream', 'Streams', 'CodecObservation', 'CodecOutcome',
    'ConnectionCodecRenegotiate', 'ConnectionSnapshot', 'ConnectionUpdate',
    'ConnectionUpdateOutcome', 'ConnectionUpdateReceipt', 'Direction', 'GateOutcome',
    'HoldConnection', 'MediaFormat', 'Mute', 'NegotiatedStream', 'OfferedStream',
    'ParticipantStreams', 'Quality', 'QualityOutcome', 'IceRestartOutcome', 'ResumeConnection',
    'SourceClock', 'StreamGateObservation', 'StreamResourceRevision', 'StreamSelectionReceipt',
    'StreamSelector', 'StreamSnapshot', 'StreamTarget', 'ConsultationOperation', 'ConsultInput',
    'ConsultationResult', 'ConversationRole', 'DeviceId', 'EndpointTarget', 'CandidateTarget',
    'HandoffInput', 'HandoffResult', 'HandoffRoleChange', 'MoveInput', 'MoveResult', 'MoveBinding',
    'SessionRole', 'TopologyChange', 'AudienceMember', 'EventSource', 'JournalEvent',
    'IncomingInvitation', 'EventStream', 'InvitationStream', 'RawCanonical', 'ValidatedEnvelope',
    'AdministrativeClient', 'AdministrativeOperations', 'TrustedProvisioningCommand', 'Json',
    'CanonicalHttpClient', 'HttpCommandScope', 'HttpControlTransport', 'HttpEnvelope',
    'SynchronousSdk', 'TextData', 'JsonData', 'BinaryData', 'DataPayload', 'ReceivedData',
    'DataAcceptance', 'DataStats'
]

_EXPORT_GROUPS = (
    ('.errors', (
        'ConnectError',
    )),
    ('.native', (
        'NativeClient',
    )),
    ('.native_audio', (
        'NativeAudioClient',
    )),
    ('.http', (
        'HttpClient',
    )),
    ('.record_context', (
        'ContextRecord',
    )),
    ('.client', (
        'Client', 'Conversation', 'Participant', 'Operation', 'OperationKind', 'OperationKinds',
        'OperationReference',
    )),
    ('.sdk_sessions', (
        'Session', 'SessionDecision', 'Sessions',
    )),
    ('.sdk_connections', (
        'Connection', 'Connections', 'Stream', 'Streams',
    )),
    ('.sdk_movements', (
        'ConsultationOperation',
    )),
    ('.sdk_events', (
        'EventStream',
    )),
    ('.sdk_incoming', (
        'InvitationStream',
    )),
    ('.sdk_raw', (
        'RawCanonical', 'ValidatedEnvelope',
    )),
    ('.sdk_administration', (
        'AdministrativeClient', 'AdministrativeOperations', 'TrustedProvisioningCommand',
    )),
    ('.sdk_http', (
        'CanonicalHttpClient', 'HttpCommandScope', 'HttpControlTransport', 'HttpEnvelope',
    )),
    ('.sdk_sync', (
        'SynchronousSdk',
    )),
    ('.sdk_media', (
        'Media', 'MediaConnection', 'MediaStream',
    )),
    ('.sdk_media_types', (
        'MediaBinding', 'MediaNegotiation', 'NegotiatedMediaStream', 'RtpHeader', 'RtpPacket',
        'SenderClockReport', 'ClockPacket', 'MediaGap', 'MediaPacket', 'LocalMediaSubmission',
        'MediaStats',
    )),
    ('.sdk_types', (
        'Cancellation', 'CapabilitySnapshot', 'Completion', 'ConversationId',
        'ConversationRevision', 'Json', 'ConversationSnapshot', 'DecisionReceipt',
        'DeliveryObservation', 'IdentityId', 'IdentityKind', 'MessageAcceptance', 'MessageId',
        'MutationOptions', 'NonEmpty', 'OperationId', 'OperationKey', 'OperationSnapshot',
        'ParticipantId', 'ParticipantSnapshot', 'RequestOptions', 'Scope', 'SdkError',
        'SdkCancelledError', 'SessionId', 'SessionRevision', 'TopologyRevision', 'StreamRevision',
        'ConnectionId', 'StreamId', 'EndpointId', 'Cursor', 'SessionSnapshot', 'ParticipantLeave',
        'RoleChange', 'MediumUpgrade', 'CodecRenegotiate', 'Medium', 'InviteMedium',
        'SessionUpdate', 'CodecObservation', 'CodecOutcome', 'ConnectionCodecRenegotiate',
        'ConnectionSnapshot', 'ConnectionUpdate', 'ConnectionUpdateOutcome',
        'ConnectionUpdateReceipt', 'Direction', 'GateOutcome', 'HoldConnection', 'MediaFormat',
        'Mute', 'NegotiatedStream', 'OfferedStream', 'ParticipantStreams', 'Quality',
        'QualityOutcome', 'IceRestartOutcome', 'ResumeConnection', 'SourceClock',
        'StreamGateObservation', 'StreamResourceRevision', 'StreamSelectionReceipt',
        'StreamSelector', 'StreamSnapshot', 'StreamTarget', 'ConsultInput', 'ConsultationResult',
        'ConversationRole', 'DeviceId', 'EndpointTarget', 'CandidateTarget', 'HandoffInput',
        'HandoffResult', 'HandoffRoleChange', 'MoveInput', 'MoveResult', 'MoveBinding',
        'SessionRole', 'TopologyChange', 'AudienceMember', 'EventSource', 'JournalEvent',
        'IncomingInvitation',
    )),
    ('.task_types', (
        'TASK_PROFILE', 'TaskCommand', 'TaskDefinition', 'TaskOutcome', 'TaskRecord', 'TaskScope',
    )),
    ('.membership_types', (
        'MembershipAction', 'MembershipRequest', 'MembershipOutcome', 'MembershipSnapshot',
        'MembershipInvitations',
    )),
    ('.sdk_data_types', (
        'TextData', 'JsonData', 'BinaryData', 'DataPayload', 'ReceivedData', 'DataAcceptance',
        'DataStats',
    )),
)
_EXPORTS = {name: module for module, names in _EXPORT_GROUPS for name in names}

if TYPE_CHECKING:
    from .errors import ConnectError
    from .native import NativeClient
    from .native_audio import NativeAudioClient
    from .http import HttpClient
    from .record_context import ContextRecord
    from .client import Client, Conversation, Participant, Operation, OperationKind, OperationKinds, OperationReference
    from .sdk_sessions import Session, SessionDecision, Sessions
    from .sdk_connections import Connection, Connections, Stream, Streams
    from .sdk_movements import ConsultationOperation
    from .sdk_events import EventStream
    from .sdk_incoming import InvitationStream
    from .sdk_raw import RawCanonical, ValidatedEnvelope
    from .sdk_administration import AdministrativeClient, AdministrativeOperations, TrustedProvisioningCommand
    from .sdk_http import CanonicalHttpClient, HttpCommandScope, HttpControlTransport, HttpEnvelope
    from .sdk_sync import SynchronousSdk
    from .sdk_media import Media, MediaConnection, MediaStream
    from .sdk_media_types import (MediaBinding, MediaNegotiation, NegotiatedMediaStream, RtpHeader,
        RtpPacket, SenderClockReport, ClockPacket, MediaGap, MediaPacket, LocalMediaSubmission, MediaStats)
    from .sdk_types import (
        Cancellation, CapabilitySnapshot, Completion, ConversationId, ConversationRevision, Json,
        ConversationSnapshot, DecisionReceipt, DeliveryObservation, IdentityId, IdentityKind,
        MessageAcceptance, MessageId, MutationOptions, NonEmpty, OperationId, OperationKey,
        OperationSnapshot, ParticipantId, ParticipantSnapshot, RequestOptions, Scope, SdkError,
        SdkCancelledError, SessionId, SessionRevision, TopologyRevision, StreamRevision,
        ConnectionId, StreamId, EndpointId, Cursor,
        SessionSnapshot, ParticipantLeave, RoleChange, MediumUpgrade, CodecRenegotiate,
        Medium, InviteMedium, SessionUpdate,
        CodecObservation, CodecOutcome, ConnectionCodecRenegotiate, ConnectionSnapshot, ConnectionUpdate,
        ConnectionUpdateOutcome, ConnectionUpdateReceipt, Direction, GateOutcome, HoldConnection,
        MediaFormat, Mute, NegotiatedStream, OfferedStream, ParticipantStreams, Quality, QualityOutcome, IceRestartOutcome,
        ResumeConnection, SourceClock, StreamGateObservation, StreamResourceRevision, StreamSelectionReceipt,
        StreamSelector, StreamSnapshot, StreamTarget,
        ConsultInput, ConsultationResult, ConversationRole, DeviceId, EndpointTarget, CandidateTarget,
        HandoffInput, HandoffResult, HandoffRoleChange, MoveInput, MoveResult, MoveBinding, SessionRole, TopologyChange,
        AudienceMember, EventSource, JournalEvent, IncomingInvitation,
    )
    from .task_types import TASK_PROFILE, TaskCommand, TaskDefinition, TaskOutcome, TaskRecord, TaskScope
    from .membership_types import MembershipAction, MembershipRequest, MembershipOutcome, MembershipSnapshot, MembershipInvitations
    from .sdk_data_types import TextData, JsonData, BinaryData, DataPayload, ReceivedData, DataAcceptance, DataStats

else:
    from importlib import import_module as _import_module

    def __getattr__(name: str):
        module = _EXPORTS.get(name)
        if module is None:
            raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
        value = getattr(_import_module(module, __name__), name)
        globals()[name] = value
        return value

    def __dir__():
        return sorted(set(globals()) | set(__all__))
