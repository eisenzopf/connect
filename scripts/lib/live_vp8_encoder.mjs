// Owned finite fixture encoder. It grants no transport or feedback authority.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { lstat } from "node:fs/promises";
import { isAbsolute } from "node:path";

const MAX_FRAME = 256 * 1024;
const HEADER = 16;
const RAW_FRAME = 640 * 360 * 3 / 2;

export class LiveVp8Encoder {
    #child;
    #closed;
    #exit;
    #failure;
    #pending;
    #buffer = Buffer.alloc(0);
    #previous = -1;
    #count;
    #closing = false;
    #join;

    static async open(executable, raw, count) {
        assert.ok([executable, raw].every(p => typeof p === "string" && isAbsolute(p) && !p.includes("\0")));
        assert.ok(Number.isInteger(count) && count >= 1 && count <= 1860);
        const [binary, input] = await Promise.all([lstat(executable), lstat(raw)]);
        assert.ok(binary.isFile() && !binary.isSymbolicLink() && (binary.mode & 0o111));
        assert.ok(input.isFile() && !input.isSymbolicLink() && !(input.mode & 0o077));
        assert.equal(input.size, count * RAW_FRAME);
        return new LiveVp8Encoder(spawn(executable, [raw, String(count)], {
            stdio: ["pipe", "pipe", "ignore"], env: { LANG: "C" },
        }), count);
    }

    constructor(child, count) {
        this.#child = child;
        this.#count = count;
        this.#closed = new Promise(resolve => {
            child.once("close", (code, signal) => {
                this.#exit = { code, signal };
                if (!this.#closing || this.#pending || this.#buffer.length || code !== 0 || signal !== null)
                    this.#fail(Error("live encoder ended before clean owned shutdown"));
                resolve(this.#exit);
            });
        });
        child.once("error", () => this.#fail(Error("live encoder process failed")));
        child.stdin.on("error", () => this.#fail(Error("live encoder input failed")));
        child.stdout.on("error", () => this.#fail(Error("live encoder output failed")));
        child.stdout.on("data", chunk => this.#receive(chunk));
    }

    get pid() { return this.#child.pid; }

    #fail(error) {
        this.#failure ??= error;
        if (this.#pending) {
            const pending = this.#pending;
            this.#pending = undefined;
            pending.clear();
            pending.reject(this.#failure);
        }
        this.#buffer = Buffer.alloc(0);
        if (!this.#exit) this.#child.kill("SIGKILL");
    }

    #receive(chunk) {
        if (this.#failure) return;
        try {
            const pending = this.#pending;
            assert.ok(pending, "unsolicited encoder output");
            assert.ok(this.#buffer.length + chunk.length <= HEADER + MAX_FRAME, "encoder output bound");
            this.#buffer = Buffer.concat([this.#buffer, chunk]);
            if (this.#buffer.length < HEADER) return;
            const bytes = this.#buffer;
            assert.equal(bytes.subarray(0, 4).toString("ascii"), "V8F1");
            assert.equal(bytes.readUInt32LE(4), pending.index);
            const length = bytes.readUInt32LE(8), key = bytes.readUInt32LE(12);
            assert.ok(length >= 3 && length <= MAX_FRAME && (key === 0 || key === 1));
            assert.ok(bytes.length <= HEADER + length, "extra encoder reply");
            if (bytes.length < HEADER + length) return;
            const frame = Buffer.from(bytes.subarray(HEADER));
            assert.equal(!(frame[0] & 1), Boolean(key));
            if (key) {
                assert.ok(frame.length >= 10 && frame.subarray(3, 6).equals(Buffer.from([0x9d, 1, 0x2a])));
                assert.equal(frame.readUInt16LE(6) & 0x3fff, 640);
                assert.equal(frame.readUInt16LE(8) & 0x3fff, 360);
            }
            assert.ok(!pending.force || key, "requested fresh keyframe missing");
            this.#buffer = Buffer.alloc(0);
            this.#pending = undefined;
            pending.clear();
            pending.resolve(Object.freeze({ index: pending.index, timestamp: pending.index * 3000,
                key: Boolean(key), bytes: frame }));
        } catch { this.#fail(Error("live encoder reply violated the original request")); }
    }

    // One request belongs to this owner until its complete reply or joined
    // process failure. Racing a caller's wait does not consume/reissue pipe IO.
    async frame(index, force, { signal, timeoutMs = 100 } = {}) {
        assert.ok(Number.isInteger(index) && index > this.#previous && index < this.#count);
        assert.equal(typeof force, "boolean");
        assert.ok(Number.isInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 200);
        assert.ok(!this.#pending, "one encoder request at a time");
        if (this.#failure) throw this.#failure;
        assert.ok(!this.#closing && !this.#exit, "closed encoder owner");
        if (signal?.aborted) {
            this.#fail(Error("live encoder request cancelled"));
            await this.stopAndJoin();
            throw this.#failure;
        }
        this.#previous = index;
        const promise = new Promise((resolve, reject) => {
            const abort = () => this.#fail(Error("live encoder request cancelled"));
            const timer = setTimeout(() => this.#fail(Error("live encoder original request deadline")), timeoutMs);
            this.#pending = { index, force, resolve, reject,
                clear: () => { clearTimeout(timer); signal?.removeEventListener("abort", abort); } };
            signal?.addEventListener("abort", abort, { once: true });
            const request = Buffer.alloc(8);
            request.writeUInt32LE(index); request.writeUInt32LE(Number(force), 4);
            // A single eight-byte write; no unbounded command queue or retry.
            this.#child.stdin.write(request);
        });
        try { return await promise; }
        catch (error) { await this.stopAndJoin(); throw error; }
    }

    // Always join before resolving, including cancellation and failed replies.
    // Clean EOF is attempted only when there is no partially owned request.
    stopAndJoin() {
        return this.#join ??= (async () => {
            this.#closing = true;
            if (this.#pending) this.#fail(Error("live encoder stopped with a pending original request"));
            if (!this.#exit) {
                if (this.#failure) this.#child.kill("SIGKILL");
                else this.#child.stdin.end();
            }
            const timer = setTimeout(() => this.#fail(Error("live encoder shutdown deadline")), 500);
            try {
                const exit = await this.#closed;
                return Object.freeze({ joined: true, clean: !this.#failure && exit.code === 0 && exit.signal === null,
                    exitCode: exit.code, signal: exit.signal });
            } finally { clearTimeout(timer); }
        })();
    }
}
