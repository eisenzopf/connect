import { ConnectError } from "./errors.js";

/** One monotonic budget across capture, ICE, signaling and sender configuration. */
export class BrowserDeadline {
    private readonly expires: number;
    constructor(timeoutMs: number) {
        this.expires = performance.now() + timeoutMs;
    }
    remaining(): number {
        const milliseconds = this.expires - performance.now();
        if (milliseconds <= 0)
            throw new ConnectError("timeout", "WebRTC participation timed out");
        return milliseconds;
    }
    async run<T>(operation: () => Promise<T>): Promise<T> {
        const remaining = this.remaining();
        let timer: ReturnType<typeof setTimeout> | undefined;
        try {
            return await Promise.race([
                operation(),
                new Promise<never>((_, reject) => {
                    timer = setTimeout(
                        () =>
                            reject(
                                new ConnectError(
                                    "timeout",
                                    "WebRTC participation timed out",
                                ),
                            ),
                        remaining,
                    );
                }),
            ]);
        } finally {
            clearTimeout(timer);
        }
    }
}
