import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { BrowserOpusCodec, BrowserVp8Codec } from "./browser_codecs.js";
import { RtpSender, Vp8Assembler, type MediaBinding } from "./browser_rtp.js";
import { RtcpCaptureClock, RtcpReceiveClock } from "./browser_rtcp.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import { type Envelope } from "./uctp_wire.js";
export type BrowserDataProtocol = "text" | "json" | "binary";
export type BrowserDataBody = {
    type: "text";
    value: string;
} | {
    type: "json";
    value: Json;
} | {
    type: "binary";
    value: Uint8Array;
};
export interface BrowserConnectionOptions {
    conversationId: string;
    sessionId: string;
    connectionId: string;
    operationId: string;
    /** Target-owned audio stays isolated until an attributed handoff commits. */
    purpose?: "handoff-preparation";
    audio?: string;
    video?: string;
    screen?: string;
    data?: {
        id: string;
        protocol: BrowserDataProtocol;
    }[];
}
export interface BrowserMediaCallbacks {
    /** Borrowed synchronously; timestamps use the local performance.now() timeline.
     * The SDK closes each decoded frame after this callback. */
    audio?: (binding: Readonly<MediaBinding>, frame: AudioData) => void;
    /** Same local receive timeline and borrowed lifetime as decoded audio. */
    video?: (binding: Readonly<MediaBinding>, frame: VideoFrame) => void;
    data?: (message: {
        conversationId: string;
        sessionId: string;
        connectionId: string;
        streamId: string;
        operationId: string;
        messageId: string;
        from: string;
        body: BrowserDataBody;
    }) => void;
    error?: (error: Error) => void;
}
interface Route {
    binding: Readonly<MediaBinding>;
    sender?: RtpSender;
    opus?: BrowserOpusCodec;
    vp8?: BrowserVp8Codec;
    clock: RtcpReceiveClock;
    captureClock: RtcpCaptureClock;
    lastReport: number;
    clockProfile: boolean;
    assembler?: Vp8Assembler;
    observed: {
        encoded: number;
        keys: number;
        received: number;
        complete: number;
        submitted: number;
        skipped: number;
        decoded: number;
        expired: number;
        clockReports: number;
        synchronized: number;
        unsynchronized: number;
    };
}
/** One datagram owner per authenticated physical client; protocol/media state has no UI. */
export declare class BrowserMediaPeer {
    private readonly client;
    private readonly callbacks;
    private readonly tasks;
    private readonly reader;
    private readonly writer;
    private readonly registry;
    private readonly routes;
    private readonly connections;
    private readonly transitions;
    private readonly queue;
    private readonly stopEvents;
    private readonly stopLifetime;
    private readonly timer;
    private stopped;
    private sending;
    private readonly mtu;
    private counters;
    constructor(client: BrowserClient, callbacks?: BrowserMediaCallbacks);
    get diagnostics(): Readonly<typeof this.counters & {
        queuedPackets: number;
        queuedBytes: number;
        droppedPackets: number;
        activeBindings: number;
    }>;
    get routeDiagnostics(): ReadonlyArray<{
        streamId: string;
        direction: MediaBinding["direction"];
        kind: MediaBinding["kind"];
        counters: Readonly<Route["observed"]>;
        clockProfile: boolean;
        incompleteFrames: number;
        fragments: number;
        recovery?: BrowserVp8Codec["decodeDiagnostics"];
    }>;
    private check;
    private die;
    private event;
    private serverOffer;
    private reconcile;
    private stable;
    private answer;
    private plan;
    private requireState;
    private install;
    private drain;
    private receive;
    /** Start one original offer. `accepted` means validated current physical
     * parameters arrived; `ready` separately observes installation. Neither
     * promise can retry the mutation or lend ownership after peer retirement. */
    beginOfferConnection(input: BrowserConnectionOptions, options?: RequestOptions): {
        accepted: Promise<void>;
        ready: Promise<BrowserMediaConnection>;
        readonly submissionAttempted: boolean;
    };
    offerConnection(input: BrowserConnectionOptions, options?: RequestOptions, accepted?: () => void, attempted?: () => void): Promise<BrowserMediaConnection>;
    /** Start one original native control while retaining its counteroffer owner. */
    beginControl(id: string, operationId: string, type: "stream.subscribe" | "stream.unsubscribe" | "connection.update", payload: JsonObject, expectedRevision: number, options: RequestOptions): {
        response: Promise<Envelope[]>;
        ready: Promise<void>;
        readonly submissionAttempted: boolean;
    };
    private transition;
    private completed;
    private state;
    streams(id: string): Readonly<MediaBinding>[];
    revision(id: string): number;
    onConnectionClose(id: string, callback: () => void): () => void;
    subscribe(id: string, operationId: string, participantId: string, kinds: ("audio" | "video" | "screen" | "data")[], options: RequestOptions): Promise<void>;
    encodeAudio(id: string, streamId: string, samples: Float32Array, timestampUs: number): boolean;
    encodeVideo(id: string, streamId: string, frame: VideoFrame): boolean;
    sendData(id: string, operationId: string, messageId: string, streamId: string, body: BrowserDataBody, options: RequestOptions): Promise<void>;
    private dataReceived;
    end(id: string, operationId: string, options: RequestOptions): Promise<void>;
    private remove;
    /** Retire local owners after observing an authorized durable end receipt. */
    retireLocalConnection(id: string): void;
    private retire;
    close(): void;
    ownCleanup(work: Promise<unknown>): void;
    shutdown(): Promise<void>;
}
export declare class BrowserMediaConnection {
    private readonly peer;
    readonly id: string;
    constructor(peer: BrowserMediaPeer, id: string);
    ownCleanup(work: Promise<unknown>): void;
    get streams(): Readonly<MediaBinding>[];
    get revision(): number;
    onClose(callback: () => void): () => void;
    subscribe(operationId: string, participantId: string, kinds: ("audio" | "video" | "screen" | "data")[], options?: RequestOptions): Promise<void>;
    encodeAudio(streamId: string, samples: Float32Array, timestampUs: number): boolean;
    encodeVideo(streamId: string, frame: VideoFrame): boolean;
    sendData(operationId: string, messageId: string, streamId: string, body: BrowserDataBody, options?: RequestOptions): Promise<void>;
    end(operationId: string, options?: RequestOptions): Promise<void>;
}
export {};
