const num = (v) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const closed = (v, keys) => v !== null &&
    typeof v === "object" &&
    !Array.isArray(v) &&
    Object.keys(v).length === keys.split(" ").length &&
    keys.split(" ").every((k) => k in v);
const reference = (v) => typeof v === "string" &&
    v.trim().length > 0 &&
    Buffer.byteLength(v) <= 512 &&
    !/\p{Cc}/u.test(v);
const nil = (v, f) => v === null || f(v);
function gap(v) {
    return (closed(v, "first_sequence last_sequence observations") &&
        num(v.first_sequence) &&
        v.first_sequence > 0 &&
        num(v.last_sequence) &&
        v.first_sequence <= v.last_sequence &&
        v.observations === v.last_sequence - v.first_sequence + 1);
}
export function validAgent(v) {
    if (!closed(v, "local_sequence capture_complete missing_before provider_session_id provider_event_id captured_at content") ||
        !num(v.local_sequence) ||
        v.local_sequence === 0 ||
        v.capture_complete !== false ||
        !num(v.captured_at) ||
        !nil(v.provider_session_id, reference) ||
        !nil(v.provider_event_id, reference) ||
        !(v.missing_before === null ||
            (gap(v.missing_before) &&
                v.missing_before.last_sequence < v.local_sequence)) ||
        Buffer.byteLength(JSON.stringify(v)) > 65536)
        return false;
    const c = v.content;
    if (c === null || typeof c !== "object" || Array.isArray(c))
        return false;
    const d = c;
    const role = (r) => [null, "user", "assistant", "system", "other"].includes(r);
    const completeness = (x) => ["partial", "final", "unavailable"].includes(String(x));
    switch (d.kind) {
        case "transcript":
            return (closed(d, "kind text speaker_role completeness") &&
                (d.text === null || typeof d.text === "string") &&
                role(d.speaker_role) &&
                completeness(d.completeness));
        case "speech":
            return (closed(d, "kind state speaker_role turn completeness") &&
                nil(d.state, reference) &&
                role(d.speaker_role) &&
                nil(d.turn, num) &&
                completeness(d.completeness));
        case "action":
            return (closed(d, "kind name call_id phase arguments result completeness") &&
                nil(d.name, reference) &&
                nil(d.call_id, reference) &&
                ["requested", "result-observed", "unknown"].includes(String(d.phase)) &&
                completeness(d.completeness));
        case "provider-status":
            return closed(d, "kind state") && reference(d.state);
        case "unavailable":
            return (closed(d, "kind issue") &&
                [
                    "missing-fields",
                    "malformed-event",
                    "mismatched-session",
                    "unsupported-event",
                    "sequence-exhausted",
                ].includes(String(d.issue)));
        case "gap":
            return (closed(d, "kind first_sequence last_sequence observations") &&
                gap({
                    first_sequence: d.first_sequence,
                    last_sequence: d.last_sequence,
                    observations: d.observations,
                }) &&
                d.last_sequence === v.local_sequence);
        default:
            return false;
    }
}
export function agentSupported(v, cap) {
    if (!validAgent(v))
        return false;
    const c = v.content;
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
