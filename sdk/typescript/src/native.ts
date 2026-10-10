import { exchangeTimeout } from "./task_operations.js";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import { ConnectError } from "./errors.js";
import { Operations } from "./operations.js";
import {
    expectedKind,
    helperResponse,
    MAX_REQUEST,
    MAX_RESPONSE,
    timeoutValue,
} from "./protocol.js";
import type { Operation, Outcome } from "./types.js";
interface Pending {
    id: string;
    expected: Outcome["kind"];
    resolve: (v: Outcome) => void;
    reject: (error: ConnectError) => void;
    timer: NodeJS.Timeout;
}
/** Rust owns TLS/auth/UCTP/QUIC; helper failure closes this client. */
export class NativeClient extends Operations {
    private readonly child: ChildProcessWithoutNullStreams;
    private readonly exited: Promise<void>;
    private pending?: Pending;
    private buffer: Buffer = Buffer.alloc(0);
    private closed = false;
    private chain: Promise<unknown> = Promise.resolve();
    private readonly timeoutMs: number;
    constructor(config: string, executable = "connect", timeoutMs = 15000) {
        super();
        this.timeoutMs = timeoutValue(timeoutMs);
        this.child = spawn(executable, ["stdio", "--config", config], {
            stdio: ["pipe", "pipe", "pipe"],
        });
        this.child.stderr.pipe(process.stderr);
        this.exited = new Promise((resolve) => {
            this.child.once("close", resolve);
        });
        this.child.stdout.on("data", (chunk: Buffer) => this.receive(chunk));
        this.child.on("error", () => this.fail());
        this.child.stdin.on("error", () => this.fail());
        this.child.on("exit", () => this.fail());
    }
    private fail(
        error = new ConnectError(
            "transport-uncertain",
            "Helper failed; preserve logical request ID",
        ),
    ): void {
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
    private receive(chunk: Buffer): void {
        if (this.closed) return;
        if (!this.pending || this.buffer.length + chunk.length > MAX_RESPONSE) {
            this.fail(
                new ConnectError(
                    "protocol",
                    "Unexpected or oversized helper response",
                ),
            );
            return;
        }
        this.buffer = Buffer.concat([this.buffer, chunk]);
        const newline = this.buffer.indexOf(10);
        if (newline < 0) return; // The exchange timer also covers partial responses.
        if (newline !== this.buffer.length - 1) {
            this.fail(new ConnectError("protocol", "Extra helper response"));
            return;
        }
        const pending = this.pending;
        const bytes = this.buffer.subarray(0, newline);
        this.buffer = Buffer.alloc(0);
        try {
            const text = new TextDecoder("utf-8", { fatal: true }).decode(
                bytes,
            );
            const value = helperResponse(text, pending.id, pending.expected);
            this.pending = undefined;
            clearTimeout(pending.timer);
            pending.resolve(this.observeOutcome(value));
        } catch (error) {
            const failure =
                error instanceof ConnectError
                    ? error
                    : new ConnectError("protocol", "Invalid helper response");
            if (failure.code === "protocol") this.fail(failure);
            else {
                this.pending = undefined;
                clearTimeout(pending.timer);
                pending.reject(failure);
            }
        }
    }
    request(operation: Operation): Promise<Outcome> {
        const next = this.chain.then(
            () =>
                new Promise<Outcome>((resolve, reject) => {
                    if (this.closed) {
                        reject(
                            new ConnectError(
                                "transport-uncertain",
                                "Client closed",
                            ),
                        );
                        return;
                    }
                    this.requireTaskProfile(operation);
                    const expected = expectedKind(operation);
                    const id = randomUUID();
                    const line = JSON.stringify({ id, operation }) + "\n";
                    if (Buffer.byteLength(line) > MAX_REQUEST) {
                        reject(
                            new ConnectError(
                                "invalid-request",
                                "Operation exceeds 64 KiB",
                            ),
                        );
                        return;
                    }
                    const timer = setTimeout(
                        () => this.fail(),
                        exchangeTimeout(this.timeoutMs, operation),
                    );
                    this.pending = { id, expected, resolve, reject, timer };
                    this.child.stdin.write(line);
                }),
        );
        this.chain = next.catch(() => {});
        return next;
    }
    async close(): Promise<void> {
        if (!this.closed) {
            if (this.pending) this.fail();
            else {
                this.closed = true;
                this.child.stdin.end();
            }
        }
        const timer = setTimeout(() => this.child.kill("SIGKILL"), 3000);
        try {
            await this.exited;
        } finally {
            clearTimeout(timer);
        }
    }
}
