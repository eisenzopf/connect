import { ConnectError } from "./errors.js";
/** One monotonic budget across capture, ICE, signaling and sender configuration. */
export class BrowserDeadline {
    expires;
    constructor(timeoutMs) {
        this.expires = performance.now() + timeoutMs;
    }
    remaining() {
        const milliseconds = this.expires - performance.now();
        if (milliseconds <= 0)
            throw new ConnectError("timeout", "WebRTC participation timed out");
        return milliseconds;
    }
    async run(operation) {
        const remaining = this.remaining();
        let timer;
        try {
            return await Promise.race([
                operation(),
                new Promise((_, reject) => {
                    timer = setTimeout(() => reject(new ConnectError("timeout", "WebRTC participation timed out")), remaining);
                }),
            ]);
        }
        finally {
            clearTimeout(timer);
        }
    }
}
