import { ConnectError } from "./errors.js";
/** Shared public operations. Mutation IDs belong to the caller. */
export class Operations {
    async data(kind, operation) {
        const result = await this.request(operation);
        if (result.kind !== kind)
            throw new ConnectError("protocol", "Unexpected outcome");
        return result.data;
    }
    resolve(address) {
        return this.data("identity", { op: "resolve", address });
    }
    create(request_id, participants) {
        return this.data("conversation", {
            op: "create",
            request_id,
            participants,
        });
    }
    send(request_id, conversation_id, body) {
        return this.data("accepted", {
            op: "send",
            request_id,
            conversation_id,
            body,
        });
    }
    history(conversation_id, after = null, limit = 100) {
        return this.data("history", {
            op: "history",
            conversation_id,
            after,
            limit,
        });
    }
    inbox(cursor = null, limit = 100, wait_ms = 0) {
        return this.data("inbox", { op: "inbox", cursor, limit, wait_ms });
    }
    requestStatus(request_id) {
        return this.data("request_status", {
            op: "request_status",
            request_id,
        });
    }
    providerStatus(conversation_id) {
        return this.data("provider_status", {
            op: "provider_status",
            conversation_id,
        });
    }
    closeConversation(request_id, conversation_id) {
        return this.data("conversation", {
            op: "close",
            request_id,
            conversation_id,
        });
    }
    callStart(request_id, conversation_id, route_id, destination) {
        return this.data("call", {
            op: "call_start",
            request_id,
            conversation_id,
            route_id,
            destination,
        });
    }
    callStatus(conversation_id, call_id) {
        return this.data("call", {
            op: "call_status",
            conversation_id,
            call_id,
        });
    }
    callEnd(request_id, conversation_id, call_id) {
        return this.data("call", {
            op: "call_end",
            request_id,
            conversation_id,
            call_id,
        });
    }
    sessionInvite(request_id, conversation_id, participant_id, media) {
        return this.data("session", {
            op: "session_invite",
            request_id,
            conversation_id,
            participant_id,
            media,
        });
    }
    sessionAccept(request_id, conversation_id, session_id) {
        return this.data("session", {
            op: "session_accept",
            request_id,
            conversation_id,
            session_id,
        });
    }
    sessionReject(request_id, conversation_id, session_id, reason = null) {
        return this.data("session", {
            op: "session_reject",
            request_id,
            conversation_id,
            session_id,
            reason,
        });
    }
    sessionCancel(request_id, conversation_id, session_id) {
        return this.data("session", {
            op: "session_cancel",
            request_id,
            conversation_id,
            session_id,
        });
    }
    sessionEnd(request_id, conversation_id, session_id, reason = null) {
        return this.data("session", {
            op: "session_end",
            request_id,
            conversation_id,
            session_id,
            reason,
        });
    }
    sessionGet(conversation_id, session_id) {
        return this.data("session", {
            op: "session_get",
            conversation_id,
            session_id,
        });
    }
    sipDial(request_id, conversation_id, session_id, destination) {
        return this.data("sip_leg", {
            op: "sip_dial",
            request_id,
            conversation_id,
            session_id,
            destination,
        });
    }
    sipStatus(conversation_id, sip_leg_id) {
        return this.data("sip_leg", {
            op: "sip_status",
            conversation_id,
            sip_leg_id,
        });
    }
    sipEnd(request_id, conversation_id, sip_leg_id) {
        return this.data("sip_leg", {
            op: "sip_end",
            request_id,
            conversation_id,
            sip_leg_id,
        });
    }
    contextView(conversation_id, limit = 100) {
        return this.data("context_view", {
            op: "context_view",
            conversation_id,
            limit,
        });
    }
    availabilitySet(request_id, state, expires_at, preferred_route = null) {
        return this.data("availability", {
            op: "availability_set",
            request_id,
            state,
            preferred_route,
            expires_at,
        });
    }
    availabilityGet(identity_id) {
        return this.data("availability", {
            op: "availability_get",
            identity_id,
        });
    }
    artifactBegin(request_id, conversation_id, name, media_type, size, sha256, expires_at = null) {
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
    artifactAppend(request_id, conversation_id, artifact_id, offset, data) {
        return this.data("artifact", {
            op: "artifact_append",
            request_id,
            conversation_id,
            artifact_id,
            offset,
            data,
        });
    }
    artifactFinish(request_id, conversation_id, artifact_id) {
        return this.data("artifact", {
            op: "artifact_finish",
            request_id,
            conversation_id,
            artifact_id,
        });
    }
    artifactGet(conversation_id, artifact_id, offset = 0, limit = 32768) {
        return this.data("artifact_chunk", {
            op: "artifact_get",
            conversation_id,
            artifact_id,
            offset,
            limit,
        });
    }
    artifactDelete(request_id, conversation_id, artifact_id) {
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
