import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { ConnectError } from "./errors.js";
import { timeoutValue } from "./protocol.js";

const MEDIA_SUBSCRIBE = 0x01;
const MEDIA_AUDIO = 0x02;
const MEDIA_CLOSE = 0x03;
const MEDIA_READY = 0x81;
const MEDIA_RECEIVED = 0x82;
const MEDIA_ERROR = 0xff;
const MAX_MEDIA_FRAME = 4096;

interface MediaWaiter {
    resolve: (payload: Uint8Array) => void;
    reject: (error: ConnectError) => void;
    timer: NodeJS.Timeout;
}

/** Raw PCMU audio over a separate bounded binary helper channel. */
export class NativeAudioClient {
    readonly publisherStreamId: string;
    private buffer: Buffer = Buffer.alloc(0);
    private readonly frames: Uint8Array[] = [];
    private readonly waiters: MediaWaiter[] = [];
    private closed = false;

    private constructor(
        private readonly child: ChildProcessWithoutNullStreams,
        publisherStreamId: string,
        private readonly timeoutMs: number,
    ) {
        this.publisherStreamId = publisherStreamId;
    }

    static open(
        config: string,
        conversationId: string,
        sessionId: string,
        remoteIdentityId: string,
        executable = "connect",
        timeoutMs = 15000,
    ): Promise<NativeAudioClient> {
        timeoutMs = timeoutValue(timeoutMs);
        const child = spawn(
            executable,
            [
                "media-stdio",
                "--config",
                config,
                "--conversation-id",
                conversationId,
                "--session-id",
                sessionId,
                "--remote-identity-id",
                remoteIdentityId,
            ],
            { stdio: ["pipe", "pipe", "pipe"] },
        );
        child.stderr.pipe(process.stderr);
        return new Promise((resolve, reject) => {
            let buffer = Buffer.alloc(0);
            const timer = setTimeout(
                () =>
                    fail(
                        new ConnectError(
                            "transport-uncertain",
                            "Media helper startup timed out",
                        ),
                    ),
                timeoutMs,
            );
            const fail = (error: ConnectError): void => {
                clearTimeout(timer);
                child.stdout.removeAllListeners("data");
                if (child.exitCode === null && child.signalCode === null)
                    child.kill("SIGKILL");
                reject(error);
            };
            child.once("error", () =>
                fail(
                    new ConnectError(
                        "transport-uncertain",
                        "Could not start media helper",
                    ),
                ),
            );
            child.once("exit", () =>
                fail(
                    new ConnectError(
                        "transport-uncertain",
                        "Media helper exited during startup",
                    ),
                ),
            );
            child.stdout.on("data", (chunk: Buffer) => {
                buffer = Buffer.concat([buffer, chunk]);
                if (buffer.length < 5) return;
                const length = buffer.readUInt32BE(1);
                if (length > MAX_MEDIA_FRAME) {
                    fail(
                        new ConnectError(
                            "protocol",
                            "Media helper frame exceeds bounds",
                        ),
                    );
                    return;
                }
                if (buffer.length < 5 + length) return;
                const kind = buffer[0];
                const payload = buffer.subarray(5, 5 + length);
                const remainder = buffer.subarray(5 + length);
                if (kind !== MEDIA_READY || payload.length === 0) {
                    fail(
                        new ConnectError(
                            "protocol",
                            "Media helper did not become ready",
                        ),
                    );
                    return;
                }
                clearTimeout(timer);
                child.stdout.removeAllListeners("data");
                const client = new NativeAudioClient(
                    child,
                    payload.toString("utf8"),
                    timeoutMs,
                );
                client.buffer = Buffer.from(remainder);
                child.stdout.on("data", (data: Buffer) =>
                    client.receiveBytes(data),
                );
                child.once("exit", () => client.fail());
                child.once("error", () => client.fail());
                client.parseFrames();
                resolve(client);
            });
        });
    }

    private receiveBytes(chunk: Buffer): void {
        if (this.closed) return;
        if (this.buffer.length + chunk.length > MAX_MEDIA_FRAME + 5) {
            this.fail(
                new ConnectError(
                    "protocol",
                    "Media helper frame exceeds bounds",
                ),
            );
            return;
        }
        this.buffer = Buffer.concat([this.buffer, chunk]);
        this.parseFrames();
    }

    private parseFrames(): void {
        while (this.buffer.length >= 5) {
            const length = this.buffer.readUInt32BE(1);
            if (length > MAX_MEDIA_FRAME) {
                this.fail(
                    new ConnectError(
                        "protocol",
                        "Media helper frame exceeds bounds",
                    ),
                );
                return;
            }
            if (this.buffer.length < 5 + length) return;
            const kind = this.buffer[0];
            const payload = Buffer.from(this.buffer.subarray(5, 5 + length));
            this.buffer = this.buffer.subarray(5 + length);
            if (kind === MEDIA_ERROR) {
                this.fail(
                    new ConnectError("protocol", payload.toString("utf8")),
                );
                return;
            }
            if (kind !== MEDIA_RECEIVED || payload.length !== 160) {
                this.fail(
                    new ConnectError("protocol", "Invalid received PCMU frame"),
                );
                return;
            }
            const waiter = this.waiters.shift();
            if (waiter) {
                clearTimeout(waiter.timer);
                waiter.resolve(new Uint8Array(payload));
            } else if (this.frames.length < 64)
                this.frames.push(new Uint8Array(payload));
            else {
                this.fail(
                    new ConnectError(
                        "capacity-exceeded",
                        "Media receive queue is full",
                    ),
                );
                return;
            }
        }
    }

    private fail(
        error = new ConnectError("transport-uncertain", "Media helper failed"),
    ): void {
        if (this.closed) return;
        this.closed = true;
        for (const waiter of this.waiters.splice(0)) {
            clearTimeout(waiter.timer);
            waiter.reject(error);
        }
        this.frames.length = 0;
        if (this.child.exitCode === null && this.child.signalCode === null)
            this.child.kill("SIGKILL");
    }

    private write(kind: number, payload = Buffer.alloc(0)): Promise<void> {
        if (this.closed)
            return Promise.reject(
                new ConnectError(
                    "transport-uncertain",
                    "Media helper is closed",
                ),
            );
        if (payload.length > MAX_MEDIA_FRAME)
            return Promise.reject(
                new ConnectError(
                    "invalid-request",
                    "Media frame exceeds bounds",
                ),
            );
        const frame = Buffer.allocUnsafe(5 + payload.length);
        frame[0] = kind;
        frame.writeUInt32BE(payload.length, 1);
        payload.copy(frame, 5);
        return new Promise((resolve, reject) => {
            this.child.stdin.write(frame, (error) => {
                if (error) {
                    const failure = new ConnectError(
                        "transport-uncertain",
                        "Media helper write failed",
                    );
                    this.fail(failure);
                    reject(failure);
                } else resolve();
            });
        });
    }

    subscribe(publisherStreamId: string): Promise<void> {
        const payload = Buffer.from(publisherStreamId, "utf8");
        if (payload.length === 0 || payload.length > 255)
            return Promise.reject(
                new ConnectError(
                    "invalid-request",
                    "Invalid publisher stream identifier",
                ),
            );
        return this.write(MEDIA_SUBSCRIBE, payload);
    }

    sendPcmu(payload: Uint8Array): Promise<void> {
        if (payload.byteLength !== 160)
            return Promise.reject(
                new ConnectError(
                    "invalid-request",
                    "PCMU frames must contain 160 bytes",
                ),
            );
        return this.write(MEDIA_AUDIO, Buffer.from(payload));
    }

    receive(timeoutMs = this.timeoutMs): Promise<Uint8Array> {
        timeoutMs = timeoutValue(timeoutMs);
        const frame = this.frames.shift();
        if (frame) return Promise.resolve(frame);
        if (this.closed)
            return Promise.reject(
                new ConnectError(
                    "transport-uncertain",
                    "Media helper is closed",
                ),
            );
        return new Promise((resolve, reject) => {
            const waiter: MediaWaiter = {
                resolve,
                reject,
                timer: setTimeout(() => {
                    const index = this.waiters.indexOf(waiter);
                    if (index >= 0) this.waiters.splice(index, 1);
                    reject(
                        new ConnectError("timeout", "Media receive timed out"),
                    );
                }, timeoutMs),
            };
            this.waiters.push(waiter);
        });
    }

    async close(): Promise<void> {
        if (this.closed) return;
        await this.write(MEDIA_CLOSE);
        this.closed = true;
        this.child.stdin.end();
        await new Promise<void>((resolve) => {
            const timer = setTimeout(() => {
                this.child.kill("SIGKILL");
                resolve();
            }, 3000);
            this.child.once("exit", () => {
                clearTimeout(timer);
                resolve();
            });
        });
    }
}
