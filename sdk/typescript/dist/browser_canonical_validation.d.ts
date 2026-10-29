import { type Json } from "./uctp_json.js";
import { type Envelope } from "./uctp_wire.js";
/** Full fixed schema/registry validation only; no signature, authorization or media claim. */
export declare function validateBrowserCanonical(value: Json): asserts value is Envelope;
