import { ConnectError } from "./errors.js";
/** One monotonic budget across capture, ICE, signaling and sender configuration. */
export class BrowserDeadline {
    expires;
    constructor(timeoutMs) {
        this.expires = performance.now() + timeoutMs;
    }
    remaining() {
        const milliseconds = this.expires - performance.now();
        if (milliseconds < 1)
            throw new ConnectError("timeout", "WebRTC participation timed out");
        return milliseconds;
    }
    async run(operation, signal) {
        const remaining = this.remaining();
        if (signal?.aborted)
            throw new ConnectError("cancelled", "Browser wait cancelled");
        let timer;
        let abort;
        try {
            return await Promise.race([
                operation(),
                new Promise((_, reject) => {
                    abort = () => reject(new ConnectError("cancelled", "Browser wait cancelled"));
                    signal?.addEventListener("abort", abort, { once: true });
                    if (signal?.aborted)
                        abort();
                    timer = setTimeout(() => reject(new ConnectError("timeout", "WebRTC participation timed out")), remaining);
                }),
            ]);
        }
        finally {
            clearTimeout(timer);
            if (abort)
                signal?.removeEventListener("abort", abort);
        }
    }
}
