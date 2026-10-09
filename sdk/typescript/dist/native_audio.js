import { spawn } from "node:child_process";
import { ConnectError } from "./errors.js";
import { timeoutValue } from "./protocol.js";
const MEDIA_SUBSCRIBE = 0x01;
const MEDIA_AUDIO = 0x02;
const MEDIA_CLOSE = 0x03;
const MEDIA_READY = 0x81;
const MEDIA_RECEIVED = 0x82;
const MEDIA_ERROR = 0xff;
const MAX_MEDIA_FRAME = 4096;
/** Raw PCMU audio over a separate bounded binary helper channel. */
export class NativeAudioClient {
    child;
    timeoutMs;
    publisherStreamId;
    buffer = Buffer.alloc(0);
    frames = [];
    waiters = [];
    closed = false;
    constructor(child, publisherStreamId, timeoutMs) {
        this.child = child;
        this.timeoutMs = timeoutMs;
        this.publisherStreamId = publisherStreamId;
    }
    static open(config, conversationId, sessionId, remoteIdentityId, executable = "connect", timeoutMs = 15000) {
        timeoutMs = timeoutValue(timeoutMs);
        const child = spawn(executable, [
            "media-stdio",
            "--config",
            config,
            "--conversation-id",
            conversationId,
            "--session-id",
            sessionId,
            "--remote-identity-id",
            remoteIdentityId,
        ], { stdio: ["pipe", "pipe", "pipe"] });
        child.stderr.pipe(process.stderr);
        return new Promise((resolve, reject) => {
            let buffer = Buffer.alloc(0);
            const timer = setTimeout(() => fail(new ConnectError("transport-uncertain", "Media helper startup timed out")), timeoutMs);
            const fail = (error) => {
                clearTimeout(timer);
                child.stdout.removeAllListeners("data");
                if (child.exitCode === null && child.signalCode === null)
                    child.kill("SIGKILL");
                reject(error);
            };
            child.once("error", () => fail(new ConnectError("transport-uncertain", "Could not start media helper")));
            child.once("exit", () => fail(new ConnectError("transport-uncertain", "Media helper exited during startup")));
            child.stdout.on("data", (chunk) => {
                buffer = Buffer.concat([buffer, chunk]);
                if (buffer.length < 5)
                    return;
                const length = buffer.readUInt32BE(1);
                if (length > MAX_MEDIA_FRAME) {
                    fail(new ConnectError("protocol", "Media helper frame exceeds bounds"));
                    return;
                }
                if (buffer.length < 5 + length)
                    return;
                const kind = buffer[0];
                const payload = buffer.subarray(5, 5 + length);
                const remainder = buffer.subarray(5 + length);
                if (kind !== MEDIA_READY || payload.length === 0) {
                    fail(new ConnectError("protocol", "Media helper did not become ready"));
                    return;
                }
                clearTimeout(timer);
                child.stdout.removeAllListeners("data");
                const client = new NativeAudioClient(child, payload.toString("utf8"), timeoutMs);
                child.stdout.on("data", (data) => client.receiveBytes(data));
                child.once("exit", () => client.fail());
                child.once("error", () => client.fail());
                client.receiveBytes(remainder);
                resolve(client);
            });
        });
    }
    receiveBytes(chunk) {
        // A pipe read can coalesce many valid frames. Bound the incomplete
        // frame buffer, rather than treating the entire read as one frame.
        while (chunk.length && !this.closed) {
            const capacity = MAX_MEDIA_FRAME + 5 - this.buffer.length;
            const size = Math.min(capacity, chunk.length);
            if (size === 0) {
                this.fail(new ConnectError("protocol", "Media helper frame exceeds bounds"));
                return;
            }
            this.buffer = Buffer.concat([this.buffer, chunk.subarray(0, size)]);
            chunk = chunk.subarray(size);
            this.parseFrames();
        }
    }
    parseFrames() {
        while (this.buffer.length >= 5) {
            const length = this.buffer.readUInt32BE(1);
            if (length > MAX_MEDIA_FRAME) {
                this.fail(new ConnectError("protocol", "Media helper frame exceeds bounds"));
                return;
            }
            if (this.buffer.length < 5 + length)
                return;
            const kind = this.buffer[0];
            const payload = Buffer.from(this.buffer.subarray(5, 5 + length));
            this.buffer = this.buffer.subarray(5 + length);
            if (kind === MEDIA_ERROR) {
                this.fail(new ConnectError("protocol", payload.toString("utf8")));
                return;
            }
            if (kind !== MEDIA_RECEIVED || payload.length !== 160) {
                this.fail(new ConnectError("protocol", "Invalid received PCMU frame"));
                return;
            }
            const waiter = this.waiters.shift();
            if (waiter) {
                clearTimeout(waiter.timer);
                waiter.resolve(new Uint8Array(payload));
            }
            else if (this.frames.length < 64)
                this.frames.push(new Uint8Array(payload));
            else {
                this.fail(new ConnectError("capacity-exceeded", "Media receive queue is full"));
                return;
            }
        }
    }
    fail(error = new ConnectError("transport-uncertain", "Media helper failed")) {
        if (this.closed)
            return;
        this.closed = true;
        for (const waiter of this.waiters.splice(0)) {
            clearTimeout(waiter.timer);
            waiter.reject(error);
        }
        this.frames.length = 0;
        if (this.child.exitCode === null && this.child.signalCode === null)
            this.child.kill("SIGKILL");
    }
    write(kind, payload = Buffer.alloc(0)) {
        if (this.closed)
            return Promise.reject(new ConnectError("transport-uncertain", "Media helper is closed"));
        if (payload.length > MAX_MEDIA_FRAME)
            return Promise.reject(new ConnectError("invalid-request", "Media frame exceeds bounds"));
        const frame = Buffer.allocUnsafe(5 + payload.length);
        frame[0] = kind;
        frame.writeUInt32BE(payload.length, 1);
        payload.copy(frame, 5);
        return new Promise((resolve, reject) => {
            this.child.stdin.write(frame, (error) => {
                if (error) {
                    const failure = new ConnectError("transport-uncertain", "Media helper write failed");
                    this.fail(failure);
                    reject(failure);
                }
                else
                    resolve();
            });
        });
    }
    subscribe(publisherStreamId) {
        const payload = Buffer.from(publisherStreamId, "utf8");
        if (payload.length === 0 || payload.length > 255)
            return Promise.reject(new ConnectError("invalid-request", "Invalid publisher stream identifier"));
        return this.write(MEDIA_SUBSCRIBE, payload);
    }
    sendPcmu(payload) {
        if (payload.byteLength !== 160)
            return Promise.reject(new ConnectError("invalid-request", "PCMU frames must contain 160 bytes"));
        return this.write(MEDIA_AUDIO, Buffer.from(payload));
    }
    receive(timeoutMs = this.timeoutMs) {
        timeoutMs = timeoutValue(timeoutMs);
        const frame = this.frames.shift();
        if (frame)
            return Promise.resolve(frame);
        if (this.closed)
            return Promise.reject(new ConnectError("transport-uncertain", "Media helper is closed"));
        return new Promise((resolve, reject) => {
            const waiter = {
                resolve,
                reject,
                timer: setTimeout(() => {
                    const index = this.waiters.indexOf(waiter);
                    if (index >= 0)
                        this.waiters.splice(index, 1);
                    reject(new ConnectError("timeout", "Media receive timed out"));
                }, timeoutMs),
            };
            this.waiters.push(waiter);
        });
    }
    async close() {
        if (this.closed)
            return;
        await this.write(MEDIA_CLOSE);
        this.closed = true;
        this.child.stdin.end();
        await new Promise((resolve) => {
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
