// Internal metadata validation. Only the original signed operation reader may
// supply these values to the owned decoder lifecycle; parsing grants no IO.
import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";
import { canonicalJson } from "./uctp_json.js";
export const WEBRTC_BRIDGE_OPT_IN = Object.freeze({ profile: "connect/webrtc-public-video-precut-bridge/1", carrier: "webrtc-rtp", presentation: "exclusive-public", feed: "public-programme-private-decode" });
const bad = () => new ConnectError("protocol", "Original signed B bridge metadata changed");
export const bridgeCanonical = (value) => canonicalJson(value);
export function bridgeClosed(value, fields) {
    if (value === null || typeof value !== "object" || Array.isArray(value))
        throw bad();
    const keys = Reflect.ownKeys(value), d = Object.getOwnPropertyDescriptors(value);
    if (keys.length !== fields.length || keys.some(k => typeof k !== "string" || !fields.includes(k)) || fields.some(k => !d[k] || !Object.hasOwn(d[k], "value")))
        throw bad();
    return Object.fromEntries(fields.map(k => [k, d[k].value]));
}
const uint = (v, max = 0xffffffff) => Number.isSafeInteger(v) && Number(v) >= 0 && Number(v) <= max;
const positive = (v) => uint(v, Number.MAX_SAFE_INTEGER) && Number(v) > 0;
const opaque = (v) => typeof v === "string" && /^[\x21-\x7e]{1,256}$/.test(v);
const hex = (v) => typeof v === "string" && /^[0-9a-f]{32}$/.test(v) && !/^0+$/.test(v);
const decimal = (v) => typeof v === "string" && /^(0|[1-9][0-9]{0,19})$/.test(v) && (v.length < 20 || v <= "18446744073709551615");
const bindingFields = ["stream_id", "preview_stream_id", "mid", "stream_generation", "physical_writer_incarnation", "connection_generation", "primary_ssrc", "payload_type", "clock_rate_hz", "codec"];
export function parseWebRtcBridgeOptIn(value) {
    const v = bridgeClosed(value, ["profile", "carrier", "presentation", "feed"]);
    if (bridgeCanonical(v) !== bridgeCanonical(WEBRTC_BRIDGE_OPT_IN))
        throw bad();
    return WEBRTC_BRIDGE_OPT_IN;
}
function binding(value, expected) {
    const b = bridgeClosed(value, bindingFields);
    if (bindingFields.some(k => b[k] !== expected[k]))
        throw bad();
    return Object.freeze({ ...expected });
}
export function parseWebRtcBridgeSnapshot(value, root, reservationId, expected, privateNonce, reservationNonce, targetGeneration) {
    const v = bridgeClosed(value, ["opt_in", "feeding", "decoded_ready"]);
    const opt_in = parseWebRtcBridgeOptIn(v.opt_in);
    let feeding = null, decoded_ready = null;
    if (v.feeding !== null) {
        const f = bridgeClosed(v.feeding, ["profile", "phase", "reservation_id", "operation_record_id", "operation_id", "target_connection_id", "target_generation", "binding", "bridge_readiness"]);
        const c = bridgeClosed(f.bridge_readiness, ["nonce", "expires_at", "decoder_startup"]);
        if (f.profile !== opt_in.profile || f.phase !== "feeding" || f.reservation_id !== reservationId || f.operation_record_id !== root.recordId || f.operation_id !== root.operationId || f.target_connection_id !== root.targetConnectionId || !positive(f.target_generation) || f.target_generation !== targetGeneration
            || !identifier(c.nonce) || c.nonce === privateNonce || c.nonce === reservationNonce || c.nonce === reservationId || !positive(c.expires_at) || c.decoder_startup !== "fresh-b-receiver-precut-keyframe")
            throw bad();
        feeding = Object.freeze({ ...f, binding: binding(f.binding, expected), bridge_readiness: Object.freeze({ ...c }) });
    }
    if (v.decoded_ready !== null) {
        if (!feeding)
            throw bad();
        const m = bridgeClosed(v.decoded_ready, ["profile", "operation_record_id", "bridge_readiness"]);
        const r = bridgeClosed(m.bridge_readiness, ["reservation_id", "nonce", "binding", "decoded_rtp_timestamp", "decoded_frames", "decoder_startup"]);
        if (m.profile !== "connect-replacement-public-video-bridge-ready-v1" || m.operation_record_id !== root.recordId || r.reservation_id !== reservationId || r.nonce !== feeding.bridge_readiness.nonce || !uint(r.decoded_rtp_timestamp) || !positive(r.decoded_frames) || r.decoder_startup !== feeding.bridge_readiness.decoder_startup)
            throw bad();
        decoded_ready = Object.freeze({ ...m, bridge_readiness: Object.freeze({ ...r, binding: binding(r.binding, expected) }) });
    }
    return Object.freeze({ opt_in, feeding, decoded_ready });
}
/** Original signed writer receipts only; absence means the same poll is pending.
 * RTP order is modular on this exact writer, including a normal u32 wrap. */
export function parseWebRtcPreCutKeyframes(value) {
    if (value === undefined)
        return undefined;
    if (!Array.isArray(value) || value.length < 1 || value.length > 16 || !Array.from(value).every(v => uint(v)) || new Set(value).size !== value.length)
        throw bad();
    for (let i = 1; i < value.length; i++) {
        const distance = (value[i] - value[i - 1]) >>> 0;
        if (!distance || distance >= 0x80000000)
            throw bad();
    }
    return Object.freeze([...value]);
}
const scopeFields = ["operation_record_id", "operation_id", "reservation_id", "source_connection_id", "target_connection_id", "stream_id", "stream_generation", "connection_generation", "target_generation", "source_cut_id"];
export function receiverScope(root, phase) {
    if (!opaque(phase.source_cut_id))
        throw bad();
    return Object.freeze({ operation_record_id: root.recordId, operation_id: root.operationId, reservation_id: phase.reservation_id, source_connection_id: root.sourceConnectionId, target_connection_id: root.targetConnectionId,
        stream_id: phase.binding.stream_id, stream_generation: phase.binding.stream_generation, connection_generation: phase.binding.connection_generation, target_generation: phase.target_generation, source_cut_id: phase.source_cut_id });
}
export function parseWebRtcReceiverPostCut(value, root, phase) {
    const v = bridgeClosed(value, ["profile", ...scopeFields, "original_frame", "floor"]), expected = receiverScope(root, phase);
    const f = bridgeClosed(v.floor, ["carrier", "physical_writer_incarnation", "primary_ssrc", "first_public_rtp_timestamp"]), g = bridgeClosed(v.original_frame, ["realm", "ordinal", "last_old_ordinal"]);
    if (v.profile !== "connect-receiver-video-post-cut-v1" || scopeFields.some(k => v[k] !== expected[k]) || !hex(g.realm) || !decimal(g.ordinal) || !decimal(g.last_old_ordinal)
        || !(g.ordinal.length > g.last_old_ordinal.length || (g.ordinal.length === g.last_old_ordinal.length && g.ordinal > g.last_old_ordinal))
        || f.carrier !== "webrtc-rtp" || f.physical_writer_incarnation !== phase.binding.physical_writer_incarnation || f.primary_ssrc !== phase.binding.primary_ssrc || !uint(f.first_public_rtp_timestamp))
        throw bad();
    return Object.freeze({ ...expected, profile: "connect-receiver-video-post-cut-v1", original_frame: Object.freeze(g), floor: Object.freeze(f) });
}
/** SAME physical writer only. Half-range/absence/negative/fraction never
 * becomes fresh output; no counter/mediaTime/generation substitution. */
export function postCutRtpEligible(witness, output) {
    return !!output && uint(output.rtpTimestamp) && positive(output.presentedFrames) && ((output.rtpTimestamp - witness.floor.first_public_rtp_timestamp) >>> 0) < 0x80000000;
}
