import { ConnectError } from "./errors.js";
import { Operations } from "./operations.js";
import { decodeResponse, expectedKind, MAX_REQUEST, MAX_RESPONSE, parseJson, timeoutValue, } from "./protocol.js";
/** Explicit loopback HTTP facade. No redirects or automatic mutation retries. */
export class HttpClient extends Operations {
    credential;
    url;
    timeoutMs;
    constructor(url, credential, timeoutMs = 12000) {
        super();
        this.credential = credential;
        const parsed = new URL(url);
        if (parsed.protocol !== "http:" ||
            !["127.0.0.1", "[::1]", "localhost"].includes(parsed.hostname) ||
            parsed.username ||
            parsed.password ||
            parsed.search ||
            parsed.hash)
            throw new Error("Use a loopback HTTP URL");
        this.url = url.replace(/\/$/, "") + "/v1/operations";
        this.timeoutMs = timeoutValue(timeoutMs);
    }
    async request(operation) {
        const expected = expectedKind(operation);
        const body = JSON.stringify(operation);
        if (Buffer.byteLength(body) > MAX_REQUEST - 1)
            throw new ConnectError("invalid-request", "Operation exceeds 64 KiB");
        try {
            const response = await fetch(this.url, {
                method: "POST",
                redirect: "manual",
                body,
                headers: {
                    "content-type": "application/json",
                    authorization: "Bearer " + this.credential,
                },
                signal: AbortSignal.timeout(this.timeoutMs),
            });
            if (response.status >= 300 && response.status < 400) {
                await response.body?.cancel();
                throw new ConnectError("protocol", "HTTP redirect refused");
            }
            const reader = response.body?.getReader();
            if (!reader)
                throw new ConnectError("protocol", "Missing HTTP body");
            const chunks = [];
            let bytes = 0;
            try {
                while (true) {
                    const { value, done } = await reader.read();
                    if (done)
                        break;
                    bytes += value.byteLength;
                    if (bytes > MAX_RESPONSE) {
                        await reader.cancel();
                        throw new ConnectError("protocol", "Response exceeds bounds");
                    }
                    chunks.push(Buffer.from(value));
                }
            }
            finally {
                reader.releaseLock();
            }
            let text;
            try {
                text = new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks));
            }
            catch {
                throw new ConnectError("protocol", "Invalid UTF-8 response");
            }
            return decodeResponse(parseJson(text), expected);
        }
        catch (error) {
            if (error instanceof ConnectError)
                throw error;
            throw new ConnectError("transport-uncertain", "HTTP failed; preserve logical request ID");
        }
    }
}
