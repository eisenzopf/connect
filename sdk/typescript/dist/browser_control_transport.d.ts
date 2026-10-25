export interface BrowserControlTransport {
    readonly ready: Promise<void>;
    readonly closed: Promise<void>;
    readonly datagrams?: WebTransportDatagramDuplexStream;
    stream(): Promise<ReadableWritablePair<Uint8Array>>;
    close(): void;
}
export declare class WebTransportControl implements BrowserControlTransport {
    private readonly transport;
    readonly ready: Promise<void>;
    readonly closed: Promise<void>;
    constructor(url: string, hashes?: WebTransportHash[]);
    get datagrams(): WebTransportDatagramDuplexStream;
    stream(): Promise<ReadableWritablePair<Uint8Array>>;
    close(): void;
}
/** WebSocket sends one canonical JSON envelope per text frame. Internal length
 * framing preserves the same signed-envelope reader used by WebTransport. */
export declare class WebSocketControl implements BrowserControlTransport {
    readonly ready: Promise<void>;
    readonly closed: Promise<void>;
    private readonly socket;
    private readonly readable;
    private readonly writable;
    private controller;
    private finished;
    private claimed;
    private readableEnded;
    constructor(url: string);
    stream(): Promise<ReadableWritablePair<Uint8Array>>;
    close(): void;
}
