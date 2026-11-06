import { ConnectError } from "./errors.js";
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
export function preserveRenderRetirement(
    connectionId: string,
    original: Promise<BrowserRenderRetirement>,
): Promise<BrowserRenderRetirement> {
    return original.then((receipt) => {
        if (
            !receipt ||
            receipt.connectionId !== connectionId ||
            receipt.scope !== "renderer-local" ||
            receipt.outputsDetached !== true ||
            !["closed", "not-created"].includes(receipt.audioGraph) ||
            !Object.isFrozen(receipt)
        )
            throw new ConnectError(
                "protocol",
                "Original renderer retirement receipt mismatch",
            );
        return receipt;
    });
}
