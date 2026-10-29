import { type Json } from "./uctp_json.js";
import type { BrowserOperationReference } from "./browser_transport_client.js";
import type { BrowserOperationState } from "./browser_sdk.js";
import type { OperationId, ConversationId, SessionId } from "./sdk_identity.js";
export interface BrowserDecisionReceipt {
    readonly decision: {
        readonly operationId: OperationId;
        readonly command: "operation.cancel" | "operation.reconcile";
        readonly scope: {
            readonly conversationId?: ConversationId;
            readonly sessionId?: SessionId;
        };
    };
    readonly target: OperationId;
    readonly observedState: BrowserOperationState;
    readonly disposition: string;
    readonly evidenceId?: string;
}
export declare function operationDecisionResult(value: Json, key: BrowserOperationReference, bad: () => never): BrowserDecisionReceipt;
