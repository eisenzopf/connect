import { ConnectError } from "./errors.js";
import type { Operation, Outcome, OutcomeData } from "./types.js";
/** Shared public operations. Mutation IDs belong to the caller. */
export abstract class Operations {
    abstract request(operation: Operation): Promise<Outcome>;
    private async data<K extends keyof OutcomeData>(
        kind: K,
        operation: Operation,
    ): Promise<OutcomeData[K]> {
        const result = await this.request(operation);
        if (result.kind !== kind)
            throw new ConnectError("protocol", "Unexpected outcome");
        return result.data as OutcomeData[K];
    }
    resolve(address: string) {
        return this.data("identity", { op: "resolve", address });
    }
    create(request_id: string, participants: string[]) {
        return this.data("conversation", {
            op: "create",
            request_id,
            participants,
        });
    }
    send(request_id: string, conversation_id: string, body: string) {
        return this.data("accepted", {
            op: "send",
            request_id,
            conversation_id,
            body,
        });
    }
    history(conversation_id: string, after: string | null = null, limit = 100) {
        return this.data("history", {
            op: "history",
            conversation_id,
            after,
            limit,
        });
    }
    inbox(cursor: string | null = null, limit = 100, wait_ms = 0) {
        return this.data("inbox", { op: "inbox", cursor, limit, wait_ms });
    }
    requestStatus(request_id: string) {
        return this.data("request_status", {
            op: "request_status",
            request_id,
        });
    }
    providerStatus(conversation_id: string) {
        return this.data("provider_status", {
            op: "provider_status",
            conversation_id,
        });
    }
    closeConversation(request_id: string, conversation_id: string) {
        return this.data("conversation", {
            op: "close",
            request_id,
            conversation_id,
        });
    }
    callStart(
        request_id: string,
        conversation_id: string,
        route_id: string,
        destination: string,
    ) {
        return this.data("call", {
            op: "call_start",
            request_id,
            conversation_id,
            route_id,
            destination,
        });
    }
    callStatus(conversation_id: string, call_id: string) {
        return this.data("call", {
            op: "call_status",
            conversation_id,
            call_id,
        });
    }
    callEnd(request_id: string, conversation_id: string, call_id: string) {
        return this.data("call", {
            op: "call_end",
            request_id,
            conversation_id,
            call_id,
        });
    }
    health() {
        return this.data("health", { op: "health" });
    }
}
