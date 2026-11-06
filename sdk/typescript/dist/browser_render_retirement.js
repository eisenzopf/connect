import { ConnectError } from "./errors.js";
/** Preserve the exact engine result; foreign/uncertain cleanup is never a
 * substitute for the selected original owner's retirement. Internal SDK hook. */
export function preserveRenderRetirement(connectionId, original) {
    return original.then((receipt) => {
        if (!receipt ||
            receipt.connectionId !== connectionId ||
            receipt.scope !== "renderer-local" ||
            receipt.outputsDetached !== true ||
            !["closed", "not-created"].includes(receipt.audioGraph) ||
            !Object.isFrozen(receipt))
            throw new ConnectError("protocol", "Original renderer retirement receipt mismatch");
        return receipt;
    });
}
