export type Operation = {
    op: "resolve";
    address: string;
} | {
    op: "capabilities";
    address: string;
    conversation_id: string | null;
} | {
    op: "browser_binding";
} | {
    op: "context_deliveries";
    conversation_id: string;
    before: string | null;
    limit: number;
} | {
    op: "create";
    request_id: string;
    participants: string[];
} | {
    op: "send";
    request_id: string;
    conversation_id: string;
    body: string;
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
    op: "health";
};
export interface Identity {
    id: string;
    alias: string;
    capabilities: string[];
}
export interface Conversation {
    id: string;
    participants: string[];
    closed: boolean;
}
export interface Message {
    id: string;
    conversation_id: string;
    sender: string;
    body: string;
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
export interface Event {
    id: string;
    kind: string;
    conversation_id: string;
    cursor: string;
    message: Message | null;
    call?: Call | null;
    session?: Session | null;
    artifact?: Artifact | null;
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
export interface ArtifactChunk {
    artifact: Artifact;
    offset: number;
    data: string;
    eof: boolean;
}
export interface Accepted {
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
    state: "ready" | "degraded" | "unavailable" | "uncertain";
    components: Array<{
        component: string;
        state: Readiness["state"];
        reason: string;
        observed_at: number;
    }>;
}
export interface Health {
    installation_id: string;
    schema: number;
    profile: string;
    readiness?: Readiness;
}
export interface OutcomeData {
    capabilities: Capabilities;
    browser_binding: BrowserBinding;
    context_deliveries: ContextDeliveryPage;
    identity: Identity;
    conversation: Conversation;
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
