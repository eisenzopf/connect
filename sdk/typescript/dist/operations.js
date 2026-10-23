import { TaskOperations } from "./task_operations.js";
import { ConnectError } from "./errors.js";
/** Shared public operations. Mutation IDs belong to the caller. */
export class Operations extends TaskOperations {
    async data(kind, operation) {
        const result = await this.request(operation);
        if (result.kind !== kind)
            throw new ConnectError("protocol", "Unexpected outcome");
        if ([
            "membership_snapshot",
            "conversation_snapshot",
            "conversation_events",
            "session_observations",
        ].includes(kind)) {
            const data = result.data;
            if (!("conversation_id" in operation) ||
                data.conversation_id !== operation.conversation_id ||
                ("session_id" in operation &&
                    data.session_id !== operation.session_id))
                throw new ConnectError("protocol", "Event result scope does not match the request");
        }
        return result.data;
    }
    conversationSnapshot(conversation_id, query = {}) {
        return this.data("conversation_snapshot", {
            op: "conversation_snapshot",
            conversation_id,
            query,
        });
    }
    conversationEvents(conversation_id, query = {}) {
        return this.data("conversation_events", {
            op: "conversation_events",
            conversation_id,
            query,
        });
    }
    sessionObservations(conversation_id, session_id, query = {}) {
        return this.data("session_observations", {
            op: "session_observations",
            conversation_id,
            session_id,
            query,
        });
    }
    admitTransition(request) {
        return this.data("durable_operation", { op: "transition", request });
    }
    operationAction(request) {
        return this.data("durable_operation", {
            op: "durable_operation",
            request,
        });
    }
    contactPairingStart(request_id, peer_identity_id, expires_at) {
        return this.data("contact_pairing", {
            op: "contact_pairing_start",
            request_id,
            peer_identity_id,
            expires_at,
        });
    }
    contactPairingComplete(request_id, pairing_id, challenge) {
        return this.data("contact_pairing", {
            op: "contact_pairing_complete",
            request_id,
            pairing_id,
            challenge,
        });
    }
    contactPairingRevoke(request_id, pairing_id) {
        return this.data("contact_pairing", {
            op: "contact_pairing_revoke",
            request_id,
            pairing_id,
        });
    }
    contactState(peer_identity_id) {
        return this.data("contact", { op: "contact_state", peer_identity_id });
    }
    capabilities(address, conversation_id = null) {
        return this.data("capabilities", {
            op: "capabilities",
            address,
            conversation_id,
        });
    }
    browserBinding(conversation_id, session_id) {
        if ((conversation_id === undefined) !== (session_id === undefined))
            throw new Error("Select both conversation_id and session_id");
        return this.data("browser_binding", {
            op: "browser_binding",
            ...(conversation_id === undefined
                ? {}
                : { conversation_id, session_id }),
        });
    }
    contextDeliveries(conversation_id, before = null, limit = 100) {
        return this.data("context_deliveries", {
            op: "context_deliveries",
            conversation_id,
            before,
            limit,
        });
    }
    resolve(address) {
        return this.data("identity", { op: "resolve", address });
    }
    create(request_id, participants) {
        return this.createConversation(request_id, {
            tenant_id: null,
            policy: "persistent",
            idle_close_secs: null,
            metadata: null,
            initial_participants: [...new Set(participants)]
                .sort()
                .map((identity_id) => ({ identity_id, role: "participant" })),
        });
    }
    async createConversation(request_id, request) {
        const value = await this.data("conversation", {
            op: "create",
            request_id,
            request,
        });
        if (value.receipt?.operation_id !== request_id)
            throw new ConnectError("protocol", "Conversation receipt does not match its mutation");
        return value;
    }
    listConversations(query = {}) {
        return this.data("conversations", { op: "conversations", query });
    }
    async changeMembership(request) {
        const value = await this.data("membership", {
            op: "membership",
            request,
        });
        if (value.receipt.operation_id !== request.operation_id ||
            value.receipt.conversation_id !== request.conversation_id)
            throw new ConnectError("protocol", "Membership receipt does not match its mutation");
        return value;
    }
    membershipSnapshot(conversation_id) {
        return this.data("membership_snapshot", {
            op: "membership_snapshot",
            conversation_id,
        });
    }
    membershipInvitations(after = null, limit = 100) {
        return this.data("membership_invitations", {
            op: "membership_invitations",
            after,
            limit,
        });
    }
    /** Plain UTF-8 text to all current members; audience is frozen on acceptance. */
    send(request_id, conversation_id, body) {
        return this.sendMessage(request_id, conversation_id, {
            to: "all",
            content_type: "text/plain",
            body,
            body_encoding: "utf8",
            label: "connect-messages",
            attachments: [],
            in_reply_to_msg: null,
            msg_id: null,
        });
    }
    async sendMessage(request_id, conversation_id, message) {
        const data = await this.data("accepted", {
            op: "send",
            request_id,
            conversation_id,
            message,
        });
        if (!data.receipt ||
            data.request_id !== request_id ||
            data.conversation_id !== conversation_id ||
            data.receipt.operation_id !== request_id ||
            data.receipt.conversation_id !== conversation_id)
            throw new ConnectError("protocol", "Message receipt does not match its mutation");
        return { ...data, receipt: data.receipt };
    }
    messageReceipts(conversation_id, message_id, after = null, limit = 100) {
        return this.data("message_receipts", {
            op: "message_receipts",
            conversation_id,
            message_id,
            after,
            limit,
        });
    }
    messageReceipt(request_id, conversation_id, message_id, state) {
        return this.data("receipt_acknowledged", {
            op: "message_receipt",
            request_id,
            conversation_id,
            message_id,
            state,
        });
    }
    receiptPreferences(conversation_id) {
        return this.data("receipt_preferences", {
            op: "receipt_preferences_get",
            conversation_id,
        });
    }
    setReceiptPreferences(request_id, conversation_id, delivery, read) {
        return this.data("receipt_preferences", {
            op: "receipt_preferences_set",
            request_id,
            conversation_id,
            delivery,
            read,
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
    async closeConversation(request_id, conversation_id, options = {
        expected_revision: null,
        reason_code: null,
        reason: null,
    }) {
        const value = await this.data("conversation", {
            op: "close",
            request_id,
            conversation_id,
            options,
        });
        if (value.id !== conversation_id ||
            value.receipt?.operation_id !== request_id)
            throw new ConnectError("protocol", "Conversation receipt does not match its mutation");
        return value;
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
    artifactBegin(request_id, conversation_id, name, media_type, size, sha256, expires_at = null, to = "all") {
        return this.data("artifact", {
            op: "artifact_begin",
            to,
            request_id,
            conversation_id,
            name,
            media_type,
            size,
            sha256,
            expires_at,
        });
    }
    artifactInspect(conversation_id, artifact_id) {
        return this.data("artifact", {
            op: "artifact_inspect",
            conversation_id,
            artifact_id,
        });
    }
    artifactUploadLink(conversation_id, artifact_id, expires_in = 60) {
        return this.data("artifact_link", {
            op: "artifact_upload_link",
            conversation_id,
            artifact_id,
            expires_in,
        });
    }
    artifactDownloadLink(conversation_id, artifact_id, message_id, expires_in = 60) {
        return this.data("artifact_link", {
            op: "artifact_download_link",
            conversation_id,
            artifact_id,
            message_id,
            expires_in,
        });
    }
    artifactLinkGet(link, offset = 0, limit = 32768) {
        return this.data("artifact_chunk", {
            op: "artifact_link_get",
            conversation_id: link.conversation_id,
            artifact_id: link.artifact_id,
            link: link.path,
            offset,
            limit,
        });
    }
    /** data is base64, and request_id remains stable when refreshing a link. */
    artifactLinkAppend(request_id, link, offset, data) {
        return this.data("artifact", {
            op: "artifact_link_append",
            request_id,
            conversation_id: link.conversation_id,
            artifact_id: link.artifact_id,
            link: link.path,
            offset,
            data,
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
