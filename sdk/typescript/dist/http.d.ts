import { type HttpControlTransport } from "./control_endpoint.js";
import { Operations } from "./operations.js";
import type { Operation, Outcome } from "./types.js";
/** Explicit loopback HTTP or selected remote HTTPS. No redirects or automatic mutation retries. */
export declare class HttpClient extends Operations {
    private readonly credential;
    private readonly url;
    private readonly baseUrl;
    private readonly timeoutMs;
    constructor(url: string, credential: string, timeoutMs?: number, transport?: HttpControlTransport);
    request(operation: Operation, taskBinding?: string): Promise<Outcome>;
    /** Upload one raw artifact chunk without base64 expansion. */
    uploadArtifactChunk(conversationId: string, artifactId: string, requestId: string, offset: number, data: Uint8Array): Promise<Outcome>;
    /** Download one raw artifact chunk with validated framing metadata. */
    downloadArtifactChunk(conversationId: string, artifactId: string, offset: number, limit: number): Promise<{
        data: Uint8Array;
        offset: number;
        eof: boolean;
    }>;
    private artifactFetch;
    private readBounded;
}
