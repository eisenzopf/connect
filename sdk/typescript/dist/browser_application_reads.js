import { ConnectError } from "./errors.js";
import { BrowserDeadline } from "./browser_deadline.js";
import { BrowserTasks } from "./browser_tasks.js";
import { canonicalJson, jsonObject, parseCanonicalJson, } from "./uctp_json.js";
import { identifier } from "./uctp_wire.js";
function fail(code = "protocol") {
    throw new ConnectError(code, "Application receipt read refused");
}
const closed = (v, names) => jsonObject(v) &&
    Object.keys(v).length === names.length &&
    names.every((k) => Object.hasOwn(v, k));
const natural = (v) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const graphic = (v, max) => typeof v === "string" &&
    v.length > 0 &&
    v.length <= max &&
    /^[\x21-\x7e]+$/.test(v);
export function applicationReadOrigin(value) {
    let url;
    try {
        url = new URL(value);
    }
    catch {
        return fail("invalid-request");
    }
    if (typeof value !== "string" ||
        value.length > 2048 ||
        url.protocol !== "https:" ||
        url.origin !== value ||
        url.username ||
        url.password ||
        url.port === "0")
        fail("invalid-request");
    return value;
}
/** Independently disposable HTTPS reads. No canonical credential or framing is reused. */
export class BrowserApplicationReads {
    resolve;
    identityId;
    deviceId;
    #cleanup = new BrowserTasks();
    #active = new Map();
    constructor(resolve, identityId, deviceId) {
        this.resolve = resolve;
        this.identityId = identityId;
        this.deviceId = deviceId;
    }
    async retire() {
        for (const read of this.#active.keys())
            read.abort();
        await Promise.all(this.#active.values());
        await this.#cleanup.join();
    }
    async receipts(cid, mid, options, physicalEpoch, current) {
        if (!identifier(cid) ||
            !identifier(mid) ||
            !options ||
            !Number.isFinite(options.timeoutMs) ||
            options.timeoutMs < 1 ||
            options.timeoutMs > 60000)
            fail("invalid-request");
        current();
        if (!this.resolve)
            fail("unsupported");
        if (this.#active.size >= 8)
            fail("conflict");
        const controller = new AbortController(), signal = AbortSignal.any([
            controller.signal,
            ...(options.signal ? [options.signal] : []),
        ]), budget = new BrowserDeadline(options.timeoutMs);
        let finished;
        this.#active.set(controller, new Promise((resolve) => {
            finished = resolve;
        }));
        const check = () => {
            current();
            budget.remaining();
            if (signal.aborted)
                fail("cancelled");
        };
        try {
            return await budget.run(async () => {
                check();
                const supplied = await this.resolve(Object.freeze({
                    signal,
                    timeoutMs: budget.remaining(),
                    identityId: this.identityId,
                    deviceId: this.deviceId,
                    physicalEpoch,
                }));
                check();
                if (!supplied || typeof supplied !== "object")
                    fail("invalid-request");
                const profile = {
                    url: supplied.url,
                    transport: supplied.transport,
                    identityId: supplied.identityId,
                    deviceId: supplied.deviceId,
                    browserOrigin: supplied.browserOrigin,
                    credential: supplied.credential,
                    expiresAt: supplied.expiresAt,
                };
                applicationReadOrigin(profile.url);
                if (profile.transport !== "http-json" ||
                    profile.identityId !== this.identityId ||
                    profile.deviceId !== this.deviceId ||
                    globalThis.isSecureContext !== true ||
                    profile.browserOrigin !== globalThis.location?.origin ||
                    !graphic(profile.credential, 256) ||
                    !Number.isSafeInteger(profile.expiresAt))
                    fail("invalid-request");
                const authorized = () => {
                    check();
                    if (profile.expiresAt <= Date.now() / 1000)
                        fail("authentication");
                };
                const observations = [], groups = new Set(), cursors = new Set();
                let after = null, total = 0;
                for (let page = 0; page < 64; page++) {
                    authorized();
                    const response = await this.#cleanup.track(fetch(profile.url + "/v1/operations", {
                        method: "POST",
                        headers: {
                            "Content-Type": "application/json",
                            Authorization: "Bearer " + profile.credential,
                        },
                        body: canonicalJson({
                            op: "message_receipts",
                            conversation_id: cid,
                            message_id: mid,
                            after,
                            limit: 100,
                        }),
                        credentials: "omit",
                        redirect: "error",
                        cache: "no-store",
                        referrerPolicy: "no-referrer",
                        signal,
                    }));
                    try {
                        authorized();
                        if (response.redirected ||
                            !response.body ||
                            (response.status >= 300 && response.status < 400))
                            fail();
                        const length = response.headers.get("content-length");
                        if (length !== null &&
                            (!/^\d+$/.test(length) ||
                                Number(length) > 2 * 1024 * 1024 - total))
                            fail();
                        const reader = response.body.getReader(), chunks = [];
                        let cancelling;
                        const cancelBody = () => {
                            cancelling ??= this.#cleanup.track(reader
                                .cancel()
                                .finally(() => reader.releaseLock()));
                        };
                        signal.addEventListener("abort", cancelBody, {
                            once: true,
                        });
                        if (signal.aborted)
                            cancelBody();
                        let size = 0;
                        try {
                            for (;;) {
                                const next = await reader.read();
                                authorized();
                                if (next.done)
                                    break;
                                size += next.value.length;
                                total += next.value.length;
                                if (total > 2 * 1024 * 1024)
                                    fail();
                                chunks.push(next.value);
                            }
                        }
                        finally {
                            signal.removeEventListener("abort", cancelBody);
                            cancelBody();
                        }
                        const bytes = new Uint8Array(size);
                        let offset = 0;
                        for (const chunk of chunks) {
                            bytes.set(chunk, offset);
                            offset += chunk.length;
                        }
                        const value = parseCanonicalJson(bytes, 2 * 1024 * 1024);
                        if (!closed(value, ["profile", "result"]) ||
                            !jsonObject(value) ||
                            value.profile !== "connect-control/1" ||
                            !jsonObject(value.result))
                            fail();
                        const result = value.result;
                        if (Object.hasOwn(result, "Err")) {
                            if (!closed(result, ["Err"]) ||
                                !jsonObject(result.Err) ||
                                !closed(result.Err, ["code", "message"]) ||
                                typeof result.Err.code !== "string" ||
                                typeof result.Err.message !== "string")
                                fail();
                            if ((response.status === 401 ||
                                response.status === 403) &&
                                ["unauthorized", "forbidden"].includes(result.Err.code))
                                throw new ConnectError(result.Err.code, "Application receipt authority refused", undefined, response.status);
                            fail();
                        }
                        if (response.status !== 200 ||
                            !closed(result, ["Ok"]) ||
                            !jsonObject(result.Ok) ||
                            !closed(result.Ok, ["kind", "data"]) ||
                            result.Ok.kind !== "message_receipts" ||
                            !jsonObject(result.Ok.data) ||
                            !closed(result.Ok.data, ["receipts", "next"]))
                            fail();
                        const data = result.Ok.data;
                        if (!Array.isArray(data.receipts) ||
                            data.receipts.length > 100)
                            fail();
                        for (const row of data.receipts) {
                            if (!jsonObject(row) ||
                                !closed(row, [
                                    "conversation_id",
                                    "message_id",
                                    "participant_id",
                                    "membership_epoch",
                                    "channel",
                                    "delivery_id",
                                    "state",
                                    "provenance",
                                    "observed_at",
                                ]) ||
                                row.conversation_id !== cid ||
                                row.message_id !== mid ||
                                !identifier(row.participant_id) ||
                                !natural(row.membership_epoch) ||
                                Number(row.membership_epoch) < 1 ||
                                !graphic(row.channel, 64) ||
                                !graphic(row.delivery_id, 512) ||
                                !natural(row.observed_at) ||
                                ![
                                    "queued",
                                    "accepted",
                                    "delivered",
                                    "read",
                                    "failed",
                                    "unknown",
                                ].includes(String(row.state)) ||
                                ![
                                    "journal",
                                    "client_reported",
                                    "provider_observation",
                                    "local_attempt",
                                ].includes(String(row.provenance)))
                                fail();
                            const group = canonicalJson([
                                row.participant_id,
                                row.membership_epoch,
                                row.channel,
                                row.delivery_id,
                            ]);
                            if (groups.has(group) ||
                                observations.length >= 4096)
                                fail();
                            groups.add(group);
                            observations.push(Object.freeze({
                                participantId: row.participant_id,
                                membershipEpoch: Number(row.membership_epoch),
                                channel: String(row.channel),
                                deliveryId: String(row.delivery_id),
                                state: row.state,
                                provenance: row.provenance,
                                observedAt: Number(row.observed_at),
                            }));
                        }
                        authorized();
                        if (data.next === null)
                            return Object.freeze(observations);
                        if (typeof data.next !== "string" ||
                            !data.next ||
                            data.next.length > 2048 ||
                            !/^[A-Za-z0-9._~-]+$/.test(data.next) ||
                            cursors.has(data.next))
                            fail();
                        after = data.next;
                        cursors.add(after);
                    }
                    finally {
                        if (response.body && !response.body.locked)
                            this.#cleanup.track(response.body.cancel());
                    }
                }
                return fail();
            }, signal);
        }
        catch (error) {
            if (error instanceof ConnectError)
                throw error;
            throw new ConnectError("transport-uncertain", "Application receipt service unavailable");
        }
        finally {
            controller.abort();
            this.#active.delete(controller);
            finished();
        }
    }
}
