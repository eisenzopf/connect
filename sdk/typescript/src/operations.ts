import { TaskOperations } from "./task_operations.js";
import { ConnectError } from "./errors.js";
import type { Operation, Outcome, OutcomeData } from "./types.js";
/** Shared public operations. Mutation IDs belong to the caller. */
export abstract class Operations extends TaskOperations {
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
    capabilities(address: string, conversation_id: string | null = null) {
        return this.data("capabilities", {
            op: "capabilities",
            address,
            conversation_id,
        });
    }
    browserBinding() {
        return this.data("browser_binding", { op: "browser_binding" });
    }
    contextDeliveries(
        conversation_id: string,
        before: string | null = null,
        limit = 100,
    ) {
        return this.data("context_deliveries", {
            op: "context_deliveries",
            conversation_id,
            before,
            limit,
        });
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
    sessionInvite(
        request_id: string,
        conversation_id: string,
        participant_id: string,
        media: string[],
    ) {
        return this.data("session", {
            op: "session_invite",
            request_id,
            conversation_id,
            participant_id,
            media,
        });
    }
    sessionAccept(
        request_id: string,
        conversation_id: string,
        session_id: string,
    ) {
        return this.data("session", {
            op: "session_accept",
            request_id,
            conversation_id,
            session_id,
        });
    }
    sessionReject(
        request_id: string,
        conversation_id: string,
        session_id: string,
        reason: string | null = null,
    ) {
        return this.data("session", {
            op: "session_reject",
            request_id,
            conversation_id,
            session_id,
            reason,
        });
    }
    sessionCancel(
        request_id: string,
        conversation_id: string,
        session_id: string,
    ) {
        return this.data("session", {
            op: "session_cancel",
            request_id,
            conversation_id,
            session_id,
        });
    }
    sessionEnd(
        request_id: string,
        conversation_id: string,
        session_id: string,
        reason: string | null = null,
    ) {
        return this.data("session", {
            op: "session_end",
            request_id,
            conversation_id,
            session_id,
            reason,
        });
    }
    sessionGet(conversation_id: string, session_id: string) {
        return this.data("session", {
            op: "session_get",
            conversation_id,
            session_id,
        });
    }
    sipDial(
        request_id: string,
        conversation_id: string,
        session_id: string,
        destination: string,
    ) {
        return this.data("sip_leg", {
            op: "sip_dial",
            request_id,
            conversation_id,
            session_id,
            destination,
        });
    }
    sipStatus(conversation_id: string, sip_leg_id: string) {
        return this.data("sip_leg", {
            op: "sip_status",
            conversation_id,
            sip_leg_id,
        });
    }
    sipEnd(request_id: string, conversation_id: string, sip_leg_id: string) {
        return this.data("sip_leg", {
            op: "sip_end",
            request_id,
            conversation_id,
            sip_leg_id,
        });
    }
    contextView(conversation_id: string, limit = 100) {
        return this.data("context_view", {
            op: "context_view",
            conversation_id,
            limit,
        });
    }
    availabilitySet(
        request_id: string,
        state: "available" | "unavailable",
        expires_at: number,
        preferred_route: string | null = null,
    ) {
        return this.data("availability", {
            op: "availability_set",
            request_id,
            state,
            preferred_route,
            expires_at,
        });
    }
    availabilityGet(identity_id: string) {
        return this.data("availability", {
            op: "availability_get",
            identity_id,
        });
    }
    artifactBegin(
        request_id: string,
        conversation_id: string,
        name: string,
        media_type: string,
        size: number,
        sha256: string,
        expires_at: number | null = null,
    ) {
        return this.data("artifact", {
            op: "artifact_begin",
            request_id,
            conversation_id,
            name,
            media_type,
            size,
            sha256,
            expires_at,
        });
    }
    artifactAppend(
        request_id: string,
        conversation_id: string,
        artifact_id: string,
        offset: number,
        data: string,
    ) {
        return this.data("artifact", {
            op: "artifact_append",
            request_id,
            conversation_id,
            artifact_id,
            offset,
            data,
        });
    }
    artifactFinish(
        request_id: string,
        conversation_id: string,
        artifact_id: string,
    ) {
        return this.data("artifact", {
            op: "artifact_finish",
            request_id,
            conversation_id,
            artifact_id,
        });
    }
    artifactGet(
        conversation_id: string,
        artifact_id: string,
        offset = 0,
        limit = 32768,
    ) {
        return this.data("artifact_chunk", {
            op: "artifact_get",
            conversation_id,
            artifact_id,
            offset,
            limit,
        });
    }
    artifactDelete(
        request_id: string,
        conversation_id: string,
        artifact_id: string,
    ) {
        return this.data("artifact", {
            op: "artifact_delete",
            request_id,
            conversation_id,
            artifact_id,
        });
    }
    health() {
        return this.data("health", { op: "health" });
    }
}
