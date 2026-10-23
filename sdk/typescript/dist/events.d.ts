import type { AgentObservation } from "./agent_observations.js";
export type * from "./agent_observations.js";
import type { Session, Message, Artifact, Call, RecipientReceipt } from "./types.js";
import type { OperationRecord, OperationState } from "./transitions.js";
export interface SnapshotQuery {
    page?: string | null;
    limit?: number | null;
}
export interface EventQuery {
    after?: string | null;
    limit?: number | null;
    lease?: string | null;
    lease_seconds?: number | null;
    wait_ms?: number;
}
export interface ObservationQuery {
    after?: string | null;
    limit?: number | null;
    wait_ms?: number;
}
export interface ConversationState {
    conversation_id: string;
    revision: number;
    closed: boolean;
    policy: "persistent" | "ephemeral";
}
export interface MemberView {
    participant_id: string;
    conversation_id: string;
    identity_id: string;
    kind: "human" | "ai" | "system" | "external";
    role: string;
    membership_epoch: number;
    active: boolean;
    removal_reason: string | null;
}
export interface SessionAppearance {
    session_id: string;
    participant_id: string;
    membership_epoch: number;
    role: string;
    state: "invited" | "joining" | "present" | "left" | "removed";
}
export interface ConnectionState {
    connection_id: string;
    session_id: string;
    participant_id: string;
    membership_epoch: number;
    endpoint_id: string;
    device_id: string;
    generation: number;
    revision: number;
    state: string;
    replaces: string | null;
}
export interface StreamState {
    stream_id: string;
    session_id: string;
    participant_id: string;
    connection_id: string | null;
    medium: string;
    state: string;
    revision: number;
}
export interface PreparedConnection {
    connection_id: string;
    conversation_id: string;
    session_id: string;
    participant_id: string;
    membership_epoch: number;
    endpoint_id: string;
    generation: 1;
    revision: 0;
    state: "preparing";
}
export interface SessionSnapshot {
    id: string;
    conversation_id: string;
    state: string;
    media: string[];
    intent: string | null;
    capabilities_offer: Record<string, unknown> | null;
    codec_preferences: string[];
    is_main: boolean;
    revision: number;
    topology_revision: number;
    created_at: number;
    updated_at: number;
    ended_reason: string | null;
}
/** Provider observations do not admit a local media Connection. */
export interface ProviderSessionVoiceView {
    id: string;
    operation_id: string;
    conversation_id: string;
    session_id: string;
    participant_id: string;
    membership_epoch: number;
    endpoint_id: string;
    state: "starting" | "active" | "ended" | "failed" | "unknown";
    direction: "inbound" | "outbound";
    /** An intended Participant is not authentication of the reported caller. */
    remote_identity_verified: false;
    revision: number;
    observed_at: number | null;
    closed_at: number | null;
    ended_reason: string | null;
    problem: string | null;
    local_media: false;
}
export type SnapshotItem = {
    kind: "provider_call";
    data: ProviderSessionVoiceView;
} | {
    kind: "conversation";
    data: ConversationState;
} | {
    kind: "participant";
    data: MemberView;
} | {
    kind: "session";
    data: SessionSnapshot;
} | {
    kind: "session_participant";
    data: SessionAppearance;
} | {
    kind: "connection";
    data: ConnectionState;
} | {
    kind: "stream";
    data: StreamState;
} | {
    kind: "operation";
    data: OperationRecord;
};
export interface SnapshotPage {
    conversation_id: string;
    snapshot_id: string;
    revision: number;
    cursor: string;
    expires_at: number;
    items: SnapshotItem[];
    next: string | null;
    has_more: boolean;
}
export interface EventSource {
    kind: string;
    correlation_id: string;
}
/** Historical local writer fact. It never asserts remote delivery. */
export interface NativeDataWrite {
    message_id: string;
    source_connection_id: string;
    target_connection_id: string;
    participant_id: string;
    membership_epoch: number;
    generation: number;
    disposition: "not_started" | "local_written" | "submission_unknown";
    provenance: "local_writer" | "runtime_recovery";
    observed_at: number;
}
export interface ConversationClosed {
    conversation_id: string;
    reason_code: number;
    reason: string;
    closed_at: number;
}
export interface SessionEnded {
    session: SessionSnapshot;
    by_participant: string | null;
    by_membership_epoch: number | null;
    reason_code: number;
    reason: string;
    ended_at: number;
}
export interface SessionStarted {
    session: SessionSnapshot;
    started_at: number;
    participants_present: string[];
    active_connections: {
        connid: string;
        participant_id: string;
        transport: "quic" | "webtransport" | "websocket+webrtc" | "sip-interop" | "webrtc-interop";
    }[];
    negotiated_capabilities: Record<string, unknown>;
}
export interface SessionParticipantJoined {
    fact_id: string;
    session: SessionSnapshot;
    membership_epoch: number;
    participant: {
        participant_id: string;
        identity_id: string;
        kind: "human" | "ai" | "system" | "external";
        role: string;
        display_name: string;
        joined_at: number;
    };
    via_connection: string;
}
export interface SessionParticipantLeft {
    fact_id: string;
    session: SessionSnapshot;
    membership_epoch: number;
    participant_id: string;
    left_at: number;
    reason: "explicit-leave" | "all-connections-ended" | "kicked" | "transferred-out";
}
export type JournalData = {
    kind: "session_participant_joined";
    data: SessionParticipantJoined;
} | {
    kind: "session_participant_left";
    data: SessionParticipantLeft;
} | {
    kind: "session_started";
    data: SessionStarted;
} | {
    kind: "session_ended";
    data: SessionEnded;
} | {
    kind: "session_control";
    data: SessionSnapshot;
} | {
    kind: "conversation_closed";
    data: ConversationClosed;
} | {
    kind: "provider_call";
    data: ProviderSessionVoiceView;
} | {
    kind: "connection_prepared";
    data: PreparedConnection;
} | {
    kind: "connection_ready";
    data: ConnectionState & {
        state: "ready";
    };
} | {
    kind: "connection_retired";
    data: ConnectionState & {
        state: "retired";
    };
} | {
    kind: "message";
    data: Message;
} | {
    kind: "session";
    data: Session;
} | {
    kind: "artifact";
    data: Artifact;
} | {
    kind: "call";
    data: Call;
} | {
    kind: "receipt";
    data: RecipientReceipt;
} | {
    kind: "native_data_write";
    data: NativeDataWrite;
} | {
    kind: "membership";
    data: {
        participant_id: string | null;
        identity_id: string | null;
        role: string | null;
        active: boolean | null;
        membership_epoch: number | null;
        removal_reason: string | null;
    };
} | {
    kind: "operation";
    data: {
        operation_id: string;
        record_id: string;
        state: OperationState;
    };
} | {
    kind: "changed";
    data: {
        resource_kind: string | null;
        resource_id: string | null;
    };
};
export interface JournalEvent {
    event_id: string;
    cursor: string;
    conversation_id: string;
    session_id: string | null;
    connection_id: string | null;
    participant_id: string | null;
    operation_id: string | null;
    kind: string;
    revision: number;
    source: EventSource;
    observed_at: number;
    audience: {
        participant_id: string;
        membership_epoch: number;
    }[];
    data: JournalData;
}
export interface EventPage {
    conversation_id: string;
    events: JournalEvent[];
    cursor: string;
    has_more: boolean;
    lease: string | null;
    expires_at: number | null;
    resync_required: boolean;
    snapshot: SnapshotPage | null;
}
export interface ObservationCapabilities {
    transcript: boolean;
    speech: boolean;
    actions: string[];
}
export type ObservationDraft = {
    kind: "agent";
    observation: AgentObservation;
} | {
    kind: "transcript";
    text: string;
    completeness: "interim" | "final" | "incomplete";
    turn_id: string | null;
    fragment_id: string | null;
} | {
    kind: "speech";
    state: "started" | "stopped";
    turn_id: string | null;
} | {
    kind: "action-request";
    action: string;
    provider_tool_call_id: string;
    arguments: Record<string, unknown>;
    turn_id: string | null;
};
export interface ObservationWork {
    task_id: string;
    effect_id: string;
    question_id: string;
}
export interface ObservationRoute {
    work: ObservationWork | null;
    correlation_id: string;
    participant_id: string;
    connection_id: string;
    generation: number;
    source: string;
    capabilities: ObservationCapabilities;
}
export interface SessionObservation {
    work: ObservationWork | null;
    observation_id: string;
    conversation_id: string;
    session_id: string;
    participant_id: string;
    membership_epoch: number;
    connection_id: string;
    generation: number;
    source: EventSource;
    observed_at: number;
    data: ObservationDraft;
}
export interface ObservationPage {
    conversation_id: string;
    session_id: string;
    observations: SessionObservation[];
    expires_at: number;
    routes: ObservationRoute[];
    cursor: string;
    has_more: boolean;
    resync_required: boolean;
}
