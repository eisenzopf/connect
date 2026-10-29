/** One owned authenticated C6M2 channel. Application waits never own its IO. */
import { type Socket } from "node:net";
import { SdkHelper, NodeSdkError } from "./sdk_helper.js";
import { type Json, type JsonObject } from "./uctp_json.js";
import { type CanonicalRequestOptions } from "./canonical_validation.js";
import type { CanonicalErrorKind, OperationKey } from "./canonical_error.js";
export declare const MEDIA_PROFILE = "connect-sdk-media/2";
export declare const MEDIA_METADATA = 16384, MEDIA_PAYLOAD = 65536, MEDIA_WINDOW = 16;
export interface MediaFrame {
    kind: number;
    id: number;
    metadata: JsonObject;
    payload: Buffer;
}
export declare function mediaError(kind: CanonicalErrorKind, mutation?: boolean, sent?: boolean, key?: OperationKey): NodeSdkError;
export declare function exact(v: unknown, keys: readonly string[]): v is JsonObject;
export declare function mediaDuration(o: CanonicalRequestOptions): void;
export declare function encodeFrame(kind: number, id: number, metadata: JsonObject, payload?: Buffer): Buffer;
/** Header bounds precede allocation. Partial bytes belong to this reader. */
export declare class MediaFrameReader {
    onFrame: (frame: MediaFrame) => void;
    onFailure: (kind: CanonicalErrorKind) => void;
    private readonly header;
    private used;
    private body?;
    private filled;
    private metadataBytes;
    private failed;
    constructor(onFrame: (frame: MediaFrame) => void, onFailure: (kind: CanonicalErrorKind) => void);
    push(chunk: Buffer): void;
    end(): void;
    fence(kind: CanonicalErrorKind): void;
}
export declare function owned<T>(p: Promise<T>): Promise<T>;
export declare function localWait<T>(promise: Promise<T>, options: CanonicalRequestOptions, mutation?: boolean, sent?: () => boolean, key?: OperationKey, closing?: boolean): Promise<T>;
interface SocketOwner {
    socket: Socket;
    closed: Promise<void>;
    reader: MediaFrameReader;
}
export declare class MediaChannel {
    private readonly owner;
    private readonly listenerClosed;
    private readonly observe;
    private sequence;
    private closed;
    private closing;
    private readonly pending;
    private readonly queue;
    private writer?;
    constructor(owner: SocketOwner, listenerClosed: Promise<void>, observe: (v: Json, key: OperationKey) => void);
    get pendingCount(): number;
    exchange(command: JsonObject, payload: Buffer, options: CanonicalRequestOptions, mutation?: boolean, key?: OperationKey): Promise<MediaFrame>;
    private pump;
    private receive;
    markClosing(): void;
    fence(kind: CanonicalErrorKind): void;
    close(): Promise<void>;
}
/** Attachment owns the listener/accepted sockets independently of caller waits. */
export declare function openMediaChannel(helper: SdkHelper, observe: (v: Json, key: OperationKey) => void): Promise<MediaChannel>;
export {};
