/** Current SDK errors retain logical intent without copying response bodies. */
export type CanonicalErrorKind =
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
export type Certainty =
    "not-submitted" | "refused" | "uncertain" | "recorded" | "read-only";
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
export class CanonicalError extends Error {
    constructor(
        readonly kind: CanonicalErrorKind,
        readonly certainty: Certainty,
        readonly operation?: OperationKey,
        readonly problem?: CanonicalProblem,
    ) {
        super(`Connect SDK ${kind} (${certainty})`);
        this.name = "CanonicalError";
    }
}
