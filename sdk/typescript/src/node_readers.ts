import { NodeSdkError } from "./sdk_helper.js";
import type { CanonicalRequestOptions } from "./canonical_validation.js";
export function readerOptions(options: CanonicalRequestOptions): void {
    if (
        !options ||
        !Number.isSafeInteger(options.timeoutMs) ||
        options.timeoutMs < 1 ||
        options.timeoutMs > 120000 ||
        (options.signal !== undefined &&
            !(options.signal instanceof AbortSignal))
    )
        throw new NodeSdkError("validation", "read-only");
}
export function readerWait<T>(
    promise: Promise<T>,
    expires: number,
    signal: AbortSignal | undefined,
    closed: AbortSignal,
): Promise<T> {
    return new Promise((resolve, reject) => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const cleanup = () => {
            clearTimeout(timer);
            signal?.removeEventListener("abort", cancelled);
            closed.removeEventListener("abort", cancelled);
        };
        const cancelled = () => {
            cleanup();
            reject(new NodeSdkError("cancelled", "read-only"));
        };
        const remaining = expires - performance.now();
        if (signal?.aborted || closed.aborted) {
            cancelled();
            return;
        }
        if (remaining <= 0) {
            reject(new NodeSdkError("deadline", "read-only"));
            return;
        }
        timer = setTimeout(() => {
            cleanup();
            reject(new NodeSdkError("deadline", "read-only"));
        }, remaining);
        signal?.addEventListener("abort", cancelled, { once: true });
        closed.addEventListener("abort", cancelled, { once: true });
        promise.then(
            (value) => {
                cleanup();
                resolve(value);
            },
            (error) => {
                cleanup();
                reject(error);
            },
        );
    });
}
