import type { SdkConnections } from "./node_connections.js";
import { sdkRevision } from "./node_sdk.js";
import type {
    ConversationId,
    ParticipantId,
    SdkId,
    SdkRevision,
    SdkMutationOptions,
    SdkOperation,
    SdkOperationKind,
} from "./node_sdk.js";
import { NodeSdkError, object } from "./sdk_helper.js";
import { identifier } from "./uctp_wire.js";
import type { Json, JsonObject } from "./uctp_json.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";

export type SessionId = SdkId<"session">;
export type SdkMedium = "audio" | "video" | "screen" | "data";
export type SdkSessionMedium =
    "voice" | "video" | "voice+video" | "screen-share" | "text-chat" | "mixed";
export interface SdkNonEmpty<T> {
    readonly first: T;
    readonly rest: readonly T[];
}
export interface SdkSessionInvite {
    readonly sessionId: SessionId;
    readonly to: readonly ParticipantId[];
    readonly medium: SdkSessionMedium;
    readonly intent: string;
    readonly capabilities: Json;
}
export type SdkSessionUpdate =
    | {
          readonly action: "participant-leave";
          readonly participantId: ParticipantId;
      }
    | {
          readonly action: "role-change";
          readonly participantId: ParticipantId;
          readonly role: string;
      }
    | {
          readonly action: "medium-upgrade";
          readonly media: SdkNonEmpty<SdkMedium>;
      }
    | {
          readonly action: "codec-renegotiate";
          readonly codecPreferences: SdkNonEmpty<string>;
      };
export interface SdkSessionDecision {
    readonly session: SdkSession;
    readonly state: string;
    readonly topologyRevision: SdkRevision<"topology">;
}
export interface SdkSessionSnapshot {
    readonly id: SessionId;
    readonly conversationId: ConversationId;
    readonly state: string;
    readonly media: readonly SdkMedium[];
    readonly intent: string | null;
    readonly capabilitiesOffer: Json;
    readonly codecPreferences: readonly string[];
    readonly isMain: boolean;
    readonly revision: SdkRevision<"session">;
    readonly topologyRevision: SdkRevision<"topology">;
    readonly createdAt: number;
    readonly updatedAt: number;
    readonly endedReason: string | null;
}
export interface SdkSessions {
    byId(id: SessionId): SdkSession;
    invite(
        input: SdkSessionInvite,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSession>>;
}
interface Kinds {
    readonly session_invite: SdkOperationKind<SdkSession>;
    readonly session_accept: SdkOperationKind<SdkSession>;
    readonly session_update: SdkOperationKind<SdkSession>;
    readonly session_end: SdkOperationKind<SdkSession>;
    readonly session_reject: SdkOperationKind<SdkSessionDecision>;
    readonly session_cancel: SdkOperationKind<SdkSessionDecision>;
}
interface Context {
    connections(id: SessionId): SdkConnections;
    readonly cid: ConversationId;
    readonly kinds: Kinds;
    mutate<T>(
        sid: SessionId,
        action: JsonObject,
        kind: SdkOperationKind<T>,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<T>>;
    refresh(
        sid: SessionId,
        options: CanonicalRequestOptions,
    ): Promise<SdkSessionSnapshot>;
}
function invalid(): never {
    throw new NodeSdkError("validation", "not-submitted");
}
function nonempty<T>(
    input: SdkNonEmpty<T>,
    check: (v: unknown) => boolean,
): T[] {
    if (
        !input ||
        !Array.isArray(input.rest) ||
        !check(input.first) ||
        input.rest.some((v) => !check(v))
    )
        invalid();
    return [input.first, ...input.rest];
}
const medium = (v: unknown): boolean =>
    typeof v === "string" && ["audio", "video", "screen", "data"].includes(v);
function invite(input: Omit<SdkSessionInvite, "sessionId">): JsonObject {
    if (
        !input ||
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
        input.capabilities === undefined
    )
        invalid();
    return {
        type: "invite",
        to: [...input.to],
        medium: input.medium,
        intent: input.intent,
        capabilities: input.capabilities,
    };
}
export function createSessions(context: Context): SdkSessions {
    const owned = Object.freeze({ ...context });
    return Object.freeze({
        byId: (id: SessionId) => {
            if (!identifier(id)) invalid();
            return new SdkSession(owned, id);
        },
        invite: (input: SdkSessionInvite, options: SdkMutationOptions) => {
            if (!input || !identifier(input.sessionId)) invalid();
            return owned.mutate(
                input.sessionId,
                invite(input),
                owned.kinds.session_invite,
                options,
            );
        },
    });
}
/** Stable IDs and typed control; creation of a handle never acquires media. */
export class SdkSession {
    readonly connections: SdkConnections;
    readonly conversationId: ConversationId;
    constructor(
        private readonly context: Context,
        readonly id: SessionId,
    ) {
        this.conversationId = context.cid;
        this.connections = context.connections(id);
        Object.freeze(this);
    }
    refresh(options: CanonicalRequestOptions): Promise<SdkSessionSnapshot> {
        return this.context.refresh(this.id, options);
    }
    inviteParticipants(
        input: {
            readonly to: SdkNonEmpty<ParticipantId>;
            readonly medium: SdkSessionMedium;
            readonly intent: string;
            readonly capabilities: Json;
            readonly expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSession>> {
        if (!input) invalid();
        const action = invite({ ...input, to: nonempty(input.to, identifier) });
        action.expected_revision = sdkRevision<"topology">(
            input.expectedTopologyRevision,
        );
        return this.context.mutate(
            this.id,
            action,
            this.context.kinds.session_invite,
            options,
        );
    }
    accept(
        input: {
            readonly capabilities: Json;
            readonly expectedTopologyRevision: SdkRevision<"topology">;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSession>> {
        if (!input || input.capabilities === undefined) invalid();
        return this.context.mutate(
            this.id,
            {
                type: "accept",
                capabilities: input.capabilities,
                expected_revision: sdkRevision<"topology">(
                    input.expectedTopologyRevision,
                ),
            },
            this.context.kinds.session_accept,
            options,
        );
    }
    reject(
        input: DecisionInput,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSessionDecision>> {
        return this.decide(
            "reject",
            input,
            this.context.kinds.session_reject,
            options,
        );
    }
    cancelInvitation(
        input: DecisionInput,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSessionDecision>> {
        return this.decide(
            "cancel",
            input,
            this.context.kinds.session_cancel,
            options,
        );
    }
    end(
        input: DecisionInput,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSession>> {
        return this.decide(
            "end",
            input,
            this.context.kinds.session_end,
            options,
        );
    }
    private decide<T>(
        type: string,
        input: DecisionInput,
        kind: SdkOperationKind<T>,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<T>> {
        if (
            !input ||
            !Number.isInteger(input.reasonCode) ||
            input.reasonCode < 0 ||
            input.reasonCode > 65535 ||
            typeof input.reason !== "string"
        )
            invalid();
        return this.context.mutate(
            this.id,
            {
                type,
                reason_code: input.reasonCode,
                reason: input.reason,
                expected_revision: sdkRevision<"topology">(
                    input.expectedTopologyRevision,
                ),
            },
            kind,
            options,
        );
    }
    update(
        input: SdkSessionUpdate,
        expectedTopologyRevision: SdkRevision<"topology">,
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkSession>> {
        if (!input) invalid();
        let update: JsonObject;
        switch (input.action) {
            case "participant-leave":
                if (!identifier(input.participantId)) invalid();
                update = {
                    action: input.action,
                    participant_id: input.participantId,
                };
                break;
            case "role-change":
                if (
                    !identifier(input.participantId) ||
                    typeof input.role !== "string"
                )
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
                    codec_preferences: nonempty(
                        input.codecPreferences,
                        (v) => typeof v === "string" && !!v,
                    ),
                };
                break;
            default:
                return invalid();
        }
        return this.context.mutate(
            this.id,
            {
                type: "update",
                update,
                expected_revision: sdkRevision<"topology">(
                    expectedTopologyRevision,
                ),
            },
            this.context.kinds.session_update,
            options,
        );
    }
}
interface DecisionInput {
    readonly reasonCode: number;
    readonly reason: string;
    readonly expectedTopologyRevision: SdkRevision<"topology">;
}
export function sessionSnapshot(
    value: Json,
    cid: ConversationId,
    sid: SessionId,
): SdkSessionSnapshot {
    if (
        !object(value, [
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
        (value.endedReason !== null && typeof value.endedReason !== "string")
    )
        throw new Error("Invalid SDK Session snapshot");
    const freeze = (v: Json): Json => {
        if (v && typeof v === "object") {
            for (const item of Object.values(v)) freeze(item);
            Object.freeze(v);
        }
        return v;
    };
    return freeze(value) as unknown as SdkSessionSnapshot;
}
