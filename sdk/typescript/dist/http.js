import { ConnectError } from "./errors.js";
import { Operations } from "./operations.js";
import { decodeResponse, expectedKind, MAX_REQUEST, MAX_RESPONSE, parseJson, timeoutValue, } from "./protocol.js";
/** Explicit loopback HTTP facade. No redirects or automatic mutation retries. */
export class HttpClient extends Operations {
    credential;
    url;
    baseUrl;
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
        this.baseUrl = url.replace(/\/$/, "");
        this.url = this.baseUrl + "/v1/operations";
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
            const raw = await this.readBounded(response, MAX_RESPONSE);
            let text;
            try {
                text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
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
    /** Upload one raw artifact chunk without base64 expansion. */
    async uploadArtifactChunk(conversationId, artifactId, requestId, offset, data) {
        if (data.byteLength === 0 || data.byteLength > 1024 * 1024)
            throw new ConnectError("invalid-request", "Artifact chunk is outside SDK bounds");
        const query = new URLSearchParams({
            request_id: requestId,
            offset: String(offset),
        });
        const response = await this.artifactFetch(conversationId, artifactId, query, "PUT", Uint8Array.from(data).buffer);
        const raw = await this.readBounded(response, MAX_RESPONSE);
        const text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
        return decodeResponse(parseJson(text), "artifact");
    }
    /** Download one raw artifact chunk with validated framing metadata. */
    async downloadArtifactChunk(conversationId, artifactId, offset, limit) {
        if (!Number.isSafeInteger(limit) || limit <= 0 || limit > 1024 * 1024)
            throw new ConnectError("invalid-request", "Artifact chunk limit is outside SDK bounds");
        const query = new URLSearchParams({
            offset: String(offset),
            limit: String(limit),
        });
        const response = await this.artifactFetch(conversationId, artifactId, query, "GET");
        if (!response.ok) {
            const raw = await this.readBounded(response, MAX_RESPONSE);
            const text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
            decodeResponse(parseJson(text), "artifact_chunk");
            throw new ConnectError("protocol", "Unexpected artifact response");
        }
        const responseOffset = Number(response.headers.get("x-connect-offset"));
        const eofHeader = response.headers.get("x-connect-eof");
        if (!Number.isSafeInteger(responseOffset) ||
            responseOffset !== offset ||
            !["true", "false"].includes(eofHeader ?? "") ||
            response.headers.get("content-type")?.split(";", 1)[0] !==
                "application/octet-stream")
            throw new ConnectError("protocol", "Invalid artifact byte response");
        return {
            data: await this.readBounded(response, limit),
            offset: responseOffset,
            eof: eofHeader === "true",
        };
    }
    async artifactFetch(conversationId, artifactId, query, method, body) {
        try {
            const response = await fetch(`${this.baseUrl}/v1/conversations/${encodeURIComponent(conversationId)}/artifacts/${encodeURIComponent(artifactId)}?${query}`, {
                method,
                redirect: "manual",
                body,
                headers: {
                    "content-type": "application/octet-stream",
                    authorization: "Bearer " + this.credential,
                },
                signal: AbortSignal.timeout(this.timeoutMs),
            });
            if (response.status >= 300 && response.status < 400) {
                await response.body?.cancel();
                throw new ConnectError("protocol", "HTTP redirect refused");
            }
            return response;
        }
        catch (error) {
            if (error instanceof ConnectError)
                throw error;
            throw new ConnectError("transport-uncertain", "HTTP failed; preserve logical request ID");
        }
    }
    async readBounded(response, maximum) {
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
                if (bytes > maximum) {
                    await reader.cancel();
                    throw new ConnectError("protocol", "Response exceeds bounds");
                }
                chunks.push(value);
            }
        }
        finally {
            reader.releaseLock();
        }
        const result = new Uint8Array(bytes);
        let offset = 0;
        for (const chunk of chunks) {
            result.set(chunk, offset);
            offset += chunk.byteLength;
        }
        return result;
    }
}
