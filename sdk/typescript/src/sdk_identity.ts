/** Shared nominal ID/revision brands. This leaf imports no platform runtime. */
declare const identity: unique symbol;
declare const revisionFamily: unique symbol;
export type SdkId<K extends string> = string & { readonly [identity]: K };
export type SdkRevision<K extends string> = number & {
    readonly [revisionFamily]: K;
};
export type ConversationId = SdkId<"conversation">;
export type IdentityId = SdkId<"identity">;
export type ParticipantId = SdkId<"participant">;
export type MessageId = SdkId<"message">;
export type OperationId = SdkId<"operation">;
export type SessionId = SdkId<"session">;
