import type { ContextRecord } from "./record_context.js";
export type { ContextRecord } from "./record_context.js";
import type { MembershipRequest, MembershipOutcome, MembershipSnapshot, MembershipInvitations } from "./membership.js";
export type * from "./membership.js";
import type { Conversation, ConversationPage, ConversationCreate, ConversationClose, ConversationListQuery } from "./conversations.js";
export type * from "./conversations.js";
import type { SnapshotQuery, EventQuery, ObservationQuery, SnapshotPage, EventPage, ObservationPage } from "./events.js";
export type * from "./events.js";
import type { TransitionRequest, OperationAction, OperationRecord } from "./transitions.js";
export type * from "./transitions.js";
import type { MediaDiagnostics } from "./media_diagnostics.js";
export type { MediaDiagnostics, MediaQueueObservation, MediaForwardObservation, QuicTransmissionObservation, QuicSendBufferObservation, } from "./media_diagnostics.js";
import type { ContactState, ContactPairing } from "./contacts.js";
export type { ContactState, ContactPairing, OwnershipState, } from "./contacts.js";
import type { TaskCommand, TaskOutcome } from "./tasks.js";
export type Operation = {
    op: "membership";
    request: MembershipRequest;
} | {
    op: "membership_snapshot";
    conversation_id: string;
} | {
    op: "membership_invitations";
    after: string | null;
    limit: number;
} | {
    op: "conversation_snapshot";
    conversation_id: string;
    query: SnapshotQuery;
} | {
    op: "conversation_events";
    conversation_id: string;
    query: EventQuery;
} | {
    op: "session_observations";
    conversation_id: string;
    session_id: string;
    query: ObservationQuery;
} | {
    op: "transition";
    request: TransitionRequest;
} | {
    op: "durable_operation";
    request: OperationAction;
} | {
    op: "contact_pairing_start";
    request_id: string;
    peer_identity_id: string;
    expires_at: number;
} | {
    op: "contact_pairing_complete";
    request_id: string;
    pairing_id: string;
    challenge: string;
} | {
    op: "contact_pairing_revoke";
    request_id: string;
    pairing_id: string;
} | {
    op: "contact_state";
    peer_identity_id: string;
} | {
    op: "task";
    request: {
        task_profile: string;
        command: TaskCommand;
    };
} | {
    op: "resolve";
    address: string;
} | {
    op: "capabilities";
    address: string;
    conversation_id: string | null;
} | {
    op: "browser_binding";
    conversation_id?: string;
    session_id?: string;
} | {
    op: "context_deliveries";
    conversation_id: string;
    before: string | null;
    limit: number;
} | {
    op: "create";
    request_id: string;
    request: ConversationCreate;
} | {
    op: "send";
    request_id: string;
    conversation_id: string;
    message: MessageDraft;
} | {
    op: "history";
    conversation_id: string;
    after: string | null;
    limit: number;
} | {
    op: "inbox";
    cursor: string | null;
    limit: number;
    wait_ms: number;
} | {
    op: "request_status";
    request_id: string;
} | {
    op: "provider_status";
    conversation_id: string;
} | {
    op: "close";
    request_id: string;
    conversation_id: string;
    options: ConversationClose;
} | {
    op: "conversations";
    query: ConversationListQuery;
} | {
    op: "call_start";
    request_id: string;
    conversation_id: string;
    route_id: string;
    destination: string;
} | {
    op: "call_status";
    conversation_id: string;
    call_id: string;
} | {
    op: "call_end";
    request_id: string;
    conversation_id: string;
    call_id: string;
} | {
    op: "session_invite";
    request_id: string;
    conversation_id: string;
    participant_id: string;
    media: string[];
} | {
    op: "session_accept" | "session_cancel";
    request_id: string;
    conversation_id: string;
    session_id: string;
} | {
    op: "session_reject" | "session_end";
    request_id: string;
    conversation_id: string;
    session_id: string;
    reason: string | null;
} | {
    op: "session_get";
    conversation_id: string;
    session_id: string;
} | {
    op: "sip_dial";
    request_id: string;
    conversation_id: string;
    session_id: string;
    destination: string;
} | {
    op: "sip_status";
    conversation_id: string;
    sip_leg_id: string;
} | {
    op: "sip_end";
    request_id: string;
    conversation_id: string;
    sip_leg_id: string;
} | {
    op: "context_view";
    conversation_id: string;
    limit: number;
} | {
    op: "availability_set";
    request_id: string;
    state: "available" | "unavailable";
    preferred_route: string | null;
    expires_at: number;
} | {
    op: "availability_get";
    identity_id: string;
} | {
    op: "artifact_begin";
    to: MessageAudience;
    request_id: string;
    conversation_id: string;
    name: string;
    media_type: string;
    size: number;
    sha256: string;
    expires_at?: number | null;
} | {
    op: "artifact_append";
    request_id: string;
    conversation_id: string;
    artifact_id: string;
    offset: number;
    data: string;
} | {
    op: "artifact_finish" | "artifact_delete";
    request_id: string;
    conversation_id: string;
    artifact_id: string;
} | {
    op: "artifact_get";
    conversation_id: string;
    artifact_id: string;
    offset: number;
    limit: number;
} | {
    op: "artifact_inspect";
    conversation_id: string;
    artifact_id: string;
} | {
    op: "artifact_upload_link";
    conversation_id: string;
    artifact_id: string;
    expires_in: number;
} | {
    op: "artifact_download_link";
    conversation_id: string;
    artifact_id: string;
    message_id: string;
    expires_in: number;
} | {
    op: "artifact_link_get";
    conversation_id: string;
    artifact_id: string;
    link: string;
    offset: number;
    limit: number;
} | {
    op: "artifact_link_append";
    request_id: string;
    conversation_id: string;
    artifact_id: string;
    link: string;
    offset: number;
    data: string;
} | {
    op: "message_receipts";
    conversation_id: string;
    message_id: string;
    after: string | null;
    limit: number;
} | {
    op: "message_receipt";
    request_id: string;
    conversation_id: string;
    message_id: string;
    state: "delivered" | "read";
} | {
    op: "receipt_preferences_get";
    conversation_id: string;
} | {
    op: "receipt_preferences_set";
    request_id: string;
    conversation_id: string;
    delivery: boolean;
    read: boolean;
} | {
    op: "health";
};
export interface Identity {
    id: string;
    alias: string;
    capabilities: string[];
}
export type MessageAudience = "all" | string[];
export interface AttachmentRef {
    id: string;
    content_type: string;
    size_bytes: number;
}
export interface MessageDraft {
    to: MessageAudience;
    content_type: string;
    body: string;
    body_encoding: "utf8" | "base64";
    label: string;
    attachments: AttachmentRef[];
    in_reply_to_msg: string | null;
    msg_id: string | null;
}
export interface Message {
    id: string;
    conversation_id: string;
    sender: string;
    body: string;
    sender_participant_id: string | null;
    audience: {
        participant_id: string;
        membership_epoch: number;
    }[];
    content_type: string;
    body_encoding: "utf8" | "base64";
    label: string;
    attachments: AttachmentRef[];
    in_reply_to_msg: string | null;
    accepted_at: number | null;
}
export interface Call {
    id: string;
    conversation_id: string;
    route_id: string;
    destination: string;
    state: string;
    direction: string;
    end_requested: boolean;
    provider_id: string | null;
    ended_reason: string | null;
}
export type DeliveryState = "queued" | "accepted" | "delivered" | "read" | "failed" | "unknown";
export interface RecipientReceipt {
    conversation_id: string;
    message_id: string;
    participant_id: string;
    membership_epoch: number;
    channel: "native" | "provider";
    delivery_id: string;
    state: DeliveryState;
    provenance: string;
    observed_at: number;
}
export interface ReceiptPage {
    receipts: RecipientReceipt[];
    next: string | null;
}
export interface ReceiptPreferences {
    conversation_id: string;
    participant_id: string;
    membership_epoch: number;
    delivery: boolean;
    read: boolean;
}
export interface ReceiptAcknowledgement {
    conversation_id: string;
    message_id: string;
    participant_id: string;
    membership_epoch: number;
    recorded: DeliveryState | null;
}
export interface Event {
    id: string;
    kind: string;
    conversation_id: string;
    cursor: string;
    message: Message | null;
    call?: Call | null;
    session?: Session | null;
    artifact?: Artifact | null;
    receipt?: RecipientReceipt | null;
}
export interface Session {
    id: string;
    conversation_id: string;
    initiator_id: string;
    participant_id: string;
    state: string;
    media: string[];
    created_at: number;
    updated_at: number;
    ended_reason: string | null;
}
export interface SipLeg {
    id: string;
    conversation_id: string;
    session_id: string;
    destination: string;
    state: string;
    connection_id: string | null;
    ended_reason: string | null;
    end_requested: boolean;
    created_at: number;
    updated_at: number;
}
export interface Availability {
    identity_id: string;
    state: string;
    preferred_route: string | null;
    expires_at?: number | null;
    observed_at: number | null;
}
export interface ContextItem {
    sequence: number;
    kind: string;
    source_id: string;
    actor_id: string;
    body: string;
    record?: ContextRecord;
}
export interface ContextDelivery {
    id: string;
    session_id: string | null;
    provider: string;
    watermark: number;
    state: string;
    provider_ref: string | null;
    created_at: number;
    updated_at: number;
}
export interface ContextView {
    conversation_id: string;
    watermark: number;
    items: ContextItem[];
    omitted: number;
    deliveries: ContextDelivery[];
    deliveries_omitted?: number;
}
export interface Artifact {
    id: string;
    conversation_id: string;
    owner_id: string;
    name: string;
    media_type: string;
    size: number;
    received: number;
    sha256: string;
    state: string;
    created_at: number;
    expires_at: number | null;
}
/** Resolve path only against the configured Connect server. Requires current credentials. */
export interface ArtifactLink {
    conversation_id: string;
    artifact_id: string;
    message_id: string | null;
    path: string;
    expires_at: number;
    purpose: "upload" | "download";
}
export interface ArtifactChunk {
    artifact: Artifact;
    offset: number;
    data: string;
    eof: boolean;
}
export interface MutationReceipt {
    operation_id: string;
    conversation_id: string;
    state: "completed";
    completion_boundary: "local-mutation";
    status_reference: {
        action: "get";
        operation_id: string;
    };
}
export interface Accepted {
    /** Present for core mutation admission; task effects have task-scoped status. */
    receipt?: MutationReceipt;
    request_id: string;
    message_id: string;
    conversation_id: string;
}
export interface History {
    messages: Message[];
    next: string | null;
}
export interface Inbox {
    events: Event[];
    cursor: string | null;
}
export interface ProviderDelivery {
    refusal_code: string | null;
    sms_decision?: string | null;
    sms_grant_id?: string | null;
    job_id: string;
    message_id: string;
    connector: string;
    state: string;
    provider_id: string | null;
}
export interface ProviderStatus {
    deliveries: ProviderDelivery[];
}
export interface RequestStatus {
    state: string;
    outcome: Outcome | null;
}
export interface Readiness {
    /** Bounded process diagnostics; absent on older servers. */
    media?: MediaDiagnostics;
    state: "ready" | "degraded" | "unavailable" | "uncertain";
    components: Array<{
        component: string;
        state: Readiness["state"];
        reason: string;
        observed_at: number;
        /** Omitted legacy observations describe configured components. */
        configured?: boolean;
    }>;
}
export interface Health {
    installation_id: string;
    schema: number;
    profile: string;
    readiness?: Readiness;
}
export interface OutcomeData {
    membership: MembershipOutcome;
    membership_snapshot: MembershipSnapshot;
    membership_invitations: MembershipInvitations;
    conversation_snapshot: SnapshotPage;
    conversation_events: EventPage;
    session_observations: ObservationPage;
    durable_operation: OperationRecord;
    contact: ContactState;
    contact_pairing: ContactPairing;
    task: TaskOutcome;
    capabilities: Capabilities;
    browser_binding: BrowserBinding;
    context_deliveries: ContextDeliveryPage;
    identity: Identity;
    conversation: Conversation;
    conversations: ConversationPage;
    accepted: Accepted;
    history: History;
    inbox: Inbox;
    call: Call;
    session: Session;
    sip_leg: SipLeg;
    availability: Availability;
    context_view: ContextView;
    artifact: Artifact;
    artifact_chunk: ArtifactChunk;
    artifact_link: ArtifactLink;
    message_receipts: ReceiptPage;
    receipt_preferences: ReceiptPreferences;
    receipt_acknowledged: ReceiptAcknowledgement;
    provider_status: ProviderStatus;
    request_status: RequestStatus;
    health: Health;
}
export type Outcome = {
    [K in keyof OutcomeData]: {
        kind: K;
        data: OutcomeData[K];
    };
}[keyof OutcomeData];
/** Configured capability does not assert live provider or recipient reachability. */
export interface RouteCapability {
    id: string;
    transport: string;
    media: string[];
    operations: string[];
    readiness: "configured" | "unconfigured";
}
export interface Capabilities {
    /** Current participant ID in the selected conversation, when available. */
    participant_id?: string | null;
    extensions?: string[];
    identity_id: string;
    conversation_id: string | null;
    routes: RouteCapability[];
    availability: Availability | null;
    limits: {
        message_bytes: number;
        artifact_bytes: number;
        artifact_chunk_bytes: number;
        page_size: number;
        audio_frame_ms: number;
        audio_codec: string;
        video_profile?: VideoProfile | null;
    };
}
export interface BrowserBinding {
    conversation_id: string;
    participant_id: string;
    session_id: string | null;
    signaling_url: string;
}
export interface ContextDeliveryPage {
    deliveries: ContextDelivery[];
    next: string | null;
}
/** Fixed codec/transport limits for the advertised native/browser VP8 profile. */
export interface VideoProfile {
    profile: string;
    codec: string;
    width: number;
    height: number;
    max_fps: number;
    target_bitrate_kbps: number;
    max_rtp_bytes: number;
    max_frame_bytes: number;
    assembly_timeout_ms: number;
    queue_packets: number;
}
