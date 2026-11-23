// CANDIDATE ONLY. Package-private receiver membership, never an authorization issuer.
// The owning signed-operation reader and physical decoder workflow must call it.
import { ConnectError } from "./errors.js";
import { identifier } from "./uctp_wire.js";
import { canonicalJson } from "./uctp_json.js";
import { nativeCut, nativeMembership } from "./browser_native_activation.js";
const scopeFields = ["operation_record_id", "operation_id", "reservation_id", "source_connection_id", "target_connection_id", "stream_id", "stream_generation", "connection_generation", "target_generation", "source_cut_id"];
const rtpFields = ["carrier", "physical_writer_incarnation", "primary_ssrc", "first_public_rtp_timestamp"];
const invalid = () => new ConnectError("protocol", "Original receiver release changed");
const uint = (v, max = 0xffffffff) => Number.isSafeInteger(v) && Number(v) >= 0 && Number(v) <= max;
const positive = (v) => uint(v, Number.MAX_SAFE_INTEGER) && Number(v) > 0;
const hex = (v) => typeof v === "string" && /^[0-9a-f]{32}$/.test(v) && !/^0+$/.test(v);
const decimal = (v) => typeof v === "string" && /^(0|[1-9][0-9]{0,19})$/.test(v) && (v.length < 20 || v <= "18446744073709551615");
function closed(value, fields) {
    if (value === null || typeof value !== "object" || Array.isArray(value))
        throw invalid();
    const keys = Reflect.ownKeys(value), descriptors = Object.getOwnPropertyDescriptors(value);
    if (keys.length !== fields.length || keys.some(key => typeof key !== "string" || !fields.includes(key)) || fields.some(key => !descriptors[key] || !Object.hasOwn(descriptors[key], "value")))
        throw invalid();
    return Object.fromEntries(fields.map(key => [key, descriptors[key].value]));
}
function scope(value) {
    const v = closed(value, scopeFields);
    if (!scopeFields.slice(0, 6).every(key => identifier(v[key])) || v.source_connection_id === v.target_connection_id || !positive(v.stream_generation) || !positive(v.connection_generation) || !positive(v.target_generation) || !identifier(v.source_cut_id))
        throw invalid();
    return Object.freeze(v);
}
function sameScope(a, b) {
    return scopeFields.every(key => a[key] === b[key]);
}
function parse(value, expected, identity) {
    const v = closed(value, ["profile", ...scopeFields, "original_frame", "floor"]);
    if (v.profile !== "connect-receiver-video-release-v1" || !sameScope(v, expected))
        throw invalid();
    const frame = closed(v.original_frame, ["realm", "ordinal", "last_old_ordinal"]);
    if (!hex(frame.realm) || !decimal(frame.ordinal) || !decimal(frame.last_old_ordinal) || !(frame.ordinal.length > frame.last_old_ordinal.length || (frame.ordinal.length === frame.last_old_ordinal.length && frame.ordinal > frame.last_old_ordinal)))
        throw invalid();
    let floor;
    if (identity.carrier === "webrtc-rtp") {
        const f = closed(v.floor, rtpFields);
        if (f.carrier !== identity.carrier || f.physical_writer_incarnation !== identity.physical_writer_incarnation || f.primary_ssrc !== identity.primary_ssrc || !uint(f.first_public_rtp_timestamp))
            throw invalid();
        floor = Object.freeze(f);
    }
    else {
        const f = closed(v.floor, ["carrier", "cut"]);
        const cut = nativeCut(f.cut);
        if (f.carrier !== identity.carrier || cut.stream_id !== expected.stream_id || cut.stream_generation !== expected.stream_generation || cut.physical_incarnation !== identity.physical_incarnation || cut.stream_local_id !== identity.stream_local_id)
            throw invalid();
        floor = Object.freeze({ carrier: "native-datagram", cut });
    }
    return Object.freeze({ ...expected, profile: "connect-receiver-video-release-v1", original_frame: Object.freeze(frame), floor });
}
/** One owner for the actual warmed receiver/decoder and original operation.
 * ownsPhysical MUST perform only existing local identity/state checks, no SQL,
 * registry admission, network IO, or locks. State-change retirement closes owner.
 * This local capability cannot substitute for cross-device old-output detach ACK.
 */
export class ReceiverVideoReleaseOwner {
    ownsPhysical;
    scope;
    release;
    rawIdentity;
    token;
    lastRtcPresented = 0;
    lastRtcTimestamp;
    lastNativeSequence = -1;
    stopped = false;
    identity;
    constructor(scopeValue, identityValue, ownsPhysical) {
        this.ownsPhysical = ownsPhysical;
        this.scope = scope(scopeValue);
        const identity = Object.freeze(closed(identityValue, identityValue.carrier === "webrtc-rtp"
            ? ["carrier", "physical_writer_incarnation", "primary_ssrc"]
            : ["carrier", "physical_incarnation", "stream_local_id"]));
        this.identity = identity;
        if (identity.carrier === "webrtc-rtp") {
            if (!hex(identity.physical_writer_incarnation) || !uint(identity.primary_ssrc))
                throw invalid();
        }
        else if (identity.carrier !== "native-datagram" || !hex(identity.physical_incarnation) || !positive(identity.stream_local_id) || identity.stream_local_id > 65535)
            throw invalid();
        this.check();
    }
    check() {
        if (this.stopped)
            throw new ConnectError("closed", "Original receiver release owner retired");
        if (!this.ownsPhysical()) {
            this.close();
            throw new ConnectError("closed", "Original receiver release owner retired");
        }
    }
    /** Call ONLY after verified original release authorization and actual old
     * output-detach acknowledgment; never from a caller supplied media option.
     * No raw PCM/image is retagged; only future matching original outputs pass.
     */
    observe(value, original) {
        this.check();
        if (original.operationRecordId !== this.scope.operation_record_id || original.operationId !== this.scope.operation_id || original.targetConnectionId !== this.scope.target_connection_id || original.releaseAuthorized !== true)
            throw invalid();
        const parsed = parse(value, this.scope, this.identity);
        const stable = canonicalJson(parsed);
        if (this.rawIdentity !== undefined && stable !== this.rawIdentity)
            throw invalid();
        this.rawIdentity ??= stable;
        this.release ??= parsed;
        return this.release;
    }
    borrow() {
        this.check();
        if (!this.release || this.token)
            throw invalid();
        const token = {};
        this.token = token;
        const current = () => {
            if (this.stopped || this.token !== token)
                return false;
            if (!this.ownsPhysical()) {
                this.close();
                return false;
            }
            return true;
        };
        const release = this.release;
        const eligibleRtp = (output) => {
            if (!current() || release.floor.carrier !== "webrtc-rtp" || !output || !uint(output.rtpTimestamp) || !positive(output.presentedFrames) || output.presentedFrames <= this.lastRtcPresented)
                return false;
            // SAME physical Track+SSRC owns the production PhysicalVideoFamily
            // and its immutable serialization timeline; no diagnostic timeline ID.
            // Half-range is ambiguous and never silently rebased.
            const fromFloor = (output.rtpTimestamp - release.floor.first_public_rtp_timestamp) >>> 0;
            const fromLast = this.lastRtcTimestamp === undefined ? 1 : (output.rtpTimestamp - this.lastRtcTimestamp) >>> 0;
            return (this.lastRtcTimestamp !== undefined || fromFloor < 0x80000000) && fromLast > 0 && fromLast < 0x80000000;
        };
        const eligibleNative = (output) => {
            if (!current() || release.floor.carrier !== "native-datagram" || !output?.identity || output.identity.connection_generation !== this.scope.connection_generation)
                return false;
            try {
                return nativeMembership(release.floor.cut, output) === "public" && output.datagramSequences.every(sequence => sequence > this.lastNativeSequence);
            }
            catch {
                return false;
            }
        };
        return Object.freeze({
            eligibleRtp, eligibleNative,
            presentedRtp: (output) => {
                if (!eligibleRtp(output))
                    throw invalid();
                this.lastRtcPresented = output.presentedFrames;
                this.lastRtcTimestamp = output.rtpTimestamp;
            },
            presentedNative: (output) => {
                if (!eligibleNative(output))
                    throw invalid();
                this.lastNativeSequence = Math.max(...output.datagramSequences);
            },
            release: () => { if (this.token === token)
                this.token = undefined; },
        });
    }
    close() { this.stopped = true; this.token = undefined; this.release = undefined; }
}
