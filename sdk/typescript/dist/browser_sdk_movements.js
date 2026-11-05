import { replacementVideo, } from "./replacement_video.js";
import { fields } from "./browser_sdk_sessions.js";
import { nativeCut } from "./browser_native_activation.js";
import { canonicalJson, jsonObject, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
export const movementCommand = (value) => ["session.handoff", "connection.replace"].includes(value);
const positive = (value) => typeof value === "number" && Number.isSafeInteger(value) && value > 0;
const revision = (value) => typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value < Number.MAX_SAFE_INTEGER;
function savedRole(value, bad) {
    if (!jsonObject(value))
        return bad();
    if (value.scope === "session")
        fields(value, ["scope"], bad);
    else if (value.scope === "conversation" &&
        revision(value.expected_conversation_revision))
        fields(value, ["scope", "expected_conversation_revision"], bad);
    else
        return bad();
    return value;
}
export function movementContinuation(value, key, bad) {
    if (!value ||
        !jsonObject(value) ||
        !identifier(value.retained_connection_id) ||
        value.retained_connection_id === key.scope.connectionId)
        return bad();
    if (key.command === "session.handoff") {
        fields(value, [
            "kind",
            "retained_connection_id",
            "source_participant",
            "target_participant",
            "candidate",
            "role",
            "outgoing",
        ], bad);
        if (value.kind !== "handoff" ||
            !identifier(value.source_participant) ||
            !identifier(value.target_participant) ||
            value.source_participant === value.target_participant ||
            !["leave", "observe"].includes(value.outgoing) ||
            (value.candidate !== undefined &&
                value.candidate !== null &&
                (!identifier(value.candidate) ||
                    value.candidate === key.scope.connectionId ||
                    value.candidate === value.retained_connection_id)))
            return bad();
        savedRole(value.role, bad);
    }
    else {
        fields(value, ["kind", "retained_connection_id", "target"], bad);
        if (key.command !== "connection.replace" ||
            value.kind !== "replacement" ||
            !jsonObject(value.target))
            return bad();
        fields(value.target, ["kind", "id"], bad);
        if (!identifier(value.target.id) ||
            !["endpoint", "candidate"].includes(value.target.kind) ||
            (value.target.kind === "candidate" &&
                (value.target.id === key.scope.connectionId ||
                    value.target.id === value.retained_connection_id)))
            return bad();
    }
    return value;
}
export function handoffIntent(input, bad) {
    fields(input, [
        "via",
        "from",
        "to",
        "retain",
        "candidate",
        "outgoing",
        "role",
        "expectedTopologyRevision",
        "consultation",
        "policyRef",
    ], bad);
    if (!identifier(input.via) ||
        !identifier(input.retain) ||
        input.via === input.retain ||
        !identifier(input.from) ||
        !identifier(input.to) ||
        input.from === input.to ||
        !["leave", "observe"].includes(input.outgoing) ||
        !revision(input.expectedTopologyRevision) ||
        (input.policyRef !== undefined && !identifier(input.policyRef)) ||
        (input.candidate !== undefined &&
            (!identifier(input.candidate) ||
                input.candidate === input.via ||
                input.candidate === input.retain)))
        return bad();
    fields(input.role, input.role?.scope === "session"
        ? ["scope"]
        : ["scope", "expectedConversationRevision"], bad);
    const role = input.role?.scope === "session"
        ? { scope: "session" }
        : input.role?.scope === "conversation" &&
            revision(input.role.expectedConversationRevision)
            ? {
                scope: "conversation",
                expected_conversation_revision: input.role.expectedConversationRevision,
            }
            : bad();
    const body = {
        source_participant: input.from,
        target_participant: input.to,
        retained_connid: input.retain,
        outgoing: input.outgoing,
        role_scope: role.scope,
        ...(input.policyRef === undefined
            ? {}
            : { policy_ref: input.policyRef }),
        ...(input.candidate === undefined
            ? {}
            : { candidate_connid: input.candidate }),
        ...(role.expected_conversation_revision === undefined
            ? {}
            : {
                expected_conversation_revision: role.expected_conversation_revision,
            }),
    };
    const context = {
        kind: "handoff",
        retained_connection_id: input.retain,
        source_participant: input.from,
        target_participant: input.to,
        candidate: input.candidate ?? null,
        role,
        outgoing: input.outgoing,
    };
    return {
        body,
        context,
        via: input.via,
        expected: input.expectedTopologyRevision,
    };
}
export function moveIntent(input, via, bad) {
    fields(input, ["retain", "target", "expectedTopologyRevision", "video"], bad);
    if (!identifier(input.retain) ||
        input.retain === via ||
        !revision(input.expectedTopologyRevision))
        return bad();
    fields(input.target, ["endpoint", "candidate"], bad);
    if (Object.keys(input.target).length !== 1)
        return bad();
    const endpoint = Object.hasOwn(input.target, "endpoint"), target = endpoint ? input.target.endpoint : input.target.candidate;
    if (!identifier(target) ||
        (!endpoint && (target === via || target === input.retain)))
        return bad();
    return {
        body: {
            retained_connid: input.retain,
            ...(input.video === undefined
                ? {}
                : { video: replacementVideo(input.video, bad) }),
            ...(endpoint
                ? { endpoint_id: target }
                : { candidate_connid: target }),
        },
        context: {
            kind: "replacement",
            retained_connection_id: input.retain,
            target: { kind: endpoint ? "endpoint" : "candidate", id: target },
        },
        expected: input.expectedTopologyRevision,
    };
}
export function movementCompletion(value, key, bad) {
    if (!jsonObject(value) ||
        new TextEncoder().encode(canonicalJson(value)).length > 32768)
        return bad();
    fields(value, [
        "retained_connection_ids",
        "admitted_connection_ids",
        "retired_connection_ids",
        "session_revision",
        "conversation_revision",
        ...(key.command === "session.handoff"
            ? ["native_handoff"]
            : [
                "connection_id",
                "replaces",
                "generation",
                "participant_id",
                "membership_epoch",
                "endpoint_id",
                "device_id",
                "readiness",
                "activation_state",
                "physical_readiness",
                "native_activation",
            ]),
    ], bad);
    const lists = [
        value.retained_connection_ids,
        value.admitted_connection_ids,
        value.retired_connection_ids,
    ];
    if (lists.some((list) => !Array.isArray(list) || list.some((id) => !identifier(id))))
        return bad();
    const stopped = key.command === "connection.replace" &&
        value.activation_state === "stopped";
    const [retained, admitted, retired] = lists, all = [...retained, ...admitted, ...retired], context = key.continuation;
    if (!retained.length ||
        admitted.length !== 1 ||
        all.length > 100 ||
        new Set(retained).size !== retained.length ||
        new Set(retired).size !== retired.length ||
        retained.some((id) => admitted.includes(id) || retired.includes(id)) ||
        admitted.some((id) => retired.includes(id) &&
            !(stopped && id === value.connection_id)) ||
        !positive(value.session_revision) ||
        (value.conversation_revision !== undefined &&
            value.conversation_revision !== null &&
            !positive(value.conversation_revision)) ||
        !context ||
        !retained.includes(context.retained_connection_id))
        return bad();
    const topology = {
        retained: retained,
        admitted: admitted,
        retired: retired,
        topologyRevision: value.session_revision,
        ...(value.conversation_revision === undefined ||
            value.conversation_revision === null
            ? {}
            : {
                conversationRevision: value.conversation_revision,
            }),
    };
    if (key.command === "session.handoff" && context.kind === "handoff") {
        if ((context.candidate !== undefined &&
            context.candidate !== null &&
            admitted[0] !== context.candidate) ||
            admitted[0] === key.scope.connectionId ||
            (context.outgoing === "leave") !==
                retired.includes(key.scope.connectionId) ||
            (context.role.scope === "session"
                ? topology.conversationRevision !== undefined
                : topology.conversationRevision !==
                    context.role.expected_conversation_revision + 1))
            return bad();
        let roleChange;
        const role = value.native_handoff;
        if (role !== undefined && role !== null) {
            if (!jsonObject(role))
                return bad();
            fields(role, [
                "participant_revision",
                "role_scope",
                "outgoing",
                "source_participant",
                "target_participant",
                "cleanup",
            ], bad);
            if (!positive(role.participant_revision) ||
                role.source_participant !== context.source_participant ||
                role.target_participant !== context.target_participant ||
                role.role_scope !== context.role.scope ||
                role.outgoing !== context.outgoing ||
                role.cleanup !==
                    (context.outgoing === "leave" ? "pending" : "not_requested"))
                return bad();
            roleChange = {
                participantRevision: role.participant_revision,
                roleScope: context.role.scope,
                outgoing: context.outgoing,
                sourceParticipant: context.source_participant,
                targetParticipant: context.target_participant,
                cleanup: role.cleanup,
            };
        }
        return {
            ...topology,
            kind: "handoff",
            ...(roleChange === undefined ? {} : { roleChange }),
        };
    }
    const activationState = value.activation_state;
    const activationValid = activationState === "ready"
        ? jsonObject(value.readiness) &&
            jsonObject(value.physical_readiness) &&
            canonicalJson(value.readiness) ===
                canonicalJson(value.physical_readiness) &&
            !retired.includes(value.connection_id)
        : activationState === "stopped"
            ? value.readiness === null &&
                jsonObject(value.physical_readiness) &&
                retired.includes(value.connection_id)
            : !("activation_state" in value) &&
                !("physical_readiness" in value) &&
                jsonObject(value.readiness);
    if (!activationValid ||
        key.command !== "connection.replace" ||
        context.kind !== "replacement" ||
        !identifier(value.connection_id) ||
        value.replaces !== key.scope.connectionId ||
        !retired.includes(value.replaces) ||
        admitted[0] !== value.connection_id ||
        !positive(value.generation) ||
        !positive(value.membership_epoch) ||
        !identifier(value.participant_id) ||
        !identifier(value.endpoint_id) ||
        !identifier(value.device_id) ||
        topology.conversationRevision !== undefined ||
        (context.target.kind === "endpoint"
            ? value.endpoint_id !== context.target.id
            : value.connection_id !== context.target.id))
        return bad();
    movementActivation(value, key, bad);
    return {
        ...topology,
        kind: "replacement",
        activationState: (activationState ?? null),
        connectionId: value.connection_id,
        binding: {
            generation: value.generation,
            participantId: value.participant_id,
            membershipEpoch: value.membership_epoch,
            endpointId: value.endpoint_id,
            deviceId: value.device_id,
        },
    };
}
// Historical metadata only; negotiated physical readers still validate exact
// incarnation/handle bindings before assigning actual packet membership.
function movementActivation(value, key, bad) {
    if (!("activation_state" in value)) {
        if ("native_activation" in value)
            bad();
        return;
    }
    if (!("native_activation" in value))
        return bad();
    const v = value.native_activation;
    if (v === null && value.activation_state === "stopped")
        return;
    if (!jsonObject(v) || !jsonObject(value.physical_readiness))
        return bad();
    fields(v, [
        "profile",
        "operation_record_id",
        "operation_id",
        "source_connection_id",
        "target_connection_id",
        "target_generation",
        "stream_revision",
        "audience",
    ], bad);
    if (v.profile !== "connect/native-public-sequence-cut/1" ||
        !identifier(v.operation_record_id) ||
        v.operation_id !== key.operationId ||
        v.source_connection_id !== key.scope.connectionId ||
        v.source_connection_id !== value.replaces ||
        v.target_connection_id !== value.connection_id ||
        v.source_connection_id === v.target_connection_id ||
        v.target_generation !== value.generation ||
        !positive(v.stream_revision) ||
        v.stream_revision <= 2 ||
        v.stream_revision !== value.physical_readiness.stream_revision ||
        !jsonObject(v.audience))
        return bad();
    const audience = v.audience;
    if (audience.scope === "private-consultation") {
        fields(audience, ["scope", "original_operation"], bad);
        if (!identifier(audience.original_operation))
            bad();
    }
    else if (audience.scope === "unsupported") {
        fields(audience, ["scope", "reason"], bad);
        if (audience.reason !== "non-native-adapter")
            bad();
    }
    else {
        fields(audience, ["scope", "streams"], bad);
        if (audience.scope !== "public" ||
            !Array.isArray(audience.streams) ||
            audience.streams.length < 1 ||
            audience.streams.length > 32 ||
            !Array.isArray(value.physical_readiness.streams))
            return bad();
        const ids = new Set(), handles = new Set(), physical = new Set();
        for (const valueCut of audience.streams) {
            let cut;
            try {
                cut = nativeCut(valueCut);
            }
            catch {
                return bad();
            }
            if (!value.physical_readiness.streams.includes(cut.stream_id) ||
                ids.has(cut.stream_id) ||
                handles.has(cut.stream_local_id) ||
                physical.has(cut.physical_incarnation))
                return bad();
            ids.add(cut.stream_id);
            handles.add(cut.stream_local_id);
            physical.add(cut.physical_incarnation);
        }
    }
}
