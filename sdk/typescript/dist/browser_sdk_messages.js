import { fields, nonEmpty } from "./browser_sdk_sessions.js";
import { identifier } from "./uctp_wire.js";
import { canonicalJson, jsonObject, } from "./uctp_json.js";
export function messageBody(input, bad) {
    fields(input, ["messageId", "to", "text"], bad);
    if (!identifier(input.messageId) ||
        typeof input.text !== "string" ||
        new TextEncoder().encode(input.text).length > 512 * 1024)
        return bad();
    const to = nonEmpty(input.to, identifier, 100, bad);
    if (new Set(to).size !== to.length)
        return bad();
    const body = {
        msg_id: input.messageId,
        to,
        content_type: "text/plain",
        body: input.text,
        body_encoding: "utf8",
        label: "connect-messages",
        attachments: [],
        in_reply_to_msg: null,
    };
    try {
        canonicalJson(body);
    }
    catch {
        return bad();
    }
    return body;
}
export function messageResult(value, key, bad) {
    if (!jsonObject(value) ||
        value.kind !== "accepted" ||
        !jsonObject(value.data))
        return bad();
    const d = value.data, r = d.receipt;
    if (d.request_id !== key.operationId ||
        d.conversation_id !== key.scope.conversationId ||
        !identifier(d.message_id) ||
        !jsonObject(r) ||
        r.operation_id !== key.operationId ||
        r.conversation_id !== key.scope.conversationId ||
        r.state !== "completed" ||
        r.completion_boundary !== "local-mutation" ||
        !jsonObject(r.status_reference) ||
        r.status_reference.action !== "get" ||
        r.status_reference.operation_id !== key.operationId)
        return bad();
    return Object.freeze({
        messageId: d.message_id,
        operation: key,
        boundary: "local-mutation",
    });
}
