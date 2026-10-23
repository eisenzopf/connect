import { validAgent, agentSupported } from "./agent_observations.js";
import { object, integer } from "./validation.js";
type Validator = (kind: unknown, data: unknown, depth: number) => boolean;
type Obj = Record<string, unknown>;
const states = [
    "accepted",
    "preparing",
    "ready",
    "committing",
    "completed",
    "cancelled",
    "failed",
    "interrupted",
    "unknown",
];
const text = (v: unknown, max = 512): v is string =>
    typeof v === "string" &&
    v.length > 0 &&
    v.length <= max &&
    !/[\x00-\x1f\x7f]/.test(v);
const token = (v: unknown): v is string =>
    typeof v === "string" && v.length <= 2048 && /^ev1_[A-Za-z0-9_-]+$/.test(v);
const nil = (v: unknown, test: (v: unknown) => boolean) =>
    v === null || test(v);
const num = (v: unknown) => integer(v);
const epoch = (v: unknown) => num(v) && Number(v) > 0;
function closed(v: unknown, keys: string): v is Obj {
    const names = keys.split(" ");
    return (
        object(v) &&
        Object.keys(v).length === names.length &&
        names.every((k) => k in v)
    );
}
function list(
    v: unknown,
    max: number,
    test: (v: unknown) => boolean,
): v is unknown[] {
    return Array.isArray(v) && v.length <= max && v.every(test);
}
function source(v: unknown): v is Obj {
    return (
        closed(v, "kind correlation_id") &&
        text(v.kind, 128) &&
        text(v.correlation_id)
    );
}
function audience(v: unknown): boolean {
    return (
        closed(v, "participant_id membership_epoch") &&
        text(v.participant_id) &&
        epoch(v.membership_epoch)
    );
}
function base(
    kind: string,
    v: unknown,
    validate: Validator,
    depth: number,
): boolean {
    const keys: Record<string, string> = {
        session:
            "id conversation_id initiator_id participant_id state media created_at updated_at ended_reason",
        artifact:
            "id conversation_id owner_id name media_type size received sha256 state created_at expires_at",
        call: "id conversation_id route_id destination state provider_id ended_reason direction end_requested",
        message:
            "id conversation_id sender body sender_participant_id audience content_type body_encoding label attachments in_reply_to_msg accepted_at",
    };
    if (!closed(v, keys[kind])) return false;
    if (kind === "message")
        return (
            validate("history", { messages: [v], next: null }, depth + 1) &&
            list(v.audience, 100, audience) &&
            list(
                v.attachments,
                100,
                (a) =>
                    closed(a, "id content_type size_bytes") &&
                    text(a.id) &&
                    text(a.content_type, 128) &&
                    num(a.size_bytes),
            )
        );
    return validate(kind, v, depth + 1);
}
function operation(v: unknown, validate: Validator, depth: number): boolean {
    return (
        closed(
            v,
            "operation_id record_id command_type conversation_id session_id connection_id expected_revision state revision expires_at created_at updated_at cancel_requested result effects causal_event_ids events_omitted",
        ) &&
        validate("durable_operation", v, depth + 1) &&
        list(v.effects, 64, (e) =>
            closed(
                e,
                "id provider correlation_key idempotency state attempt compensates_effect_id submission_generation",
            ),
        ) &&
        list(v.causal_event_ids, 100, (x) => text(x))
    );
}
function providerCall(v: unknown): v is Obj {
    return (
        closed(
            v,
            "id operation_id conversation_id session_id participant_id membership_epoch endpoint_id state direction remote_identity_verified revision observed_at closed_at ended_reason problem local_media",
        ) &&
        [
            v.id,
            v.operation_id,
            v.conversation_id,
            v.session_id,
            v.participant_id,
            v.endpoint_id,
        ].every((x) => text(x)) &&
        epoch(v.membership_epoch) &&
        epoch(v.revision) &&
        ["starting", "active", "ended", "failed", "unknown"].includes(
            String(v.state),
        ) &&
        nil(v.observed_at, num) &&
        nil(v.closed_at, num) &&
        nil(
            v.ended_reason,
            (x) =>
                typeof x === "string" &&
                x.length <= 256 &&
                !/[\x00-\x1f\x7f]/.test(x),
        ) &&
        nil(v.problem, (x) => text(x, 128)) &&
        ["inbound", "outbound"].includes(String(v.direction)) &&
        v.remote_identity_verified === false &&
        v.local_media === false
    );
}
function sessionSnapshot(v: unknown): v is Obj {
    return (
        closed(
            v,
            "id conversation_id state media intent capabilities_offer codec_preferences is_main revision topology_revision created_at updated_at ended_reason",
        ) &&
        text(v.id) &&
        text(v.conversation_id) &&
        text(v.state, 128) &&
        list(v.media, 16, (x) => text(x)) &&
        nil(v.intent, (x) => text(x, 128)) &&
        nil(v.capabilities_offer, object) &&
        list(v.codec_preferences, 16, (x) => text(x)) &&
        typeof v.is_main === "boolean" &&
        num(v.revision) &&
        num(v.topology_revision) &&
        num(v.created_at) &&
        num(v.updated_at) &&
        nil(v.ended_reason, closureReason)
    );
}
function closureReason(v: unknown): v is string {
    return (
        typeof v === "string" &&
        !/[\x00-\x1f\x7f-\x9f]/u.test(v) &&
        Array.from(v).every((c) => {
            const n = c.codePointAt(0)!;
            return n < 0xd800 || n > 0xdfff;
        }) &&
        new TextEncoder().encode(v).length <= 512
    );
}
function item(v: unknown, validate: Validator, depth: number): boolean {
    if (!closed(v, "kind data") || !object(v.data)) return false;
    const d = v.data;
    switch (v.kind) {
        case "provider_call":
            return providerCall(d);
        case "conversation":
            return (
                closed(d, "conversation_id revision closed policy") &&
                text(d.conversation_id) &&
                num(d.revision) &&
                typeof d.closed === "boolean" &&
                ["persistent", "ephemeral"].includes(String(d.policy))
            );
        case "participant":
            return (
                closed(
                    d,
                    "participant_id conversation_id identity_id kind role membership_epoch active removal_reason",
                ) &&
                [d.participant_id, d.conversation_id, d.identity_id].every(
                    (x) => text(x),
                ) &&
                ["human", "ai", "system", "external"].includes(
                    String(d.kind),
                ) &&
                text(d.role, 64) &&
                epoch(d.membership_epoch) &&
                typeof d.active === "boolean" &&
                nil(
                    d.removal_reason,
                    (x) => typeof x === "string" && x.length <= 512,
                )
            );
        case "session":
            return sessionSnapshot(d);
        case "session_participant":
            return (
                closed(
                    d,
                    "session_id participant_id membership_epoch role state",
                ) &&
                text(d.session_id) &&
                text(d.participant_id) &&
                epoch(d.membership_epoch) &&
                text(d.role, 64) &&
                ["invited", "joining", "present", "left", "removed"].includes(
                    String(d.state),
                )
            );
        case "connection":
            return (
                closed(
                    d,
                    "connection_id session_id participant_id membership_epoch endpoint_id device_id generation revision state replaces",
                ) &&
                [
                    d.connection_id,
                    d.session_id,
                    d.participant_id,
                    d.endpoint_id,
                    d.device_id,
                ].every((x) => text(x)) &&
                epoch(d.membership_epoch) &&
                epoch(d.generation) &&
                num(d.revision) &&
                [
                    "preparing",
                    "ready",
                    "active",
                    "held",
                    "retiring",
                    "retired",
                    "interrupted",
                    "unknown",
                    "failed",
                ].includes(String(d.state)) &&
                nil(d.replaces, (x) => text(x))
            );
        case "stream":
            return (
                closed(
                    d,
                    "stream_id session_id participant_id connection_id medium state revision",
                ) &&
                [
                    d.stream_id,
                    d.session_id,
                    d.participant_id,
                    d.medium,
                    d.state,
                ].every((x) => text(x)) &&
                nil(d.connection_id, (x) => text(x)) &&
                num(d.revision)
            );
        case "operation":
            return operation(d, validate, depth + 1);
        default:
            return false;
    }
}
function snapshot(v: unknown, validate: Validator, depth: number): v is Obj {
    return (
        closed(
            v,
            "conversation_id snapshot_id revision cursor expires_at items next has_more",
        ) &&
        text(v.conversation_id) &&
        text(v.snapshot_id) &&
        num(v.revision) &&
        num(v.expires_at) &&
        token(v.cursor) &&
        nil(v.next, token) &&
        typeof v.has_more === "boolean" &&
        v.has_more === (v.next !== null) &&
        list(
            v.items,
            500,
            (x) =>
                item(x, validate, depth + 1) &&
                object(x) &&
                object(x.data) &&
                (!("conversation_id" in x.data) ||
                    x.data.conversation_id === v.conversation_id),
        )
    );
}
function receipt(d: unknown): boolean {
    return (
        closed(
            d,
            "conversation_id message_id participant_id membership_epoch channel delivery_id state provenance observed_at",
        ) &&
        [
            d.conversation_id,
            d.message_id,
            d.participant_id,
            d.delivery_id,
        ].every((x) => text(x)) &&
        epoch(d.membership_epoch) &&
        num(d.observed_at) &&
        ["native", "provider"].includes(String(d.channel)) &&
        [
            "queued",
            "accepted",
            "delivered",
            "read",
            "failed",
            "unknown",
        ].includes(String(d.state)) &&
        [
            "journal",
            "client_reported",
            "provider_observation",
            "provider_state",
            "local_attempt",
        ].includes(String(d.provenance)) &&
        (d.state !== "read" ||
            (d.channel === "native" && d.provenance === "client_reported"))
    );
}
function journalData(v: unknown, validate: Validator, depth: number): boolean {
    if (!closed(v, "kind data") || !object(v.data)) return false;
    const d = v.data;
    if (["message", "session", "artifact", "call"].includes(String(v.kind)))
        return base(String(v.kind), d, validate, depth + 1);
    switch (v.kind) {
        case "provider_call":
            return providerCall(d);
        case "connection_prepared":
            return (
                closed(
                    d,
                    "connection_id conversation_id session_id participant_id membership_epoch endpoint_id generation revision state",
                ) &&
                [
                    d.connection_id,
                    d.conversation_id,
                    d.session_id,
                    d.participant_id,
                    d.endpoint_id,
                ].every((x) => text(x)) &&
                epoch(d.membership_epoch) &&
                d.generation === 1 &&
                d.revision === 0 &&
                d.state === "preparing"
            );
        case "connection_ready":
        case "connection_retired":
            return (
                item({ kind: "connection", data: d }, validate, depth + 1) &&
                d.state ===
                    (v.kind === "connection_ready" ? "ready" : "retired") &&
                Number(d.generation) >=
                    (v.kind === "connection_ready" ? 1 : 2) &&
                Number(d.revision) > 0
            );
        case "native_data_write":
            return (
                closed(
                    d,
                    "message_id source_connection_id target_connection_id participant_id membership_epoch generation disposition provenance observed_at",
                ) &&
                [
                    "message_id",
                    "source_connection_id",
                    "target_connection_id",
                    "participant_id",
                ].every((k) => text(d[k])) &&
                epoch(d.membership_epoch) &&
                epoch(d.generation) &&
                num(d.observed_at) &&
                ["not_started", "local_written", "submission_unknown"].includes(
                    String(d.disposition),
                ) &&
                ["local_writer", "runtime_recovery"].includes(
                    String(d.provenance),
                ) &&
                (d.provenance !== "runtime_recovery" ||
                    d.disposition === "submission_unknown")
            );
        case "receipt":
            return receipt(d);
        case "membership":
            return (
                closed(
                    d,
                    "participant_id identity_id role active membership_epoch removal_reason",
                ) &&
                nil(d.participant_id, (x) => text(x)) &&
                nil(d.identity_id, (x) => text(x)) &&
                nil(d.role, (x) => text(x, 64)) &&
                nil(d.active, (x) => typeof x === "boolean") &&
                nil(d.membership_epoch, epoch) &&
                nil(
                    d.removal_reason,
                    (x) => typeof x === "string" && x.length <= 512,
                )
            );
        case "operation":
            return (
                closed(d, "operation_id record_id state") &&
                text(d.operation_id) &&
                text(d.record_id) &&
                states.includes(String(d.state))
            );
        case "session_control":
            return sessionSnapshot(d);
        case "session_participant_joined":
            return (
                closed(
                    d,
                    "fact_id session membership_epoch participant via_connection",
                ) &&
                text(d.fact_id) &&
                sessionSnapshot(d.session) &&
                d.session.state === "active" &&
                epoch(d.membership_epoch) &&
                text(d.via_connection) &&
                closed(
                    d.participant,
                    "participant_id identity_id kind role display_name joined_at",
                ) &&
                text(d.participant.participant_id) &&
                text(d.participant.identity_id) &&
                ["human", "ai", "system", "external"].includes(
                    String(d.participant.kind),
                ) &&
                text(d.participant.role, 64) &&
                text(d.participant.display_name, 256) &&
                epoch(d.participant.joined_at)
            );
        case "session_participant_left":
            return (
                closed(
                    d,
                    "fact_id session membership_epoch participant_id left_at reason",
                ) &&
                text(d.fact_id) &&
                sessionSnapshot(d.session) &&
                epoch(d.membership_epoch) &&
                text(d.participant_id) &&
                epoch(d.left_at) &&
                [
                    "explicit-leave",
                    "all-connections-ended",
                    "kicked",
                    "transferred-out",
                ].includes(String(d.reason))
            );
        case "session_started":
            return (
                closed(
                    d,
                    "session started_at participants_present active_connections negotiated_capabilities",
                ) &&
                sessionSnapshot(d.session) &&
                d.session.state === "active" &&
                epoch(d.started_at) &&
                list(d.participants_present, 256, (x) => text(x)) &&
                d.participants_present.length > 0 &&
                new Set(d.participants_present).size ===
                    d.participants_present.length &&
                list(
                    d.active_connections,
                    256,
                    (x) =>
                        closed(x, "connid participant_id transport") &&
                        text(x.connid) &&
                        text(x.participant_id) &&
                        Array.isArray(d.participants_present) &&
                        d.participants_present.includes(x.participant_id) &&
                        [
                            "quic",
                            "webtransport",
                            "websocket+webrtc",
                            "sip-interop",
                            "webrtc-interop",
                        ].includes(String(x.transport)),
                ) &&
                d.active_connections.length > 0 &&
                new Set(
                    d.active_connections.map((x) =>
                        object(x) ? x.connid : null,
                    ),
                ).size === d.active_connections.length &&
                object(d.negotiated_capabilities)
            );
        case "session_ended":
            return (
                closed(
                    d,
                    "session by_participant by_membership_epoch reason_code reason ended_at",
                ) &&
                sessionSnapshot(d.session) &&
                d.session.state === "ended" &&
                d.reason === (d.session.ended_reason ?? "unspecified") &&
                nil(d.by_participant, (x) => text(x)) &&
                nil(d.by_membership_epoch, epoch) &&
                (d.by_participant === null) ===
                    (d.by_membership_epoch === null) &&
                num(d.reason_code) &&
                Number(d.reason_code) <= 65535 &&
                closureReason(d.reason) &&
                epoch(d.ended_at)
            );
        case "conversation_closed":
            return (
                closed(d, "conversation_id reason_code reason closed_at") &&
                text(d.conversation_id) &&
                num(d.reason_code) &&
                num(d.closed_at) &&
                Number(d.closed_at) > 0 &&
                closureReason(d.reason)
            );
        case "changed":
            return (
                closed(d, "resource_kind resource_id") &&
                nil(d.resource_kind, (x) => text(x)) &&
                nil(d.resource_id, (x) => text(x))
            );
        default:
            return false;
    }
}
function event(
    v: unknown,
    cid: unknown,
    validate: Validator,
    depth: number,
): boolean {
    return (
        closed(
            v,
            "event_id cursor conversation_id session_id connection_id participant_id operation_id kind revision source observed_at audience data",
        ) &&
        text(v.event_id) &&
        token(v.cursor) &&
        v.conversation_id === cid &&
        [v.session_id, v.connection_id, v.participant_id, v.operation_id].every(
            (x) => nil(x, (y) => text(y)),
        ) &&
        text(v.kind, 128) &&
        num(v.revision) &&
        source(v.source) &&
        num(v.observed_at) &&
        list(v.audience, 100, audience) &&
        v.audience.length > 0 &&
        journalData(v.data, validate, depth + 1) &&
        object(v.data) &&
        object(v.data.data) &&
        (v.data.kind !== "session_control" ||
            (v.kind === "session_control" &&
                object(v.data.data) &&
                v.data.data.id === v.session_id &&
                v.data.data.conversation_id === cid &&
                v.data.data.revision === v.revision)) &&
        (!["session_participant_joined", "session_participant_left"].includes(
            String(v.data.kind),
        ) ||
            (v.kind === v.data.kind &&
                v.connection_id === null &&
                object(v.data.data.session) &&
                v.data.data.session.id === v.session_id &&
                v.data.data.session.conversation_id === cid &&
                v.data.data.session.revision === v.revision &&
                (v.data.kind === "session_participant_joined"
                    ? object(v.data.data.participant) &&
                      v.data.data.participant.participant_id ===
                          v.participant_id &&
                      Number(v.data.data.participant.joined_at) <=
                          Number(v.observed_at)
                    : v.data.data.participant_id === v.participant_id &&
                      Number(v.data.data.left_at) <= Number(v.observed_at)))) &&
        (v.data.kind !== "session_started" ||
            (v.kind === "session_started" &&
                v.connection_id === null &&
                v.operation_id === null &&
                object(v.data.data.session) &&
                v.data.data.session.id === v.session_id &&
                v.data.data.session.conversation_id === cid &&
                v.data.data.session.revision === v.revision &&
                Number(v.data.data.started_at) <= Number(v.observed_at))) &&
        (v.data.kind !== "session_ended" ||
            (["session_control", "session_ended"].includes(String(v.kind)) &&
                v.connection_id === null &&
                object(v.data.data.session) &&
                v.data.data.session.id === v.session_id &&
                v.data.data.session.conversation_id === cid &&
                v.data.data.session.revision === v.revision &&
                Number(v.data.data.ended_at) <= Number(v.observed_at))) &&
        (v.data.kind !== "conversation_closed" ||
            (v.kind === "conversation_closed" &&
                v.session_id === null &&
                v.connection_id === null &&
                Number(v.data.data.closed_at) <= Number(v.observed_at))) &&
        (v.data.kind !== "native_data_write" ||
            (v.kind === "native_data_write" &&
                v.operation_id !== null &&
                v.session_id !== null &&
                v.data.data.target_connection_id === v.connection_id &&
                v.data.data.participant_id === v.participant_id &&
                v.data.data.observed_at === v.observed_at)) &&
        (v.data.kind !== "provider_call" ||
            (v.kind === "provider_voice" &&
                v.connection_id === null &&
                v.data.data.session_id === v.session_id &&
                v.data.data.participant_id === v.participant_id &&
                v.data.data.operation_id === v.operation_id &&
                v.data.data.revision === v.revision)) &&
        (![
            "connection_prepared",
            "connection_ready",
            "connection_retired",
        ].includes(String(v.data.kind)) ||
            (v.data.data.session_id === v.session_id &&
                v.data.data.connection_id === v.connection_id &&
                v.data.data.participant_id === v.participant_id)) &&
        (!("conversation_id" in v.data.data) ||
            v.data.data.conversation_id === cid)
    );
}
function capabilities(v: unknown): boolean {
    return (
        closed(v, "transcript speech actions") &&
        typeof v.transcript === "boolean" &&
        typeof v.speech === "boolean" &&
        list(v.actions, 16, (x) => text(x, 128)) &&
        new Set(v.actions).size === v.actions.length
    );
}
function work(v: unknown): boolean {
    return (
        closed(v, "task_id effect_id question_id") &&
        Object.values(v).every((x) => text(x))
    );
}
function sameWork(a: unknown, b: unknown): boolean {
    return a === null
        ? b === null
        : work(a) &&
              work(b) &&
              Object.keys(a as Obj).every(
                  (k) => (a as Obj)[k] === (b as Obj)[k],
              );
}
function route(v: unknown): v is Obj {
    return (
        closed(
            v,
            "work correlation_id participant_id connection_id generation source capabilities",
        ) &&
        nil(v.work, work) &&
        [v.correlation_id, v.participant_id, v.connection_id, v.source].every(
            (x) => text(x, 128),
        ) &&
        epoch(v.generation) &&
        capabilities(v.capabilities)
    );
}
function draft(v: unknown): v is Obj {
    if (!object(v)) return false;
    switch (v.kind) {
        case "transcript":
            return (
                closed(v, "kind text completeness turn_id fragment_id") &&
                typeof v.text === "string" &&
                v.text.length > 0 &&
                ["interim", "final", "incomplete"].includes(
                    String(v.completeness),
                ) &&
                nil(v.turn_id, (x) => text(x, 128)) &&
                nil(v.fragment_id, (x) => text(x, 128))
            );
        case "speech":
            return (
                closed(v, "kind state turn_id") &&
                ["started", "stopped"].includes(String(v.state)) &&
                nil(v.turn_id, (x) => text(x, 128))
            );
        case "agent":
            return closed(v, "kind observation") && validAgent(v.observation);
        case "action-request":
            return (
                closed(
                    v,
                    "kind action provider_tool_call_id arguments turn_id",
                ) &&
                text(v.action, 128) &&
                text(v.provider_tool_call_id, 128) &&
                object(v.arguments) &&
                nil(v.turn_id, (x) => text(x, 128))
            );
        default:
            return false;
    }
}
function observation(v: unknown, page: Obj): boolean {
    if (
        !closed(
            v,
            "work observation_id conversation_id session_id participant_id membership_epoch connection_id generation source observed_at data",
        ) ||
        !nil(v.work, work) ||
        !text(v.observation_id) ||
        v.conversation_id !== page.conversation_id ||
        v.session_id !== page.session_id ||
        !text(v.participant_id) ||
        !text(v.connection_id) ||
        !epoch(v.membership_epoch) ||
        !epoch(v.generation) ||
        !source(v.source) ||
        !num(v.observed_at) ||
        !draft(v.data)
    )
        return false;
    if (
        Buffer.byteLength(JSON.stringify(v.data)) >
        (v.data.kind === "agent" ? 70 * 1024 : 16 * 1024)
    )
        return false;
    const routes = page.routes as Obj[];
    const data = v.data;
    const s = v.source;
    return routes.some(
        (r) =>
            sameWork(r.work, v.work) &&
            r.connection_id === v.connection_id &&
            r.generation === v.generation &&
            r.participant_id === v.participant_id &&
            r.source === s.kind &&
            r.correlation_id === s.correlation_id &&
            object(r.capabilities) &&
            (data.kind === "agent"
                ? agentSupported(data.observation, r.capabilities)
                : data.kind === "transcript"
                  ? r.capabilities.transcript
                  : data.kind === "speech"
                    ? r.capabilities.speech
                    : Array.isArray(r.capabilities.actions) &&
                      r.capabilities.actions.includes(data.action)),
    );
}
function validEventBody(
    kind: unknown,
    v: unknown,
    validate: Validator,
    depth: number,
): boolean {
    if (depth > 8) return false;
    if (kind === "conversation_snapshot")
        return snapshot(v, validate, depth + 1);
    if (kind === "conversation_events")
        return (
            closed(
                v,
                "conversation_id events cursor has_more lease expires_at resync_required snapshot",
            ) &&
            text(v.conversation_id) &&
            token(v.cursor) &&
            typeof v.has_more === "boolean" &&
            typeof v.resync_required === "boolean" &&
            nil(v.lease, token) &&
            nil(v.expires_at, num) &&
            (v.lease === null) === (v.expires_at === null) &&
            list(v.events, 256, (x) =>
                event(x, v.conversation_id, validate, depth + 1),
            ) &&
            (v.snapshot === null ||
                (snapshot(v.snapshot, validate, depth + 1) &&
                    v.snapshot.conversation_id === v.conversation_id &&
                    v.snapshot.cursor === v.cursor &&
                    v.events.length === 0)) &&
            (!v.resync_required || v.snapshot !== null)
        );
    if (kind === "session_observations")
        return (
            closed(
                v,
                "conversation_id session_id observations expires_at routes cursor has_more resync_required",
            ) &&
            text(v.conversation_id) &&
            text(v.session_id) &&
            token(v.cursor) &&
            num(v.expires_at) &&
            typeof v.has_more === "boolean" &&
            typeof v.resync_required === "boolean" &&
            list(v.routes, 128, route) &&
            list(v.observations, 256, (x) => observation(x, v))
        );
    return false;
}

function safeJson(value: unknown): boolean {
    const pending: unknown[] = [value];
    while (pending.length) {
        const v = pending.pop();
        if (v === null || typeof v === "boolean") continue;
        if (typeof v === "number") {
            if (!Number.isFinite(v)) return false;
            continue;
        }
        if (typeof v === "string") {
            if (Buffer.from(v, "utf8").toString("utf8") !== v) return false;
            continue;
        }
        if (Array.isArray(v)) {
            pending.push(...v);
            continue;
        }
        if (object(v)) {
            pending.push(...Object.keys(v), ...Object.values(v));
            continue;
        }
        return false;
    }
    return true;
}
export function validEvents(
    kind: unknown,
    v: unknown,
    validate: Validator,
    depth: number,
): boolean {
    try {
        return (
            Buffer.byteLength(JSON.stringify(v)) <= 512 * 1024 &&
            safeJson(v) &&
            validEventBody(kind, v, validate, depth)
        );
    } catch {
        return false;
    }
}
