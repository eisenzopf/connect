// G6-059 declaration-only target. This is not an implemented or runnable SDK.
declare const identity: unique symbol;
declare const resultType: unique symbol;
export type Id<K extends string> = string & { readonly [identity]: K };
export type ConversationId = Id<"conversation">;
export type ParticipantId = Id<"participant">;
export type SessionId = Id<"session">;
export type ConnectionId = Id<"connection">;
export type StreamId = Id<"stream">;
export type OperationId = Id<"operation">;
export type MessageId = Id<"message">;
export type EndpointId = Id<"endpoint">;
export type IdentityId = Id<"identity">;
export type Cursor = Id<"cursor">;
export type Revision<K extends string> = number & { readonly [identity]: K };
export type Json =
    | null
    | boolean
    | number
    | string
    | readonly Json[]
    | { readonly [key: string]: Json };
export interface NonEmpty<T> {
    readonly first: T;
    readonly rest: readonly T[];
}
export interface Scope {
    readonly conversationId?: ConversationId;
    readonly sessionId?: SessionId;
    readonly connectionId?: ConnectionId;
}
export interface OperationKey {
    readonly operationId: OperationId;
    readonly command: string;
    readonly scope: Scope;
}
export interface OperationReference<T> extends OperationKey {
    readonly [resultType]: (result: T) => T;
}
export interface RequestOptions {
    readonly timeoutMs: number;
    readonly signal?: AbortSignal;
}
export interface MutationOptions extends RequestOptions {
    readonly operationId: OperationId;
    readonly extensions?: Readonly<Record<string, Json>>;
}
export interface WaitOptions extends RequestOptions {}
export type OperationState =
    | "accepted"
    | "preparing"
    | "ready"
    | "committing"
    | "completed"
    | "cancelled"
    | "failed"
    | "interrupted"
    | "unknown";
export interface CanonicalProblem {
    readonly code: number;
    readonly category: string;
    readonly reason: string;
}
export type ErrorKind =
    | "validation"
    | "configuration"
    | "trust"
    | "authentication"
    | "protocol"
    | "deadline"
    | "cancelled"
    | "transport-uncertain"
    | "unsupported"
    | "conflict"
    | "application"
    | "receipt-unavailable"
    | "resync-required"
    | "cleanup-incomplete";
export declare class ConnectError extends Error {
    readonly kind: ErrorKind;
    readonly certainty:
        "not-submitted" | "refused" | "uncertain" | "recorded" | "read-only";
    readonly operation?: OperationKey;
    readonly problem?: CanonicalProblem;
}
export interface OperationSnapshot {
    readonly key: OperationKey;
    readonly state: OperationState;
    readonly revision: number;
    readonly cancelRequested?: boolean;
    readonly expiresAt?: number;
    readonly problem?: Json;
}
export interface Completion<T> {
    readonly snapshot: OperationSnapshot;
    readonly value: T;
}
export interface DecisionReceipt {
    readonly decision: OperationKey;
    readonly target: OperationKey;
    readonly observedState: OperationState;
}
export interface Operation<T> {
    readonly reference: OperationReference<T>;
    status(options: RequestOptions): Promise<OperationSnapshot>;
    wait(options: WaitOptions): Promise<Completion<T>>;
    waitUntilReady(options: WaitOptions): Promise<OperationSnapshot>;
    cancel(options: MutationOptions): Promise<Operation<DecisionReceipt>>;
    reconcile(
        input: {
            outcome: "completed" | "cancelled" | "failed";
            evidenceId: string;
        },
        options: MutationOptions,
    ): Promise<Operation<DecisionReceipt>>;
}
export interface SessionSnapshot {
    readonly id: SessionId;
    readonly conversationId: ConversationId;
    readonly state: string;
    readonly media: readonly Medium[];
    readonly intent: string | null;
    readonly capabilitiesOffer: Json;
    readonly codecPreferences: readonly string[];
    readonly isMain: boolean;
    readonly revision: Revision<"session">;
    readonly topologyRevision: Revision<"topology">;
    readonly createdAt: number;
    readonly updatedAt: number;
    readonly endedReason: string | null;
}
export type IdentityKind = "human" | "ai" | "system" | "external";
export type Medium = "audio" | "video" | "screen" | "data";
export interface ParticipantSnapshot {
    readonly id: ParticipantId;
    readonly conversationId: ConversationId;
    readonly identityId: IdentityId;
    readonly membershipEpoch: number;
    readonly active: boolean;
    readonly role: string;
}
export interface ConversationSnapshot {
    readonly id: ConversationId;
    readonly revision: Revision<"conversation">;
    readonly participants: readonly ParticipantSnapshot[];
    readonly closed: boolean;
}
export interface ConnectionSnapshot {
    readonly id: ConnectionId;
    readonly conversationId: ConversationId;
    readonly sessionId: SessionId;
    readonly generation: number;
    readonly streamRevision: Revision<"stream">;
    readonly state: string;
}
export interface StreamSelection {
    readonly streams: readonly Stream[];
    readonly streamRevision: Revision<"stream">;
}
export interface MessageAcceptance {
    readonly messageId: MessageId;
    readonly operation: OperationKey;
    readonly boundary: "local-mutation";
}
export interface DeliveryObservation {
    readonly participantId: ParticipantId;
    readonly membershipEpoch: number;
    readonly channel: string;
    readonly deliveryId: string;
    readonly state:
        "queued" | "accepted" | "delivered" | "read" | "failed" | "unknown";
    readonly provenance: string;
    readonly observedAt: number;
}
export interface ConsultationResult {
    readonly session: Session;
    readonly retained: Connection;
}
export interface HandoffResult {
    readonly retained: Connection;
    readonly admitted: readonly Connection[];
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: Revision<"topology">;
}
export interface MoveResult {
    readonly connection: Connection;
    readonly retained: Connection;
    readonly retired: readonly ConnectionId[];
    readonly topologyRevision: Revision<"topology">;
}
export interface SessionDecision {
    readonly session: Session;
    readonly state: string;
    readonly revision: Revision<"session">;
}
export interface EventSource {
    readonly kind: string;
    readonly correlationId: string;
}
export interface JournalEvent {
    readonly eventId: string;
    readonly cursor: Cursor;
    readonly conversationId: ConversationId;
    readonly sessionId?: SessionId;
    readonly connectionId?: ConnectionId;
    readonly operationId?: OperationId;
    readonly kind: string;
    readonly revision: number;
    readonly source: EventSource;
    readonly observedAt: number;
    readonly data: Json;
}
export interface EventStream extends AsyncIterable<JournalEvent> {
    close(): Promise<void>;
}
export interface InvitationStream extends AsyncIterable<IncomingInvitation> {
    close(): Promise<void>;
}
export interface IncomingInvitation {
    readonly session: Session;
    readonly from: ParticipantId;
    readonly operationId: OperationId;
    readonly source: EventSource;
}
export interface CapabilitySnapshot {
    readonly profile: "connect/2";
    readonly runtimeRole:
        "rust" | "browser" | "node" | "python" | "http-control";
    readonly selectedTransport: "quic" | "webtransport" | "wss-webrtc" | "http";
    readonly fallbackReason?: string;
    readonly features: ReadonlySet<string>;
}
export interface Capabilities {
    refresh(options: RequestOptions): Promise<CapabilitySnapshot>;
    require(feature: string, options: RequestOptions): Promise<void>;
}
export interface ValidatedEnvelope {
    readonly value: Json;
    readonly [identity]: "validated-current-canonical-envelope";
}
export interface RawCanonical {
    validate(value: Json): ValidatedEnvelope;
    request(
        envelope: ValidatedEnvelope,
        options: RequestOptions,
    ): Promise<readonly ValidatedEnvelope[]>;
}
export interface Participant {
    readonly id: ParticipantId;
    readonly conversationId: ConversationId;
    refresh(options: RequestOptions): Promise<ParticipantSnapshot>;
    update(
        input: {
            role: string;
            expectedConversationRevision: Revision<"conversation">;
        },
        options: MutationOptions,
    ): Promise<Operation<Participant>>;
    remove(
        input: { expectedConversationRevision: Revision<"conversation"> },
        options: MutationOptions,
    ): Promise<Operation<Participant>>;
}
export interface ConsultInput {
    readonly via: ConnectionId;
    readonly retain: ConnectionId;
    readonly with: NonEmpty<ParticipantId>;
    readonly hold: "silence" | "configured-media";
    readonly expectedTopologyRevision: Revision<"topology">;
}
export type HandoffRole =
    | { readonly scope: "session" }
    | {
          readonly scope: "conversation";
          readonly expectedConversationRevision: Revision<"conversation">;
      };
export interface HandoffInput {
    readonly via: ConnectionId;
    readonly from: ParticipantId;
    readonly to: ParticipantId;
    readonly retain: ConnectionId;
    readonly candidate?: ConnectionId;
    readonly outgoing: "leave" | "observe";
    readonly role: HandoffRole;
    readonly expectedTopologyRevision: Revision<"topology">;
    readonly consultation?: ConsultationOperation;
}
export type MoveTarget =
    | { readonly endpoint: EndpointId; readonly candidate?: never }
    | { readonly candidate: ConnectionId; readonly endpoint?: never };
export interface MoveInput {
    readonly retain: ConnectionId;
    readonly target: MoveTarget;
    readonly expectedTopologyRevision: Revision<"topology">;
}
export interface ConsultationOperation extends Operation<ConsultationResult> {
    returnToRetained(
        input: { expectedTopologyRevision: Revision<"topology"> },
        options: MutationOptions,
    ): Promise<Operation<Session>>;
}
export type SessionUpdate =
    | {
          readonly action: "participant-leave";
          readonly participantId: ParticipantId;
      }
    | {
          readonly action: "role-change";
          readonly participantId: ParticipantId;
          readonly role: string;
      }
    | { readonly action: "medium-upgrade"; readonly media: NonEmpty<Medium> }
    | {
          readonly action: "codec-renegotiate";
          readonly codecPreferences: NonEmpty<string>;
      };
export type ConnectionUpdate =
    | {
          readonly action: "mute" | "unmute";
          readonly streams: NonEmpty<StreamId>;
      }
    | {
          readonly action: "hold";
          readonly streams?: NonEmpty<StreamId>;
          readonly direction: "sendonly" | "recvonly" | "inactive";
      }
    | { readonly action: "resume"; readonly streams?: NonEmpty<StreamId> }
    | { readonly action: "quality"; readonly intervalMs: number }
    | {
          readonly action: "codec-renegotiate";
          readonly codecPreferences: NonEmpty<string>;
      };
export interface Session {
    readonly id: SessionId;
    readonly conversationId: ConversationId;
    refresh(options: RequestOptions): Promise<SessionSnapshot>;
    accept(
        input: { capabilities: Json; expectedRevision: Revision<"session"> },
        options: MutationOptions,
    ): Promise<Operation<Session>>;
    reject(
        input: {
            reasonCode: number;
            reason: string;
            expectedRevision: Revision<"session">;
        },
        options: MutationOptions,
    ): Promise<Operation<SessionDecision>>;
    cancelInvitation(
        input: {
            reasonCode: number;
            reason: string;
            expectedRevision: Revision<"session">;
        },
        options: MutationOptions,
    ): Promise<Operation<SessionDecision>>;
    consult(
        input: ConsultInput,
        options: MutationOptions,
    ): Promise<ConsultationOperation>;
    handoff(
        input: HandoffInput,
        options: MutationOptions,
    ): Promise<Operation<HandoffResult>>;
    update(
        input: SessionUpdate,
        expectedRevision: Revision<"session">,
        options: MutationOptions,
    ): Promise<Operation<Session>>;
    end(
        input: {
            reasonCode: number;
            reason: string;
            expectedRevision: Revision<"session">;
        },
        options: MutationOptions,
    ): Promise<Operation<Session>>;
    readonly connections: {
        byId(id: ConnectionId): Connection;
        offer(
            input: {
                connectionId: ConnectionId;
                streams: NonEmpty<{
                    id: StreamId;
                    kind: Medium;
                    direction: "sendonly" | "recvonly" | "sendrecv";
                    codecs: NonEmpty<string>;
                }>;
            },
            options: MutationOptions,
        ): Promise<Operation<Connection>>;
    };
}
export interface Connection {
    readonly id: ConnectionId;
    readonly sessionId: SessionId;
    readonly conversationId: ConversationId;
    refresh(options: RequestOptions): Promise<ConnectionSnapshot>;
    update(
        input: ConnectionUpdate,
        expectedStreamRevision: Revision<"stream">,
        options: MutationOptions,
    ): Promise<Operation<Connection>>;
    replaceDevice(
        input: MoveInput,
        options: MutationOptions,
    ): Promise<Operation<MoveResult>>;
    end(options: MutationOptions): Promise<Operation<Connection>>;
    readonly streams: {
        byId(id: StreamId): Stream;
        subscribe(
            input: {
                from: ParticipantId;
                kinds: NonEmpty<Medium>;
                expectedStreamRevision: Revision<"stream">;
            },
            options: MutationOptions,
        ): Promise<Operation<StreamSelection>>;
        unsubscribe(
            input: {
                streams: NonEmpty<StreamId>;
                expectedStreamRevision: Revision<"stream">;
            },
            options: MutationOptions,
        ): Promise<Operation<StreamSelection>>;
    };
}
export interface Stream {
    readonly id: StreamId;
    readonly connectionId: ConnectionId;
    readonly sessionId: SessionId;
    readonly conversationId: ConversationId;
}
export interface Conversation {
    readonly id: ConversationId;
    refresh(options: RequestOptions): Promise<ConversationSnapshot>;
    send(
        input: {
            messageId: MessageId;
            to: NonEmpty<ParticipantId>;
            text: string;
        },
        options: MutationOptions,
    ): Promise<Operation<MessageAcceptance>>;
    receipts(
        messageId: MessageId,
        options: RequestOptions,
    ): Promise<readonly DeliveryObservation[]>;
    events(
        input: { after?: Cursor | undefined },
        options: RequestOptions,
    ): EventStream;
    readonly participants: {
        byId(id: ParticipantId): Participant;
        add(
            input: {
                identityId: IdentityId;
                kind: IdentityKind;
                role: string;
                expectedRevision: Revision<"conversation">;
            },
            options: MutationOptions,
        ): Promise<Operation<Participant>>;
    };
    readonly sessions: {
        byId(id: SessionId): Session;
        invite(
            input: {
                sessionId: SessionId;
                to: NonEmpty<ParticipantId>;
                medium:
                    | "voice"
                    | "video"
                    | "voice+video"
                    | "screen-share"
                    | "text-chat"
                    | "mixed";
                intent: string;
                capabilities: Json;
            },
            options: MutationOptions,
        ): Promise<Operation<Session>>;
    };
}
export interface ParticipantClient {
    readonly conversations: {
        byId(id: ConversationId): Conversation;
        create(
            input: {
                participants: readonly IdentityId[];
                policy: "persistent" | "ephemeral";
            },
            options: MutationOptions,
        ): Promise<Operation<Conversation>>;
    };
    readonly operations: {
        recover<T>(reference: OperationReference<T>): Operation<T>;
    };
    readonly capabilities: Capabilities;
    readonly raw: RawCanonical;
    incomingInvitations(options: RequestOptions): InvitationStream;
    close(options: RequestOptions): Promise<void>;
}
export interface BrowserClient extends ParticipantClient {
    readonly role: "browser";
}
export interface NodeClient extends ParticipantClient {
    readonly role: "node";
}
// The full trusted provisioning enum is connect-protocol::ProvisioningCommand.
// Its validated command path is deliberately absent from ParticipantClient.
export interface TrustedProvisioningCommand {
    readonly value: Json;
    readonly [identity]: "validated-trusted-provisioning";
}
export interface AdministrativeClient {
    validateProvisioning(value: Json): TrustedProvisioningCommand;
    provision(
        command: TrustedProvisioningCommand,
        options: MutationOptions,
    ): Promise<Operation<Json>>;
    close(options: RequestOptions): Promise<void>;
}
