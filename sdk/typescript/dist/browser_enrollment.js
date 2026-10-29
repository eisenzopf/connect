import { BrowserClient, } from "./browser_client.js";
import { ConnectError } from "./errors.js";
import { parseCanonicalJson } from "./uctp_json.js";
import { validateSigningIdentity } from "./uctp_signing.js";
import { BrowserEnrollmentKey, } from "./browser_enrollment_key.js";
import { browserOrigin, checkCredential, closed, ENROLLMENT_PROFILE, invalid, json, normalizeOffer, normalizeRequest, origin, snapshot, } from "./browser_enrollment_contract.js";
export { BrowserEnrollmentKey } from "./browser_enrollment_key.js";
/** Protected in-memory approved state; JSON serialization exposes no fields.
 * Use BrowserEnrollmentStore to persist before network IO. */
export class BrowserEnrollmentSession {
    #origin;
    #offer;
    #key;
    constructor(origin, offer, key) {
        this.#origin = origin;
        this.#offer = offer;
        this.#key = key;
    }
    /** Explicit private structured-clone restoration, also used by the vault.
     * An expired challenge remains signable for current completed recovery. */
    static async restore(state) {
        if (state.version !== 1 || !state.offer)
            invalid();
        const contact = origin(state.origin);
        const request = normalizeRequest(state.request);
        const offer = normalizeOffer(state.offer);
        const key = await BrowserEnrollmentKey.restore(state.key);
        if (request.browser_origin !== browserOrigin() ||
            json(request) !== json(offer.request) ||
            request.algorithm !== key.algorithm ||
            request.public_key !== key.publicKey)
            invalid();
        return new BrowserEnrollmentSession(contact, offer, key);
    }
    get offer() {
        return snapshot(this.#offer);
    }
    snapshot() {
        return {
            version: 1,
            origin: this.#origin,
            request: snapshot(this.#offer.request),
            offer: this.offer,
            key: this.#key.snapshot(),
        };
    }
    async claim(options = {}) {
        const signal = options.signal;
        if (browserOrigin() !== this.#offer.request.browser_origin)
            invalid();
        const timeout = options.timeoutMs ?? 5000;
        if (!Number.isFinite(timeout) || timeout < 1 || timeout > 60000)
            invalid();
        if (signal?.aborted)
            throw new ConnectError("cancelled", "Enrollment cancelled; retain the original offer and key");
        const controller = new AbortController();
        const deadline = performance.now() + timeout;
        const stopped = () => {
            if (controller.signal.aborted)
                throw controller.signal.reason;
            if (performance.now() >= deadline)
                throw new ConnectError("timeout", "Enrollment timed out; retain the original offer and key");
        };
        const cancel = () => controller.abort(new ConnectError("cancelled", "Enrollment cancelled; retain the original offer and key"));
        signal?.addEventListener("abort", cancel, { once: true });
        const timer = setTimeout(() => controller.abort(new ConnectError("timeout", "Enrollment timed out; retain the original offer and key")), timeout);
        let listener;
        try {
            const work = async () => {
                const claim = await this.#key.claim(this.#offer);
                stopped();
                const body = json(claim);
                const response = await fetch(`${this.#origin}/v2/enrollments/${this.#offer.enrollment_id}/claim`, {
                    method: "POST",
                    body,
                    headers: { "Content-Type": "application/json" },
                    credentials: "omit",
                    redirect: "error",
                    cache: "no-store",
                    referrerPolicy: "no-referrer",
                    signal: controller.signal,
                });
                stopped();
                if (response.redirected ||
                    (response.status >= 300 && response.status < 400) ||
                    !response.body)
                    throw new ConnectError("protocol", "Invalid enrollment response");
                const length = response.headers.get("content-length");
                if (length !== null &&
                    (!/^\d+$/.test(length) || Number(length) > 65536))
                    throw new ConnectError("protocol", "Enrollment response exceeds bounds");
                const reader = response.body.getReader();
                const chunks = [];
                let size = 0;
                try {
                    for (;;) {
                        const next = await reader.read();
                        stopped();
                        if (next.done)
                            break;
                        size += next.value.length;
                        if (size > 65536)
                            throw new ConnectError("protocol", "Enrollment response exceeds bounds");
                        chunks.push(next.value);
                    }
                }
                finally {
                    void reader.cancel().catch(() => { });
                }
                const bytes = new Uint8Array(size);
                let offset = 0;
                for (const chunk of chunks) {
                    bytes.set(chunk, offset);
                    offset += chunk.length;
                }
                const decoded = parseCanonicalJson(bytes, 65536);
                closed(decoded, ["profile", "result"]);
                if (decoded.profile !== ENROLLMENT_PROFILE)
                    throw new ConnectError("protocol", "Invalid enrollment response profile");
                const result = decoded.result;
                if (result &&
                    typeof result === "object" &&
                    Object.hasOwn(result, "Err")) {
                    closed(result, ["Err"]);
                    closed(result.Err, ["code", "message"]);
                    if (typeof result.Err.code !== "string" ||
                        typeof result.Err.message !== "string")
                        throw new ConnectError("protocol", "Invalid enrollment refusal");
                    const code = typeof result.Err.code === "string" &&
                        [
                            "unauthorized",
                            "forbidden",
                            "conflict",
                            "gone",
                            "capacity-exceeded",
                            "invalid-request",
                            "storage-unavailable",
                            "transport-uncertain",
                        ].includes(result.Err.code)
                        ? result.Err.code
                        : "enrollment-refused";
                    throw new ConnectError(code, "Enrollment refused; retain the original offer and key", undefined, response.status);
                }
                closed(result, ["Ok"]);
                if (response.status !== 200)
                    throw new ConnectError("protocol", "Invalid enrollment response status");
                const credential = checkCredential(result.Ok, this.#offer);
                stopped();
                return credential;
            };
            const aborted = new Promise((_, reject) => {
                listener = () => reject(controller.signal.reason);
                controller.signal.addEventListener("abort", listener, {
                    once: true,
                });
            });
            const result = await Promise.race([work(), aborted]);
            stopped();
            return result;
        }
        catch (error) {
            stopped();
            if (error instanceof ConnectError) {
                if (error.code === "invalid-request")
                    throw new ConnectError("protocol", "Invalid enrollment response");
                throw error;
            }
            throw new ConnectError("transport-uncertain", "Enrollment response unavailable; retain the original offer and key");
        }
        finally {
            clearTimeout(timer);
            signal?.removeEventListener("abort", cancel);
            if (listener)
                controller.signal.removeEventListener("abort", listener);
            controller.abort();
        }
    }
    /** Explicit memory-only installation into exactly the approved transport.
     * Contact/TLS and server signing trust must come from trusted configuration. */
    clientOptions(credential, contact) {
        if (browserOrigin() !== this.#offer.request.browser_origin)
            invalid();
        const value = checkCredential(credential, this.#offer);
        const r = this.#offer.request;
        if (typeof contact.url !== "string" || contact.url.length > 2048)
            invalid();
        const pins = contact.serverCertificateHashes;
        if (pins !== undefined &&
            (r.transport !== "webtransport" ||
                !Array.isArray(pins) ||
                pins.length > 8))
            invalid();
        for (const pin of pins ?? []) {
            if (pin.algorithm !== "sha-256" ||
                !(pin.value instanceof ArrayBuffer ||
                    ArrayBuffer.isView(pin.value)) ||
                pin.value.byteLength !== 32)
                invalid();
        }
        const url = new URL(contact.url);
        if (url.href !== contact.url ||
            url.protocol !==
                (r.transport === "webtransport" ? "https:" : "wss:") ||
            url.username ||
            url.password ||
            url.search ||
            url.hash ||
            url.port === "0")
            invalid();
        validateSigningIdentity(contact.server, "verify");
        const server = { ...contact.server };
        if (!Number.isFinite(server.notBefore) ||
            !Number.isFinite(server.expiresAt) ||
            server.notBefore > Date.now() / 1000 ||
            server.expiresAt <= Date.now() / 1000)
            invalid();
        return {
            url: url.href,
            transport: r.transport === "webtransport" ? "webtransport" : "websocket",
            identityId: r.identity_id,
            credential: value.credential.credential,
            device: { ...r.device },
            signing: this.#key.signing(r.key_id),
            server,
            serverCertificateHashes: r.transport === "webtransport"
                ? contact.serverCertificateHashes?.map((h) => ({
                    algorithm: h.algorithm,
                    value: ArrayBuffer.isView(h.value)
                        ? Uint8Array.from(new Uint8Array(h.value.buffer, h.value.byteOffset, h.value.byteLength)).buffer
                        : h.value.slice(0),
                }))
                : undefined,
        };
    }
    async open(credential, contact, options = {}) {
        return BrowserClient.open({
            ...this.clientOptions(credential, contact),
            timeoutMs: options.timeoutMs,
            signal: options.signal,
        });
    }
}
