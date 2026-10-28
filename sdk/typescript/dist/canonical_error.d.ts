/** Current SDK errors retain logical intent without copying response bodies. */
export type CanonicalErrorKind = "validation" | "configuration" | "trust" | "authentication" | "protocol" | "deadline" | "cancelled" | "transport-uncertain" | "unsupported" | "conflict" | "application" | "receipt-unavailable" | "resync-required" | "cleanup-incomplete";
export type Certainty = "not-submitted" | "refused" | "uncertain" | "recorded" | "read-only";
export interface OperationKey {
    readonly operationId: string;
    readonly command: string;
    readonly scope: {
        readonly conversationId?: string;
        readonly sessionId?: string;
        readonly connectionId?: string;
    };
}
export interface CanonicalProblem {
    readonly code: number;
    readonly category: string;
    readonly reason: string;
}
export declare class CanonicalError extends Error {
    readonly kind: CanonicalErrorKind;
    readonly certainty: Certainty;
    readonly operation?: OperationKey | undefined;
    readonly problem?: CanonicalProblem | undefined;
    constructor(kind: CanonicalErrorKind, certainty: Certainty, operation?: OperationKey | undefined, problem?: CanonicalProblem | undefined);
}
