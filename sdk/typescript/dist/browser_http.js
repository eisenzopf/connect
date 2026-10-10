import { exchangeTimeout } from "./task_operations.js";
import { ConnectError } from "./errors.js";
import { Operations } from "./operations.js";
import { decodeResponse, expectedKind, MAX_REQUEST, MAX_RESPONSE, parseJson, timeoutValue, } from "./protocol.js";
/** Browser-safe loopback HTTP client. Credentials remain memory-only. */
export class BrowserHttpClient extends Operations {
    credential;
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
        this.timeoutMs = timeoutValue(timeoutMs);
    }
    async request(operation, taskBinding) {
        this.requireTaskProfile(operation, taskBinding);
        const expected = expectedKind(operation);
        const body = JSON.stringify(operation);
        if (new TextEncoder().encode(body).byteLength > MAX_REQUEST - 1)
            throw new ConnectError("invalid-request", "Operation exceeds 64 KiB");
        const response = await this.fetch(this.baseUrl + "/v1/operations", "POST", body, "application/json", exchangeTimeout(this.timeoutMs, operation));
        return this.observeOutcome(this.decode(await this.readBounded(response, MAX_RESPONSE), expected));
    }
    async uploadArtifactChunk(conversationId, artifactId, requestId, offset, data) {
        if (data.byteLength === 0 || data.byteLength > 1024 * 1024)
            throw new ConnectError("invalid-request", "Artifact chunk is outside SDK bounds");
        const query = new URLSearchParams({
            request_id: requestId,
            offset: String(offset),
        });
        const response = await this.fetch(this.artifactUrl(conversationId, artifactId, query), "PUT", Uint8Array.from(data).buffer, "application/octet-stream");
        return this.decode(await this.readBounded(response, MAX_RESPONSE), "artifact");
    }
    async downloadArtifactChunk(conversationId, artifactId, offset, limit) {
        if (!Number.isSafeInteger(limit) || limit <= 0 || limit > 1024 * 1024)
            throw new ConnectError("invalid-request", "Artifact chunk limit is outside SDK bounds");
        const query = new URLSearchParams({
            offset: String(offset),
            limit: String(limit),
        });
        const response = await this.fetch(this.artifactUrl(conversationId, artifactId, query), "GET");
        if (!response.ok) {
            this.decode(await this.readBounded(response, MAX_RESPONSE), "artifact_chunk");
            throw new ConnectError("protocol", "Unexpected HTTP artifact response");
        }
        const responseOffset = Number(response.headers.get("x-connect-offset"));
        const eof = response.headers.get("x-connect-eof");
        if (responseOffset !== offset ||
            !Number.isSafeInteger(responseOffset) ||
            !["true", "false"].includes(eof ?? "") ||
            response.headers.get("content-type")?.split(";", 1)[0] !==
                "application/octet-stream")
            throw new ConnectError("protocol", "Invalid artifact byte response");
        return {
            data: await this.readBounded(response, limit),
            offset: responseOffset,
            eof: eof === "true",
        };
    }
    artifactUrl(conversationId, artifactId, query) {
        return `${this.baseUrl}/v1/conversations/${encodeURIComponent(conversationId)}/artifacts/${encodeURIComponent(artifactId)}?${query}`;
    }
    async fetch(url, method, body, contentType, timeoutMs = this.timeoutMs) {
        try {
            const response = await globalThis.fetch(url, {
                method,
                body,
                redirect: "manual",
                headers: {
                    authorization: "Bearer " + this.credential,
                    ...(contentType ? { "content-type": contentType } : {}),
                },
                signal: AbortSignal.timeout(timeoutMs),
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
    decode(raw, expected) {
        try {
            const text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
            return decodeResponse(parseJson(text), expected);
        }
        catch (error) {
            if (error instanceof ConnectError)
                throw error;
            throw new ConnectError("protocol", "Invalid UTF-8 response");
        }
    }
    async readBounded(response, maximum) {
        const reader = response.body?.getReader();
        if (!reader)
            throw new ConnectError("protocol", "Missing HTTP body");
        const chunks = [];
        let size = 0;
        try {
            while (true) {
                const { value, done } = await reader.read();
                if (done)
                    break;
                size += value.byteLength;
                if (size > maximum) {
                    await reader.cancel();
                    throw new ConnectError("protocol", "Response exceeds bounds");
                }
                chunks.push(value);
            }
        }
        finally {
            reader.releaseLock();
        }
        const result = new Uint8Array(size);
        let offset = 0;
        for (const chunk of chunks) {
            result.set(chunk, offset);
            offset += chunk.byteLength;
        }
        return result;
    }
}
