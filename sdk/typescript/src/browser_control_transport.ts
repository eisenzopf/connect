// Canonical signed control transports; no media or operation replay.
import { ConnectError } from "./errors.js";
const maximumEnvelope = 1_048_576;
const maximumQueuedBytes = 4 * (maximumEnvelope + 4);
export interface BrowserControlTransport {
    readonly ready: Promise<void>;
    readonly closed: Promise<void>;
    readonly datagrams?: WebTransportDatagramDuplexStream;
    stream(): Promise<ReadableWritablePair<Uint8Array>>;
    close(): void;
}
export class WebTransportControl implements BrowserControlTransport {
    private readonly transport: WebTransport;
    readonly ready: Promise<void>;
    readonly closed: Promise<void>;
    constructor(url: string, hashes?: WebTransportHash[]) {
        this.transport = new WebTransport(url, {
            allowPooling: false,
            serverCertificateHashes: hashes,
        });
        this.ready = this.transport.ready;
        this.closed = this.transport.closed.then(() => {});
    }
    get datagrams(): WebTransportDatagramDuplexStream {
        return this.transport.datagrams;
    }
    stream(): Promise<ReadableWritablePair<Uint8Array>> {
        return this.transport.createBidirectionalStream();
    }
    close(): void {
        this.transport.close({ closeCode: 0, reason: "Connect client closed" });
    }
}
/** WebSocket sends one canonical JSON envelope per text frame. Internal length
 * framing preserves the same signed-envelope reader used by WebTransport. */
export class WebSocketControl implements BrowserControlTransport {
    readonly ready: Promise<void>;
    readonly closed: Promise<void>;
    private readonly socket: WebSocket;
    private readonly readable: ReadableStream<Uint8Array>;
    private readonly writable: WritableStream<Uint8Array>;
    private controller!: ReadableStreamDefaultController<Uint8Array>;
    private finished = false;
    private claimed = false;
    private readableEnded = false;
    constructor(url: string) {
        this.socket = new WebSocket(url);
        this.readable = new ReadableStream<Uint8Array>(
            {
                start: (controller) => {
                    this.controller = controller;
                },
                cancel: () => {
                    this.readableEnded = true;
                    this.close();
                },
            },
            {
                highWaterMark: maximumQueuedBytes,
                size: (bytes) => bytes.byteLength,
            },
        );
        let resolveReady!: () => void, rejectReady!: (error: Error) => void;
        this.ready = new Promise((resolve, reject) => {
            resolveReady = resolve;
            rejectReady = reject;
        });
        let resolveClosed!: () => void, rejectClosed!: (error: Error) => void;
        this.closed = new Promise((resolve, reject) => {
            resolveClosed = resolve;
            rejectClosed = reject;
        });
        const fail = (code: string, message: string) => {
            if (this.finished) return;
            this.finished = true;
            const error = new ConnectError(code, message);
            rejectReady(error);
            rejectClosed(error);
            if (!this.readableEnded) {
                this.readableEnded = true;
                this.controller.error(error);
            }
            this.socket.close();
        };
        this.socket.addEventListener("open", () => resolveReady());
        this.socket.addEventListener("error", () =>
            fail("transport", "Canonical WebSocket transport failed"),
        );
        this.socket.addEventListener("close", () => {
            if (this.finished) return;
            this.finished = true;
            rejectReady(
                new ConnectError(
                    "transport",
                    "Canonical WebSocket closed before ready",
                ),
            );
            if (!this.readableEnded) {
                this.readableEnded = true;
                this.controller.close();
            }
            resolveClosed();
        });
        this.socket.addEventListener("message", (event) => {
            if (this.finished || this.readableEnded) return;
            if (
                typeof event.data !== "string" ||
                event.data.length > maximumEnvelope
            ) {
                fail(
                    "protocol",
                    "Canonical WebSocket requires bounded text envelopes",
                );
                return;
            }
            const body = new TextEncoder().encode(event.data);
            if (
                body.byteLength === 0 ||
                body.byteLength > maximumEnvelope ||
                (this.controller.desiredSize ?? 0) < body.byteLength + 4
            ) {
                fail(
                    "backpressure",
                    "Canonical WebSocket receive budget exceeded",
                );
                return;
            }
            const framed = new Uint8Array(body.byteLength + 4);
            new DataView(framed.buffer).setUint32(0, body.byteLength);
            framed.set(body, 4);
            this.controller.enqueue(framed);
        });
        this.writable = new WritableStream<Uint8Array>({
            write: (bytes) => {
                if (this.finished || this.socket.readyState !== WebSocket.OPEN)
                    throw new ConnectError(
                        "closed",
                        "Canonical WebSocket is closed",
                    );
                const length = bytes.byteLength;
                if (
                    length < 5 ||
                    length > maximumEnvelope + 4 ||
                    new DataView(
                        bytes.buffer,
                        bytes.byteOffset,
                        length,
                    ).getUint32(0) !==
                        length - 4
                )
                    throw new ConnectError(
                        "protocol",
                        "Canonical WebSocket requires one bounded envelope",
                    );
                if (this.socket.bufferedAmount + length - 4 > maximumEnvelope)
                    throw new ConnectError(
                        "backpressure",
                        "Canonical WebSocket send budget exceeded",
                    );
                // Completing send means local admission only. Only the original
                // correlated canonical receipt can establish a mutation outcome.
                let text: string;
                try {
                    text = new TextDecoder("utf-8", { fatal: true }).decode(
                        bytes.subarray(4),
                    );
                } catch {
                    throw new ConnectError(
                        "protocol",
                        "Canonical WebSocket requires valid UTF-8",
                    );
                }
                this.socket.send(text);
            },
            abort: () => this.close(),
            close: () => this.close(),
        });
    }
    async stream(): Promise<ReadableWritablePair<Uint8Array>> {
        if (this.claimed)
            throw new ConnectError(
                "conflict",
                "Canonical control stream already owned",
            );
        this.claimed = true;
        await this.ready;
        return { readable: this.readable, writable: this.writable };
    }
    close(): void {
        if (this.finished) return;
        this.socket.close(1000, "Connect client closed");
    }
}
