/** Immutable local renderer cleanup only. It is not media readiness,
 * Connection retirement, browser-process/resource restoration, or playback success. */
export interface BrowserRenderRetirement {
    readonly connectionId: string;
    readonly scope: "renderer-local";
    readonly audioGraph: "closed" | "not-created";
    readonly outputsDetached: true;
}
/** Preserve the exact engine result; foreign/uncertain cleanup is never a
 * substitute for the selected original owner's retirement. Internal SDK hook. */
export declare function preserveRenderRetirement(connectionId: string, original: Promise<BrowserRenderRetirement>): Promise<BrowserRenderRetirement>;
