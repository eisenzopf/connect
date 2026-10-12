import { exchangeTimeout } from "./task_operations.js";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { ConnectError } from "./errors.js";
import { Operations } from "./operations.js";
import { expectedKind, helperResponse, MAX_REQUEST, MAX_RESPONSE, timeoutValue, } from "./protocol.js";
/** Rust owns TLS/auth/UCTP/QUIC; helper failure closes this client. */
export class NativeClient extends Operations {
    child;
    exited;
    pending;
    buffer = Buffer.alloc(0);
    closed = false;
    chain = Promise.resolve();
    timeoutMs;
    constructor(config, executable = "connect", timeoutMs = 15000) {
        super();
        this.timeoutMs = timeoutValue(timeoutMs);
        this.child = spawn(executable, ["stdio", "--config", config], {
            stdio: ["pipe", "pipe", "pipe"],
        });
        this.child.stderr.pipe(process.stderr);
        this.exited = new Promise((resolve) => {
            this.child.once("close", resolve);
        });
        this.child.stdout.on("data", (chunk) => this.receive(chunk));
        this.child.on("error", () => this.fail());
        this.child.stdin.on("error", () => this.fail());
        this.child.on("exit", () => this.fail());
    }
    fail(error = new ConnectError("transport-uncertain", "Helper failed; preserve logical request ID")) {
        this.closed = true;
        this.buffer = Buffer.alloc(0);
        const pending = this.pending;
        this.pending = undefined;
        if (pending) {
            clearTimeout(pending.timer);
            pending.reject(error);
        }
        if (this.child.exitCode === null && this.child.signalCode === null)
            this.child.kill("SIGKILL");
    }
    receive(chunk) {
        if (this.closed)
            return;
        if (!this.pending || this.buffer.length + chunk.length > MAX_RESPONSE) {
            this.fail(new ConnectError("protocol", "Unexpected or oversized helper response"));
            return;
        }
        this.buffer = Buffer.concat([this.buffer, chunk]);
        const newline = this.buffer.indexOf(10);
        if (newline < 0)
            return; // The exchange timer also covers partial responses.
        if (newline !== this.buffer.length - 1) {
            this.fail(new ConnectError("protocol", "Extra helper response"));
            return;
        }
        const pending = this.pending;
        const bytes = this.buffer.subarray(0, newline);
        this.buffer = Buffer.alloc(0);
        try {
            const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
            const value = helperResponse(text, pending.id, pending.expected);
            this.pending = undefined;
            clearTimeout(pending.timer);
            pending.resolve(this.observeOutcome(value));
        }
        catch (error) {
            const failure = error instanceof ConnectError
                ? error
                : new ConnectError("protocol", "Invalid helper response");
            if (failure.code === "protocol")
                this.fail(failure);
            else {
                this.pending = undefined;
                clearTimeout(pending.timer);
                pending.reject(failure);
            }
        }
    }
    request(operation, taskBinding) {
        const next = this.chain.then(() => new Promise((resolve, reject) => {
            if (this.closed) {
                reject(new ConnectError("transport-uncertain", "Client closed"));
                return;
            }
            this.requireTaskProfile(operation, taskBinding);
            const expected = expectedKind(operation);
            const id = randomUUID();
            const line = JSON.stringify({
                id,
                operation,
                task_binding: taskBinding,
            }) + "\n";
            if (Buffer.byteLength(line) > MAX_REQUEST) {
                reject(new ConnectError("invalid-request", "Operation exceeds 64 KiB"));
                return;
            }
            const timer = setTimeout(() => this.fail(), exchangeTimeout(this.timeoutMs, operation));
            this.pending = { id, expected, resolve, reject, timer };
            this.child.stdin.write(line);
        }));
        this.chain = next.catch(() => { });
        return next;
    }
    async close() {
        if (!this.closed) {
            if (this.pending)
                this.fail();
            else {
                this.closed = true;
                this.child.stdin.end();
            }
        }
        const timer = setTimeout(() => this.child.kill("SIGKILL"), 3000);
        try {
            await this.exited;
        }
        finally {
            clearTimeout(timer);
        }
    }
}
