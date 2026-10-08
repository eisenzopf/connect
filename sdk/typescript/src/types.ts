export type Operation =
    | { op: "resolve"; address: string }
    | { op: "create"; request_id: string; participants: string[] }
    | { op: "send"; request_id: string; conversation_id: string; body: string }
    | {
          op: "history";
          conversation_id: string;
          after: string | null;
          limit: number;
      }
    | { op: "inbox"; cursor: string | null; limit: number; wait_ms: number }
    | { op: "request_status"; request_id: string }
    | { op: "provider_status"; conversation_id: string }
    | { op: "close"; request_id: string; conversation_id: string }
    | {
          op: "call_start";
          request_id: string;
          conversation_id: string;
          route_id: string;
          destination: string;
      }
    | { op: "call_status"; conversation_id: string; call_id: string }
    | {
          op: "call_end";
          request_id: string;
          conversation_id: string;
          call_id: string;
      }
    | { op: "health" };

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
export interface Health {
    installation_id: string;
    schema: number;
    profile: string;
}
export interface OutcomeData {
    identity: Identity;
    conversation: Conversation;
    accepted: Accepted;
    history: History;
    inbox: Inbox;
    call: Call;
    provider_status: ProviderStatus;
    request_status: RequestStatus;
    health: Health;
}
export type Outcome = {
    [K in keyof OutcomeData]: { kind: K; data: OutcomeData[K] };
}[keyof OutcomeData];
