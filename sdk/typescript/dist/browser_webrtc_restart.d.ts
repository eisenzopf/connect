import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { type JsonObject } from "./uctp_json.js";
import { type Scope } from "./uctp_wire.js";
export interface WebRtcIceRestartReceipt {
    readonly operationId: string;
    readonly iceGeneration: number;
    readonly streamRevision: number;
    readonly connectionGeneration: number;
}
/** Internal same-peer owner. Retries reuse both the gathered offer and the
 * original local description application promises. Status never creates RTC. */
export declare class BrowserIceRestart {
    private readonly client;
    private readonly peer;
    private readonly lifetime;
    private readonly check;
    private readonly fail;
    private readonly original;
    private readonly tasks;
    private current?;
    private running?;
    private generation;
    constructor(client: BrowserClient, peer: RTCPeerConnection, original: Scope, lifetime: AbortSignal, check: () => void, fail: (error: Error) => void);
    get saved(): {
        scope: Scope;
        payload?: JsonObject;
        submissionAttempted: boolean;
    } | undefined;
    restart(operationId: string, revision: number, expected: number, options: RequestOptions): Promise<WebRtcIceRestartReceipt>;
    private prepare;
    private gather;
    private run;
    shutdown(): Promise<void>;
}
