import { object, integer } from "./validation.js";
const closed = (v, keys) => object(v) &&
    Object.keys(v).sort().join(" ") === keys.split(" ").sort().join(" ");
const text = (v, max = 128) => typeof v === "string" &&
    v.length > 0 &&
    new TextEncoder().encode(v).length <= max &&
    !/[\u0000-\u001f\u007f-\u009f]/.test(v);
const identifier = (v) => text(v) && /^[A-Za-z0-9._~-]+$/.test(v);
const reason = (v) => v === null ||
    (typeof v === "string" &&
        new TextEncoder().encode(v).length <= 512 &&
        !/[\u0000-\u001f\u007f-\u009f]/.test(v));
const epoch = (v) => integer(v) && Number(v) > 0;
function member(v) {
    return (closed(v, "participant_id conversation_id identity_id kind role membership_epoch active removal_reason") &&
        [v.participant_id, v.conversation_id, v.identity_id].every((x) => identifier(x)) &&
        typeof v.kind === "string" &&
        ["human", "ai", "system", "external"].includes(v.kind) &&
        text(v.role, 64) &&
        epoch(v.membership_epoch) &&
        typeof v.active === "boolean" &&
        reason(v.removal_reason));
}
function invitation(v) {
    return (closed(v, "invitation_id conversation_id identity_id expires_at state conversation_revision") &&
        [v.invitation_id, v.conversation_id, v.identity_id].every((x) => identifier(x)) &&
        epoch(v.expires_at) &&
        integer(v.conversation_revision) &&
        typeof v.state === "string" &&
        ["pending", "joined", "declined", "cancelled", "expired"].includes(v.state));
}
function receipt(v, cid) {
    return (closed(v, "operation_id conversation_id state completion_boundary status_reference") &&
        identifier(v.operation_id) &&
        v.conversation_id === cid &&
        v.state === "completed" &&
        v.completion_boundary === "local-mutation" &&
        closed(v.status_reference, "action operation_id") &&
        v.status_reference.action === "get" &&
        v.status_reference.operation_id === v.operation_id);
}
export function validMembership(kind, v) {
    if (kind === "membership_snapshot")
        return (closed(v, "conversation_id revision participants") &&
            identifier(v.conversation_id) &&
            integer(v.revision) &&
            Array.isArray(v.participants) &&
            v.participants.length <= 128 &&
            v.participants.every((p) => member(p) &&
                object(p) &&
                p.conversation_id === v.conversation_id));
    if (kind === "membership_invitations")
        return (closed(v, "invitations next") &&
            Array.isArray(v.invitations) &&
            v.invitations.length <= 100 &&
            v.invitations.every(invitation) &&
            (v.next === null || text(v.next, 2048)));
    if (v.kind === "participant")
        return (closed(v, "kind participant revision receipt") &&
            member(v.participant) &&
            object(v.participant) &&
            integer(v.revision) &&
            receipt(v.receipt, v.participant.conversation_id));
    if (v.kind === "invitation")
        return (closed(v, "kind invitation receipt") &&
            invitation(v.invitation) &&
            object(v.invitation) &&
            receipt(v.receipt, v.invitation.conversation_id));
    if (v.kind === "sharing")
        return (closed(v, "kind sharing_id conversation_id revision active receipt") &&
            identifier(v.sharing_id) &&
            identifier(v.conversation_id) &&
            integer(v.revision) &&
            typeof v.active === "boolean" &&
            receipt(v.receipt, v.conversation_id));
    return false;
}
