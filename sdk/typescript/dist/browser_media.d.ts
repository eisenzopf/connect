import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { BrowserOpusCodec, BrowserVp8Codec, RtpClock } from "./browser_codecs.js";
import { RtpSender, Vp8Assembler, type MediaBinding } from "./browser_rtp.js";
import { type Json } from "./uctp_json.js";
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
    audio?: string;
    video?: string;
    screen?: string;
    data?: {
        id: string;
        protocol: BrowserDataProtocol;
    }[];
}
export interface BrowserMediaCallbacks {
    /** Borrowed synchronously; the SDK closes each decoded frame after this callback. */
    audio?: (binding: Readonly<MediaBinding>, frame: AudioData) => void;
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
    clock: RtpClock;
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
    };
}
/** One datagram owner per authenticated physical client; protocol/media state has no UI. */
export declare class BrowserMediaPeer {
    private readonly client;
    private readonly callbacks;
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
        incompleteFrames: number;
        fragments: number;
    }>;
    private check;
    private die;
    private event;
    private serverOffer;
    private reconcile;
    private stable;
    private answer;
    private plan;
    private install;
    private drain;
    private receive;
    offerConnection(input: BrowserConnectionOptions, options?: RequestOptions): Promise<BrowserMediaConnection>;
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
    private retire;
    close(): void;
}
export declare class BrowserMediaConnection {
    private readonly peer;
    readonly id: string;
    constructor(peer: BrowserMediaPeer, id: string);
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
