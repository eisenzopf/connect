export type Operation = {
    op: 'resolve';
    address: string;
} | {
    op: 'create';
    request_id: string;
    participants: string[];
} | {
    op: 'send';
    request_id: string;
    conversation_id: string;
    body: string;
} | {
    op: 'history';
    conversation_id: string;
    after: string | null;
    limit: number;
} | {
    op: 'inbox';
    cursor: string | null;
    limit: number;
    wait_ms: number;
} | {
    op: 'request_status';
    request_id: string;
} | {
    op: 'provider_status';
    conversation_id: string;
} | {
    op: 'close';
    request_id: string;
    conversation_id: string;
} | {
    op: 'call_start';
    request_id: string;
    conversation_id: string;
    route_id: string;
    destination: string;
} | {
    op: 'call_status';
    conversation_id: string;
    call_id: string;
} | {
    op: 'call_end';
    request_id: string;
    conversation_id: string;
    call_id: string;
} | {
    op: 'health';
};
export interface Outcome {
    kind: string;
    data: Record<string, unknown>;
}
export declare class ConnectError extends Error {
    code: string;
    constructor(code: string, message: string);
}
declare abstract class Operations {
    abstract request(operation: Operation): Promise<Outcome>;
    resolve(address: string): Promise<Record<string, unknown>>;
    create(request_id: string, participants: string[]): Promise<Record<string, unknown>>;
    send(request_id: string, conversation_id: string, body: string): Promise<Record<string, unknown>>;
    history(conversation_id: string, after?: string | null, limit?: number): Promise<Record<string, unknown>>;
    inbox(cursor?: string | null, limit?: number, wait_ms?: number): Promise<Record<string, unknown>>;
    requestStatus(request_id: string): Promise<Record<string, unknown>>;
    providerStatus(conversation_id: string): Promise<Record<string, unknown>>;
    closeConversation(request_id: string, conversation_id: string): Promise<Record<string, unknown>>;
    callStart(request_id: string, conversation_id: string, route_id: string, destination: string): Promise<Record<string, unknown>>;
    callStatus(conversation_id: string, call_id: string): Promise<Record<string, unknown>>;
    callEnd(request_id: string, conversation_id: string, call_id: string): Promise<Record<string, unknown>>;
    health(): Promise<Record<string, unknown>>;
}
/** Rust helper owns TLS/auth/UCTP/QUIC. No automatic mutation retries. */
export declare class NativeClient extends Operations {
    private timeoutMs;
    private child;
    private pending?;
    private closed;
    private chain;
    constructor(config: string, executable?: string, timeoutMs?: number);
    private fail;
    request(operation: Operation): Promise<Outcome>;
    close(): Promise<void>;
}
/** HTTP facade only; it does not advertise native QUIC. */
export declare class HttpClient extends Operations {
    private credential;
    private timeoutMs;
    private url;
    constructor(url: string, credential: string, timeoutMs?: number);
    request(operation: Operation): Promise<Outcome>;
}
export {};
