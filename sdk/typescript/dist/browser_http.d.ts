import { Operations } from "./operations.js";
import type { Operation, Outcome } from "./types.js";
/** Browser-safe loopback HTTP client. Credentials remain memory-only. */
export declare class BrowserHttpClient extends Operations {
    private readonly credential;
    private readonly baseUrl;
    private readonly timeoutMs;
    constructor(url: string, credential: string, timeoutMs?: number);
    request(operation: Operation, taskBinding?: string): Promise<Outcome>;
    uploadArtifactChunk(conversationId: string, artifactId: string, requestId: string, offset: number, data: Uint8Array): Promise<Outcome>;
    downloadArtifactChunk(conversationId: string, artifactId: string, offset: number, limit: number): Promise<{
        data: Uint8Array;
        offset: number;
        eof: boolean;
    }>;
    private artifactUrl;
    private fetch;
    private decode;
    private readBounded;
}
