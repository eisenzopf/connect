import { type JsonObject } from "./uctp_json.js";
export interface Envelope extends JsonObject {
    v: 2;
    type: string;
    id: string;
    ts: string;
    payload: JsonObject;
}
export interface Scope {
    cid?: string;
    sid?: string;
    connid?: string;
    operation_id?: string;
    expected_revision?: number;
    in_reply_to?: string;
}
export declare function identifier(value: unknown): value is string;
export declare function envelope(type: string, payload: JsonObject, scope?: Scope): Envelope;
export declare function validateEnvelope(frame: unknown): asserts frame is Envelope;
export declare function encodeEnvelope(frame: JsonObject): Uint8Array<ArrayBuffer>;
/** Incremental length framing with one bounded frame under assembly. */
export declare class EnvelopeReader {
    private header;
    private headerSize;
    private body?;
    private bodySize;
    push(chunk: Uint8Array): Generator<Envelope>;
    end(): void;
}
