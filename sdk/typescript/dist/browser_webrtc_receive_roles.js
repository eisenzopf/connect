import { ConnectError } from "./errors.js";
import { validateBrowserAnswer } from "./browser_capture.js";
import { originalWebRtcReceiveIntent } from "./browser_webrtc_receive_reservation.js";
const invalid = () => new ConnectError("protocol", "Native WebRTC receive reservation binding changed");
const mid = (value) => typeof value === "string" && /^[\x21-\x7e]{1,256}$/.test(value);
/** Optional internal metadata registry. No native operations/listeners/tasks are
 * created here. In particular a binding is NOT decoded-ready or public credit. */
export class WebRtcReceiveRoles {
    peer;
    intent;
    scope;
    slots = new Map();
    constructor(peer, scope, intent, transceivers) {
        if (!originalWebRtcReceiveIntent(intent, scope))
            throw invalid();
        if (peer.connectionState !== "new" || peer.signalingState !== "stable" || peer.localDescription !== null || peer.remoteDescription !== null)
            throw invalid();
        const selected = [transceivers.audio, transceivers.preview, transceivers.public];
        const actual = peer.getTransceivers();
        if (new Set(selected).size !== 3 || actual.length !== 3 ||
            selected.some(item => actual.filter(value => value === item).length !== 1) ||
            new Set(selected.map(item => item.receiver)).size !== 3 ||
            selected.some((item, index) => item.direction !== (index === 2 ? "recvonly" : "sendrecv")))
            throw invalid();
        const ids = [scope.audioStreamId, intent.preview_stream_id, intent.public_stream_id];
        for (const [index, role] of ["audio", "preview", "public"].entries()) {
            const transceiver = selected[index];
            if (!transceiver.receiver || transceiver.receiver.track.kind !== (index === 0 ? "audio" : "video"))
                throw invalid();
            this.slots.set(role, { role, streamId: ids[index], transceiver, receiver: transceiver.receiver });
        }
        this.intent = intent;
        this.scope = Object.freeze({ recordId: scope.recordId, operationId: scope.operationId,
            targetConnectionId: scope.targetConnectionId, audioStreamId: scope.audioStreamId,
            previewStreamId: scope.previewStreamId, publicStreamId: scope.publicStreamId });
        this.peer = peer;
        this.check();
    }
    check() {
        const peer = this.peer;
        if (!peer)
            throw new ConnectError("closed", "Original WebRTC receive roles retired");
        if (!this.intent || !this.scope || !originalWebRtcReceiveIntent(this.intent, this.scope))
            throw invalid();
        if (["closed", "failed"].includes(peer.connectionState))
            throw invalid();
        const actual = peer.getTransceivers();
        if (actual.length !== 3 || new Set(actual).size !== 3)
            throw invalid();
        const mids = [];
        for (const slot of this.slots.values()) {
            if (!actual.includes(slot.transceiver) || slot.transceiver.receiver !== slot.receiver ||
                slot.transceiver.direction !== (slot.role === "public" ? "recvonly" : "sendrecv") ||
                (slot.transceiver.currentDirection !== null && slot.transceiver.currentDirection !== (slot.role === "public" ? "recvonly" : "sendrecv")))
                throw invalid();
            const value = slot.transceiver.mid;
            if (value === null) {
                if (slot.mid !== undefined)
                    throw invalid();
            }
            else {
                if (!mid(value) || (slot.mid !== undefined && value !== slot.mid) || mids.includes(value))
                    throw invalid();
                slot.mid = value;
                mids.push(value);
            }
            if (slot.track && (slot.receiver.track !== slot.track || slot.track.readyState !== "live"))
                throw invalid();
        }
    }
    accept(event) {
        this.check();
        const slot = [...this.slots.values()].find(value => value.transceiver === event.transceiver);
        if (!slot || event.receiver !== slot.receiver || slot.receiver.track !== event.track ||
            event.track.kind !== (slot.role === "audio" ? "audio" : "video") || event.track.readyState !== "live" ||
            (slot.track !== undefined && slot.track !== event.track))
            throw invalid();
        slot.track = event.track;
        return slot.role;
    }
    binding(role) {
        this.check();
        const slot = this.slots.get(role);
        if (!slot.track || slot.mid === undefined)
            return undefined;
        return Object.freeze({ role: slot.role, streamId: slot.streamId, mid: slot.mid,
            transceiver: slot.transceiver, receiver: slot.receiver, track: slot.track });
    }
    /** Exact original three-m-section answer and native pointer/MID association.
     * This is negotiation only: no packet, decode, source cut or activation. */
    validateAnswer(sdp) {
        this.check();
        if (!sdp.startsWith("v=0\r\n") || sdp.includes("\0") ||
            new TextEncoder().encode(sdp).byteLength > 512 * 1024 ||
            this.peer.remoteDescription?.sdp !== sdp)
            throw invalid();
        const sections = sdp.replace(/\r/g, "").split(/\n(?=m=)/);
        const session = sections.shift(), media = sections;
        const inherited = session.match(/^a=(sendrecv|sendonly|recvonly|inactive)$/m)?.[1] ?? "sendrecv";
        if (media.length !== 3)
            throw invalid();
        const used = new Set();
        for (const section of media) {
            const fields = section.split("\n", 1)[0].split(/\s+/);
            const mids = [...section.matchAll(/^a=mid:([^\n]+)$/gm)].map(value => value[1]);
            if (mids.length !== 1 || !mid(mids[0]) || used.has(mids[0]) ||
                !/^\d+$/.test(fields[1]) || Number(fields[1]) === 0)
                throw invalid();
            used.add(mids[0]);
            const slot = [...this.slots.values()].find(value => value.mid === mids[0]);
            const direction = section.match(/^a=(sendrecv|sendonly|recvonly|inactive)$/m)?.[1] ?? inherited;
            if (!slot || fields[0] !== (slot.role === "audio" ? "m=audio" : "m=video") ||
                direction !== (slot.role === "public" ? "sendonly" : "sendrecv") ||
                slot.transceiver.currentDirection !== (slot.role === "public" ? "recvonly" : "sendrecv"))
                throw invalid();
            if (slot.role === "audio") {
                // Validate the original audio section with the existing fixed
                // audio validator. This does not alter either native SDP.
                validateBrowserAnswer(`${session}\n${section}`.replace(/\n/g, "\r\n"), false);
                continue;
            }
            const tokens = fields.slice(3), payloads = tokens.map(Number);
            if (!tokens.length || tokens.some(pt => !/^\d+$/.test(pt) || Number(pt) > 127) ||
                new Set(payloads).size !== payloads.length)
                throw invalid();
            const codecs = new Map();
            for (const value of section.matchAll(/^a=rtpmap:(\d+)[ \t]+([^\n]+)$/gim)) {
                const pt = Number(value[1]);
                if (!payloads.includes(pt))
                    continue;
                if (codecs.has(pt))
                    throw invalid();
                codecs.set(pt, value[2].trim().toLowerCase());
            }
            const primary = payloads.filter(pt => codecs.get(pt) === "vp8/90000");
            if (!primary.length)
                throw invalid();
            for (const pt of payloads) {
                if (primary.includes(pt))
                    continue;
                if (codecs.get(pt) !== "rtx/90000")
                    throw invalid();
                const parameters = [...section.matchAll(/^a=fmtp:(\d+)[ \t]+([^\n]+)$/gim)].filter(value => Number(value[1]) === pt);
                if (parameters.length !== 1)
                    throw invalid();
                const associations = parameters[0][2].split(";").map(value => value.trim()).filter(value => /^apt(?:\s*=|$)/i.test(value));
                if (associations.length !== 1)
                    throw invalid();
                const apt = /^apt\s*=\s*(\d+)$/i.exec(associations[0]);
                if (!apt || !primary.includes(Number(apt[1])))
                    throw invalid();
            }
        }
    }
    close() { this.slots.clear(); this.peer = undefined; this.intent = undefined; this.scope = undefined; }
}
/** Absence preserves the default without inspecting or changing a native peer.
 * The future constructor must itself create B before its initial offer only
 * after the original authenticated preparing snapshot supplied this intent. */
export function receiveRolesForOriginalIntent(peer, scope, intent, transceivers) {
    if (intent === undefined)
        return undefined;
    return new WebRtcReceiveRoles(peer, scope, intent, transceivers);
}
