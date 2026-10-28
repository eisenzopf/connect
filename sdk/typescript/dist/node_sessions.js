import { sdkRevision } from "./node_sdk.js";
import { NodeSdkError, object } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
function invalid() {
    throw new NodeSdkError("validation", "not-submitted");
}
function nonempty(input, check) {
    if (!input ||
        !Array.isArray(input.rest) ||
        !check(input.first) ||
        input.rest.some((v) => !check(v)))
        invalid();
    return [input.first, ...input.rest];
}
const medium = (v) => typeof v === "string" && ["audio", "video", "screen", "data"].includes(v);
function invite(input) {
    if (!input ||
        !Array.isArray(input.to) ||
        input.to.some((v) => !identifier(v)) ||
        ![
            "voice",
            "video",
            "voice+video",
            "screen-share",
            "text-chat",
            "mixed",
        ].includes(input.medium) ||
        typeof input.intent !== "string" ||
        input.capabilities === undefined)
        invalid();
    return {
        type: "invite",
        to: [...input.to],
        medium: input.medium,
        intent: input.intent,
        capabilities: input.capabilities,
    };
}
export function createSessions(context) {
    const owned = Object.freeze({ ...context });
    return Object.freeze({
        byId: (id) => {
            if (!identifier(id))
                invalid();
            return new SdkSession(owned, id);
        },
        invite: (input, options) => {
            if (!input || !identifier(input.sessionId))
                invalid();
            return owned.mutate(input.sessionId, invite(input), owned.kinds.session_invite, options);
        },
    });
}
/** Stable IDs and typed control; creation of a handle never acquires media. */
export class SdkSession {
    context;
    id;
    conversationId;
    constructor(context, id) {
        this.context = context;
        this.id = id;
        this.conversationId = context.cid;
        Object.freeze(this);
    }
    refresh(options) {
        return this.context.refresh(this.id, options);
    }
    inviteParticipants(input, options) {
        if (!input)
            invalid();
        const action = invite({ ...input, to: nonempty(input.to, identifier) });
        action.expected_revision = sdkRevision(input.expectedTopologyRevision);
        return this.context.mutate(this.id, action, this.context.kinds.session_invite, options);
    }
    accept(input, options) {
        if (!input || input.capabilities === undefined)
            invalid();
        return this.context.mutate(this.id, {
            type: "accept",
            capabilities: input.capabilities,
            expected_revision: sdkRevision(input.expectedTopologyRevision),
        }, this.context.kinds.session_accept, options);
    }
    reject(input, options) {
        return this.decide("reject", input, this.context.kinds.session_reject, options);
    }
    cancelInvitation(input, options) {
        return this.decide("cancel", input, this.context.kinds.session_cancel, options);
    }
    end(input, options) {
        return this.decide("end", input, this.context.kinds.session_end, options);
    }
    decide(type, input, kind, options) {
        if (!input ||
            !Number.isInteger(input.reasonCode) ||
            input.reasonCode < 0 ||
            input.reasonCode > 65535 ||
            typeof input.reason !== "string")
            invalid();
        return this.context.mutate(this.id, {
            type,
            reason_code: input.reasonCode,
            reason: input.reason,
            expected_revision: sdkRevision(input.expectedTopologyRevision),
        }, kind, options);
    }
    update(input, expectedTopologyRevision, options) {
        if (!input)
            invalid();
        let update;
        switch (input.action) {
            case "participant-leave":
                if (!identifier(input.participantId))
                    invalid();
                update = {
                    action: input.action,
                    participant_id: input.participantId,
                };
                break;
            case "role-change":
                if (!identifier(input.participantId) ||
                    typeof input.role !== "string")
                    invalid();
                update = {
                    action: input.action,
                    participant_id: input.participantId,
                    role: input.role,
                };
                break;
            case "medium-upgrade":
                update = {
                    action: input.action,
                    media: nonempty(input.media, medium),
                };
                break;
            case "codec-renegotiate":
                update = {
                    action: input.action,
                    codec_preferences: nonempty(input.codecPreferences, (v) => typeof v === "string" && !!v),
                };
                break;
            default:
                return invalid();
        }
        return this.context.mutate(this.id, {
            type: "update",
            update,
            expected_revision: sdkRevision(expectedTopologyRevision),
        }, this.context.kinds.session_update, options);
    }
}
export function sessionSnapshot(value, cid, sid) {
    if (!object(value, [
        "id",
        "conversationId",
        "state",
        "media",
        "intent",
        "capabilitiesOffer",
        "codecPreferences",
        "isMain",
        "revision",
        "topologyRevision",
        "createdAt",
        "updatedAt",
        "endedReason",
    ]) ||
        value.id !== sid ||
        value.conversationId !== cid ||
        typeof value.state !== "string" ||
        !Array.isArray(value.media) ||
        value.media.some((v) => !medium(v)) ||
        (value.intent !== null && typeof value.intent !== "string") ||
        value.capabilitiesOffer === undefined ||
        !Array.isArray(value.codecPreferences) ||
        value.codecPreferences.some((v) => typeof v !== "string") ||
        typeof value.isMain !== "boolean" ||
        ![
            value.revision,
            value.topologyRevision,
            value.createdAt,
            value.updatedAt,
        ].every((v) => Number.isSafeInteger(v) && Number(v) >= 0) ||
        (value.endedReason !== null && typeof value.endedReason !== "string"))
        throw new Error("Invalid SDK Session snapshot");
    const freeze = (v) => {
        if (v && typeof v === "object") {
            for (const item of Object.values(v))
                freeze(item);
            Object.freeze(v);
        }
        return v;
    };
    return freeze(value);
}
