/** Typed packet/data handles on the original NodeClient's separate IPC owner. */
import { SdkHelper, NodeSdkError } from "./sdk_helper.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
import { duration } from "./canonical_validation.js";
import type { OperationKey } from "./canonical_error.js";
import {
    canonicalJson,
    parseCanonicalJson,
    type Json,
    type JsonObject,
} from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
import type {
    ConversationId,
    ParticipantId,
    MessageId,
    OperationId,
    SdkMutationOptions,
    SdkOperation,
    SdkRevision,
} from "./node_sdk.js";
import type { SessionId, SdkNonEmpty, SdkMedium } from "./node_sessions.js";
import type {
    ConnectionId,
    StreamId,
    SdkDirection,
} from "./node_connections.js";
import {
    MediaChannel,
    openMediaChannel,
    mediaError,
    mediaDuration,
    encodeFrame,
    exact,
    localWait,
    owned,
    MEDIA_PROFILE,
    MEDIA_METADATA,
    type MediaFrame,
} from "./node_media_channel.js";
import {
    SdkRtpPacket,
    capturedRtp,
    clockWire,
    bindingWire,
    captureBinding,
    decimal,
    uint,
    capturedData,
    receivedData,
    mediaFrozen,
    mediaJson,
    type SdkMediaBinding,
    type SdkMediaNegotiation,
    type SdkMediaPacket,
    type SdkSenderClockReport,
    type SdkLocalMediaSubmission,
    type SdkMediaStats,
    type SdkDataPayload,
    type SdkReceivedData,
    type SdkDataAcceptance,
    type SdkDataStats,
} from "./node_media_types.js";

export interface SdkMediaStream {
    readonly binding: SdkMediaBinding;
    readonly id: StreamId;
    sendRtp(
        packet: SdkRtpPacket,
        options: CanonicalRequestOptions,
    ): Promise<SdkLocalMediaSubmission>;
    sendClock(
        sequence: number,
        report: SdkSenderClockReport,
        options: CanonicalRequestOptions,
    ): Promise<SdkLocalMediaSubmission>;
    receivePacket(options: CanonicalRequestOptions): Promise<SdkMediaPacket>;
    sendData(
        input: {
            readonly messageId: MessageId;
            readonly to: SdkNonEmpty<ParticipantId>;
            readonly payload: SdkDataPayload;
        },
        options: SdkMutationOptions,
    ): Promise<SdkOperation<SdkDataAcceptance>>;
    receiveData(options: CanonicalRequestOptions): Promise<SdkReceivedData>;
}
export interface SdkMediaConnection {
    readonly binding: SdkMediaBinding;
    refresh(options: CanonicalRequestOptions): Promise<SdkMediaNegotiation>;
    streamById(id: StreamId): SdkMediaStream;
}
export interface SdkMedia {
    connection(binding: SdkMediaBinding): SdkMediaConnection;
    stats(options: CanonicalRequestOptions): Promise<SdkMediaStats>;
    dataStats(options: CanonicalRequestOptions): Promise<SdkDataStats>;
    close(options: CanonicalRequestOptions): Promise<void>;
}
interface Context {
    restore(value: Json, key: OperationKey): SdkOperation<SdkDataAcceptance>;
    observe(value: Json, key: OperationKey): void;
}
function sameBinding(a: Json, b: SdkMediaBinding): boolean {
    return canonicalJson(a) === canonicalJson(bindingWire(b));
}
function ordinary(frame: MediaFrame): void {
    if (frame.kind !== 0x81 || frame.payload.length) throw new Error();
}
function negotiation(
    binding: SdkMediaBinding,
    frame: MediaFrame,
): SdkMediaNegotiation {
    ordinary(frame);
    const v = frame.metadata;
    if (
        !exact(v, [
            "type",
            "binding",
            "source_clock",
            "stream_revision",
            "streams",
        ]) ||
        v.type !== "negotiation" ||
        !sameBinding(v.binding, binding) ||
        typeof v.source_clock !== "boolean" ||
        !Number.isSafeInteger(v.stream_revision) ||
        Number(v.stream_revision) < 1 ||
        !Array.isArray(v.streams) ||
        v.streams.length > 128
    )
        throw new Error();
    const streams = v.streams.map((s) => {
        if (
            !exact(s, [
                "id",
                "medium",
                "direction",
                "codec",
                "stream_local_id",
            ]) ||
            !identifier(s.id) ||
            !["audio", "video", "screen", "data"].includes(
                s.medium as string,
            ) ||
            !["sendonly", "recvonly", "sendrecv", "inactive"].includes(
                s.direction as string,
            ) ||
            (s.stream_local_id !== null &&
                (!Number.isSafeInteger(s.stream_local_id) ||
                    Number(s.stream_local_id) < 1 ||
                    Number(s.stream_local_id) > 65535))
        )
            throw new Error();
        return mediaFrozen({
            id: s.id as StreamId,
            medium: s.medium as SdkMedium,
            direction: s.direction as SdkDirection,
            codec: s.codec,
            ...(s.stream_local_id === null
                ? {}
                : { streamLocalId: s.stream_local_id as number }),
        });
    });
    if (new Set(streams.map((s) => s.id)).size !== streams.length)
        throw new Error();
    return mediaFrozen({
        binding,
        sourceClock: v.source_clock,
        streamRevision: v.stream_revision as SdkRevision<"stream">,
        streams,
    });
}
function submitted(
    binding: SdkMediaBinding,
    id: StreamId,
    sequence: number,
    f: MediaFrame,
): SdkLocalMediaSubmission {
    ordinary(f);
    const v = f.metadata;
    if (
        !exact(v, [
            "type",
            "binding",
            "stream_id",
            "boundary",
            "datagram_sequence",
        ]) ||
        v.type !== "submitted" ||
        !sameBinding(v.binding, binding) ||
        v.stream_id !== id ||
        v.boundary !== "local-media-queue" ||
        v.datagram_sequence !== sequence
    )
        throw new Error();
    return Object.freeze({
        boundary: "local-media-queue",
        datagramSequence: sequence,
    });
}
function packet(
    binding: SdkMediaBinding,
    id: StreamId,
    f: MediaFrame,
): SdkMediaPacket {
    const v = f.metadata;
    if (
        f.kind !== 0x82 ||
        !exact(v, ["type", "binding", "stream_id", "packet"]) ||
        v.type !== "packet" ||
        !sameBinding(v.binding, binding) ||
        v.stream_id !== id
    )
        throw new Error();
    const p = v.packet;
    if (
        p !== null &&
        typeof p === "object" &&
        !Array.isArray(p) &&
        p.type === "rtp"
    ) {
        if (!exact(p, ["type", "datagram_sequence", "header"]))
            throw new Error();
        const result = new SdkRtpPacket(
                p.datagram_sequence as number,
                f.payload,
            ),
            h = result.header;
        if (
            !exact(p.header, [
                "payload_type",
                "sequence_number",
                "timestamp",
                "ssrc",
                "marker",
            ]) ||
            canonicalJson(p.header) !==
                canonicalJson({
                    payload_type: h.payloadType,
                    sequence_number: h.sequenceNumber,
                    timestamp: h.timestamp,
                    ssrc: h.ssrc,
                    marker: h.marker,
                })
        )
            throw new Error();
        return result;
    }
    if (
        f.payload.length ||
        p === null ||
        typeof p !== "object" ||
        Array.isArray(p)
    )
        throw new Error();
    if (p.type === "gap" && exact(p, ["type", "dropped_packets"]))
        return Object.freeze({
            kind: "gap",
            droppedPackets: decimal(p.dropped_packets),
        });
    if (
        p.type !== "clock" ||
        !exact(p, ["type", "datagram_sequence", "report"]) ||
        !exact(p.report, ["ssrc", "ntp", "rtp", "packets", "octets", "cname"])
    )
        throw new Error();
    const r = p.report,
        report = {
            ssrc: r.ssrc as number,
            ntp: decimal(r.ntp),
            rtp: r.rtp as number,
            packets: r.packets as number,
            octets: r.octets as number,
            cname: r.cname as string,
        };
    clockWire(report);
    return mediaFrozen({
        kind: "clock",
        datagramSequence: uint(p.datagram_sequence),
        report,
    });
}
function dataPacket(
    binding: SdkMediaBinding,
    id: StreamId,
    f: MediaFrame,
): SdkReceivedData {
    const v = f.metadata;
    if (
        f.kind !== 0x82 ||
        !exact(v, [
            "type",
            "binding",
            "stream_id",
            "message_id",
            "operation_id",
            "from",
            "format",
        ]) ||
        v.type !== "data" ||
        !sameBinding(v.binding, binding) ||
        v.stream_id !== id ||
        !identifier(v.message_id) ||
        !identifier(v.operation_id) ||
        !identifier(v.from)
    )
        throw new Error();
    // Do not recursively inspect/freeze private byte storage or getters.
    return Object.freeze({
        messageId: v.message_id as MessageId,
        operationId: v.operation_id as OperationId,
        fromParticipant: v.from as ParticipantId,
        payload: receivedData(v.format, f.payload),
    });
}
const mediaCounters = {
    received_datagrams: "receivedDatagrams",
    rtp_packets: "rtpPackets",
    clock_reports: "clockReports",
    malformed_datagrams: "malformedDatagrams",
    unknown_handles: "unknownHandles",
    overflow_drops: "overflowDrops",
    control_drops: "controlDrops",
    binding_change_drops: "bindingChangeDrops",
};
const dataCounters = {
    received_frames: "receivedFrames",
    accepted_messages: "acceptedMessages",
    malformed_frames: "malformedFrames",
    unowned_frames: "unownedFrames",
    overflow_drops: "overflowDrops",
    binding_change_drops: "bindingChangeDrops",
};
function stats(f: MediaFrame, data: boolean): SdkMediaStats | SdkDataStats {
    ordinary(f);
    const v = f.metadata,
        map = data ? dataCounters : mediaCounters,
        packets = data ? "queued_messages" : "queued_packets";
    if (
        !exact(v, ["type", ...Object.keys(map), packets, "queued_bytes"]) ||
        v.type !== (data ? "data_stats" : "stats") ||
        !Number.isSafeInteger(v[packets]) ||
        Number(v[packets]) < 0 ||
        Number(v[packets]) > 256 ||
        !Number.isSafeInteger(v.queued_bytes) ||
        Number(v.queued_bytes) < 0 ||
        Number(v.queued_bytes) > (data ? 1 : 4) * 1024 * 1024
    )
        throw new Error();
    return Object.freeze({
        ...Object.fromEntries(
            Object.entries(map).map(([wire, name]) => [name, decimal(v[wire])]),
        ),
        [data ? "queuedMessages" : "queuedPackets"]: v[packets],
        queuedBytes: v.queued_bytes,
    }) as unknown as SdkMediaStats | SdkDataStats;
}
interface Slot {
    busy: boolean;
    data: boolean;
    flight?: Promise<unknown>;
}
class Owner {
    private opening?: Promise<MediaChannel>;
    private detaching?: Promise<void>;
    private closed = false;
    private readonly slots = new Map<string, Slot>();
    constructor(
        private readonly helper: SdkHelper,
        private readonly ctx: Context,
    ) {}
    async request<T>(
        command: JsonObject,
        payload: Buffer,
        options: CanonicalRequestOptions,
        decode: (f: MediaFrame) => T,
        mutation = false,
        key?: OperationKey,
    ): Promise<T> {
        const started = performance.now();
        let captured: JsonObject;
        try {
            options = {
                timeoutMs: options?.timeoutMs,
                signal: options?.signal,
            };
            mediaDuration(options);
            captured = parseCanonicalJson(
                mediaJson(command, MEDIA_METADATA),
                MEDIA_METADATA,
            ) as JsonObject;
            encodeFrame(
                1,
                2,
                { timeout_ms: options.timeoutMs, command: captured },
                payload,
            );
        } catch {
            throw mediaError("validation", mutation, false, key);
        }
        if (options.signal?.aborted)
            throw mediaError("cancelled", mutation, false, key);
        if (this.closed)
            throw mediaError("configuration", mutation, false, key);
        if (!this.opening)
            this.opening = owned(
                openMediaChannel(this.helper, this.ctx.observe),
            );
        let channel: MediaChannel;
        try {
            channel = await localWait(this.opening, options);
        } catch (e) {
            if (e instanceof NodeSdkError)
                throw new NodeSdkError(
                    e.kind,
                    mutation ? "not-submitted" : "read-only",
                    key,
                    e.problem,
                );
            throw mediaError("configuration", mutation, false, key);
        }
        const remaining = Math.floor(
            options.timeoutMs - (performance.now() - started),
        );
        if (remaining <= 0) throw mediaError("deadline", mutation, false, key);
        const frame = await channel.exchange(
            captured,
            payload,
            { timeoutMs: remaining, signal: options.signal },
            mutation,
            key,
        );
        try {
            return decode(frame);
        } catch {
            channel.fence("protocol");
            throw mediaError("protocol", mutation, mutation, key);
        }
    }
    async receive<T>(
        binding: SdkMediaBinding,
        id: StreamId,
        options: CanonicalRequestOptions,
        data: boolean,
        decode: (f: MediaFrame) => T,
    ): Promise<T> {
        options = { timeoutMs: options?.timeoutMs, signal: options?.signal };
        mediaDuration(options);
        if (this.closed) throw mediaError("configuration");
        if (options.signal?.aborted) throw mediaError("cancelled");
        const key = JSON.stringify([
            data,
            binding.conversationId,
            binding.sessionId,
            binding.connectionId,
            id,
        ]);
        let slot = this.slots.get(key);
        if (!slot) {
            if (
                this.slots.size >= 256 ||
                (data &&
                    [...this.slots.values()].filter((s) => s.data).length >= 16)
            )
                throw mediaError("application");
            slot = { busy: false, data };
            this.slots.set(key, slot);
        }
        if (slot.busy) throw mediaError("conflict");
        slot.busy = true;
        const original = slot;
        const forget = () => {
            if (
                !original.busy &&
                !original.flight &&
                this.slots.get(key) === original
            )
                this.slots.delete(key);
        };
        try {
            if (!slot.flight) {
                const flight = owned(
                    this.request(
                        {
                            action: data ? "receive_data" : "receive",
                            binding: bindingWire(binding),
                            stream_id: id,
                        },
                        Buffer.alloc(0),
                        { timeoutMs: options.timeoutMs },
                        decode,
                    ),
                );
                slot.flight = flight;
                void flight.catch(() => {
                    if (original.flight === flight) {
                        original.flight = undefined;
                        forget();
                    }
                });
            }
            const flight = slot.flight as Promise<T>,
                result = await localWait(flight, options);
            if (slot.flight === flight) slot.flight = undefined;
            return result;
        } finally {
            slot.busy = false;
            forget();
        }
    }
    async close(options: CanonicalRequestOptions): Promise<void> {
        options = { timeoutMs: options?.timeoutMs, signal: options?.signal };
        duration(options);
        if (
            options.signal !== undefined &&
            !(options.signal instanceof AbortSignal)
        )
            throw mediaError("validation");
        this.closed = true;
        if (!this.opening) return;
        if (!this.detaching)
            this.detaching = owned(
                (async () => {
                    let channel: MediaChannel;
                    try {
                        channel = await this.opening!;
                    } catch {
                        return;
                    }
                    channel.markClosing();
                    try {
                        if (!this.helper.isClosed)
                            await this.helper.request(
                                { type: "media_detach" },
                                { timeoutMs: 4000 },
                                (v) => {
                                    if (
                                        !exact(v, ["profile", "closed"]) ||
                                        v.profile !== MEDIA_PROFILE ||
                                        v.closed !== true
                                    )
                                        throw new Error();
                                },
                            );
                    } finally {
                        await channel.close();
                        await Promise.allSettled(
                            [...this.slots.values()].flatMap((s) =>
                                s.flight ? [s.flight] : [],
                            ),
                        );
                        this.slots.clear();
                    }
                })(),
            );
        await localWait(
            this.detaching,
            options,
            false,
            () => false,
            undefined,
            true,
        );
    }
    stream(binding: SdkMediaBinding, id: StreamId): SdkMediaStream {
        if (!identifier(id)) throw mediaError("validation", true);
        return Object.freeze({
            binding,
            id,
            sendRtp: (p: SdkRtpPacket, o: CanonicalRequestOptions) =>
                this.request(
                    {
                        action: "send_rtp",
                        binding: bindingWire(binding),
                        stream_id: id,
                        datagram_sequence: p?.datagramSequence,
                    },
                    capturedRtp(p),
                    o,
                    (f) => submitted(binding, id, p.datagramSequence, f),
                    true,
                ),
            sendClock: (
                sequence: number,
                report: SdkSenderClockReport,
                o: CanonicalRequestOptions,
            ) =>
                this.request(
                    {
                        action: "send_clock",
                        binding: bindingWire(binding),
                        stream_id: id,
                        datagram_sequence: uint(sequence),
                        report: clockWire(report),
                    },
                    Buffer.alloc(0),
                    o,
                    (f) => submitted(binding, id, sequence, f),
                    true,
                ),
            receivePacket: (o: CanonicalRequestOptions) =>
                this.receive(binding, id, o, false, (f) =>
                    packet(binding, id, f),
                ),
            sendData: (
                input: {
                    readonly messageId: MessageId;
                    readonly to: SdkNonEmpty<ParticipantId>;
                    readonly payload: SdkDataPayload;
                },
                o: SdkMutationOptions,
            ) => {
                const operationId = o?.operationId;
                if (!o || !identifier(operationId))
                    throw mediaError("validation", true);
                const key: OperationKey = mediaFrozen({
                    operationId,
                    command: "message.send",
                    scope: {
                        conversationId: binding.conversationId,
                        sessionId: binding.sessionId,
                        connectionId: binding.connectionId,
                    },
                });
                let payload: Buffer, to: ParticipantId[];
                const messageId = input?.messageId,
                    audience = input?.to,
                    rest = audience?.rest;
                try {
                    if (
                        !input ||
                        !identifier(messageId) ||
                        !audience ||
                        !Array.isArray(rest)
                    )
                        throw new Error();
                    to = [audience.first, ...rest];
                    if (
                        !to.length ||
                        to.length > 256 ||
                        to.some((v) => !identifier(v)) ||
                        new Set(to).size !== to.length
                    )
                        throw new Error();
                    payload = capturedData(input.payload);
                } catch {
                    throw mediaError("validation", true, false, key);
                }
                return this.request(
                    {
                        action: "send_data",
                        binding: bindingWire(binding),
                        stream_id: id,
                        message_id: messageId,
                        to,
                        format: input.payload.kind,
                        mutation: {
                            operation_id: key.operationId,
                            ...(o.extensions === undefined
                                ? {}
                                : { extensions: o.extensions as JsonObject }),
                        },
                    },
                    payload,
                    o,
                    (f) => {
                        ordinary(f);
                        const v = f.metadata;
                        if (
                            !exact(v, [
                                "type",
                                "binding",
                                "stream_id",
                                "reference",
                            ]) ||
                            v.type !== "data_submitted" ||
                            !sameBinding(v.binding, binding) ||
                            v.stream_id !== id
                        )
                            throw new Error();
                        return this.ctx.restore(v.reference, key);
                    },
                    true,
                    key,
                );
            },
            receiveData: (o: CanonicalRequestOptions) =>
                this.receive(binding, id, o, true, (f) =>
                    dataPacket(binding, id, f),
                ),
        });
    }
}
export function createMedia(helper: SdkHelper, ctx: Context): SdkMedia {
    const owner = new Owner(helper, ctx);
    return Object.freeze({
        connection: (input: SdkMediaBinding): SdkMediaConnection => {
            const binding = captureBinding(input);
            return Object.freeze({
                binding,
                refresh: (o: CanonicalRequestOptions) =>
                    owner.request(
                        { action: "describe", binding: bindingWire(binding) },
                        Buffer.alloc(0),
                        o,
                        (f) => negotiation(binding, f),
                    ),
                streamById: (id: StreamId) => owner.stream(binding, id),
            });
        },
        stats: (o: CanonicalRequestOptions) =>
            owner.request(
                { action: "stats" },
                Buffer.alloc(0),
                o,
                (f) => stats(f, false) as SdkMediaStats,
            ),
        dataStats: (o: CanonicalRequestOptions) =>
            owner.request(
                { action: "data_stats" },
                Buffer.alloc(0),
                o,
                (f) => stats(f, true) as SdkDataStats,
            ),
        close: (o: CanonicalRequestOptions) => owner.close(o),
    });
}
