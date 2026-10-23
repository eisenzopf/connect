/** Provider roles/references and reported results are data, never authority. */
export type AgentCompleteness = "partial" | "final" | "unavailable";
export type AgentSpeakerRole = "user" | "assistant" | "system" | "other";
export type AgentActionPhase = "requested" | "result-observed" | "unknown";
export type AgentObservationIssue = "missing-fields" | "malformed-event" | "mismatched-session" | "unsupported-event" | "sequence-exhausted";
export interface AgentObservationGap {
    first_sequence: number;
    last_sequence: number;
    observations: number;
}
export type AgentObservationContent = {
    kind: "transcript";
    text: string | null;
    speaker_role: AgentSpeakerRole | null;
    completeness: AgentCompleteness;
} | {
    kind: "speech";
    state: string | null;
    speaker_role: AgentSpeakerRole | null;
    turn: number | null;
    completeness: AgentCompleteness;
} | {
    kind: "action";
    name: string | null;
    call_id: string | null;
    phase: AgentActionPhase;
    arguments: unknown;
    result: unknown;
    completeness: AgentCompleteness;
} | {
    kind: "provider-status";
    state: string;
} | {
    kind: "unavailable";
    issue: AgentObservationIssue;
} | ({
    kind: "gap";
} & AgentObservationGap);
export interface AgentObservation {
    local_sequence: number;
    capture_complete: false;
    missing_before: AgentObservationGap | null;
    provider_session_id: string | null;
    provider_event_id: string | null;
    captured_at: number;
    content: AgentObservationContent;
}
export declare function validAgent(v: unknown): boolean;
export declare function agentSupported(v: unknown, cap: Record<string, unknown>): boolean;
