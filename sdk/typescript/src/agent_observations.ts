/** Provider roles/references and reported results are data, never authority. */
export type AgentCompleteness = "partial" | "final" | "unavailable";
export type AgentSpeakerRole = "user" | "assistant" | "system" | "other";
export type AgentActionPhase = "requested" | "result-observed" | "unknown";
export type AgentObservationIssue =
    | "missing-fields"
    | "malformed-event"
    | "mismatched-session"
    | "unsupported-event"
    | "sequence-exhausted";
export interface AgentObservationGap {
    first_sequence: number;
    last_sequence: number;
    observations: number;
}
export type AgentObservationContent =
    | {
          kind: "transcript";
          text: string | null;
          speaker_role: AgentSpeakerRole | null;
          completeness: AgentCompleteness;
      }
    | {
          kind: "speech";
          state: string | null;
          speaker_role: AgentSpeakerRole | null;
          turn: number | null;
          completeness: AgentCompleteness;
      }
    | {
          kind: "action";
          name: string | null;
          call_id: string | null;
          phase: AgentActionPhase;
          arguments: unknown;
          result: unknown;
          completeness: AgentCompleteness;
      }
    | { kind: "provider-status"; state: string }
    | { kind: "unavailable"; issue: AgentObservationIssue }
    | ({ kind: "gap" } & AgentObservationGap);
export interface AgentObservation {
    local_sequence: number;
    capture_complete: false;
    missing_before: AgentObservationGap | null;
    provider_session_id: string | null;
    provider_event_id: string | null;
    captured_at: number;
    content: AgentObservationContent;
}
const num = (v: unknown): v is number =>
    typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const closed = (v: unknown, keys: string): v is Record<string, unknown> =>
    v !== null &&
    typeof v === "object" &&
    !Array.isArray(v) &&
    Object.keys(v).length === keys.split(" ").length &&
    keys.split(" ").every((k) => k in v);
const reference = (v: unknown) =>
    typeof v === "string" &&
    v.trim().length > 0 &&
    Buffer.byteLength(v) <= 512 &&
    !/\p{Cc}/u.test(v);
const nil = (v: unknown, f: (v: unknown) => boolean) => v === null || f(v);
function gap(v: unknown): v is AgentObservationGap {
    return (
        closed(v, "first_sequence last_sequence observations") &&
        num(v.first_sequence) &&
        v.first_sequence > 0 &&
        num(v.last_sequence) &&
        v.first_sequence <= v.last_sequence &&
        v.observations === v.last_sequence - v.first_sequence + 1
    );
}
export function validAgent(v: unknown): boolean {
    if (
        !closed(
            v,
            "local_sequence capture_complete missing_before provider_session_id provider_event_id captured_at content",
        ) ||
        !num(v.local_sequence) ||
        v.local_sequence === 0 ||
        v.capture_complete !== false ||
        !num(v.captured_at) ||
        !nil(v.provider_session_id, reference) ||
        !nil(v.provider_event_id, reference) ||
        !(
            v.missing_before === null ||
            (gap(v.missing_before) &&
                v.missing_before.last_sequence < v.local_sequence)
        ) ||
        Buffer.byteLength(JSON.stringify(v)) > 65536
    )
        return false;
    const c = v.content;
    if (c === null || typeof c !== "object" || Array.isArray(c)) return false;
    const d = c as Record<string, unknown>;
    const role = (r: unknown) =>
        [null, "user", "assistant", "system", "other"].includes(
            r as string | null,
        );
    const completeness = (x: unknown) =>
        ["partial", "final", "unavailable"].includes(String(x));
    switch (d.kind) {
        case "transcript":
            return (
                closed(d, "kind text speaker_role completeness") &&
                (d.text === null || typeof d.text === "string") &&
                role(d.speaker_role) &&
                completeness(d.completeness)
            );
        case "speech":
            return (
                closed(d, "kind state speaker_role turn completeness") &&
                nil(d.state, reference) &&
                role(d.speaker_role) &&
                nil(d.turn, num) &&
                completeness(d.completeness)
            );
        case "action":
            return (
                closed(
                    d,
                    "kind name call_id phase arguments result completeness",
                ) &&
                nil(d.name, reference) &&
                nil(d.call_id, reference) &&
                ["requested", "result-observed", "unknown"].includes(
                    String(d.phase),
                ) &&
                completeness(d.completeness)
            );
        case "provider-status":
            return closed(d, "kind state") && reference(d.state);
        case "unavailable":
            return (
                closed(d, "kind issue") &&
                [
                    "missing-fields",
                    "malformed-event",
                    "mismatched-session",
                    "unsupported-event",
                    "sequence-exhausted",
                ].includes(String(d.issue))
            );
        case "gap":
            return (
                closed(d, "kind first_sequence last_sequence observations") &&
                gap({
                    first_sequence: d.first_sequence,
                    last_sequence: d.last_sequence,
                    observations: d.observations,
                }) &&
                d.last_sequence === v.local_sequence
            );
        default:
            return false;
    }
}
export function agentSupported(
    v: unknown,
    cap: Record<string, unknown>,
): boolean {
    if (!validAgent(v)) return false;
    const c = (v as unknown as AgentObservation).content;
    return c.kind === "transcript"
        ? cap.transcript === true
        : c.kind === "speech"
          ? cap.speech === true
          : c.kind === "action"
            ? Array.isArray(cap.actions) &&
              cap.actions.length > 0 &&
              (c.name === null || cap.actions.includes(c.name))
            : true;
}
