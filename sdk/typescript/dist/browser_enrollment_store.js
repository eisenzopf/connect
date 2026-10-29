import { ConnectError } from "./errors.js";
import { BrowserEnrollmentKey } from "./browser_enrollment_key.js";
import { BrowserEnrollmentSession, } from "./browser_enrollment.js";
import { validateSigningIdentity } from "./uctp_signing.js";
import { browserOrigin, id, invalid, json, normalizeOffer, normalizeRequest, origin, snapshot, } from "./browser_enrollment_contract.js";
/** Origin-local IndexedDB vault. Saves a nonextractable client key and complete
 * original intent before approval; it never persists an issued bearer. Each
 * name owns one immutable enrollment. Rotation uses a fresh name and key. */
export class BrowserEnrollmentStore {
    #database;
    constructor(database = "connect-enrollment-1") {
        browserOrigin();
        id(database);
        if (!globalThis.indexedDB)
            throw new ConnectError("unsupported-storage", "IndexedDB required for resumable browser enrollment");
        this.#database = database;
    }
    async #transaction(mode, work) {
        return new Promise((resolve, reject) => {
            let db, tx, value;
            let failure, settled = false;
            const fail = (error) => {
                failure = error;
                if (settled)
                    return;
                settled = true;
                clearTimeout(timer);
                try {
                    tx?.abort();
                }
                catch { }
                db?.close();
                reject(error);
            };
            const timer = setTimeout(() => fail(new ConnectError("timeout", "Enrollment storage timed out; retain the original state")), 5000);
            let opening;
            try {
                opening = indexedDB.open(this.#database, 1);
            }
            catch {
                fail(new ConnectError("storage-unavailable", "Enrollment storage unavailable"));
                return;
            }
            opening.onupgradeneeded = () => opening.result.createObjectStore("enrollments");
            opening.onerror = () => fail(new ConnectError("storage-unavailable", "Enrollment storage unavailable"));
            opening.onblocked = () => fail(new ConnectError("storage-unavailable", "Enrollment storage blocked"));
            opening.onsuccess = () => {
                db = opening.result;
                if (settled) {
                    db.close();
                    return;
                }
                db.onversionchange = () => db?.close();
                try {
                    tx = db.transaction("enrollments", mode, {
                        durability: "strict",
                    });
                    tx.oncomplete = () => {
                        if (settled)
                            return;
                        settled = true;
                        clearTimeout(timer);
                        db?.close();
                        resolve(value);
                    };
                    tx.onabort = () => fail(failure ??
                        new ConnectError("storage-unavailable", "Enrollment storage transaction refused"));
                    tx.onerror = () => fail(failure ??
                        new ConnectError("storage-unavailable", "Enrollment storage transaction refused"));
                    work(tx.objectStore("enrollments"), (result) => {
                        value = result;
                    }, fail);
                }
                catch {
                    fail(new ConnectError("storage-unavailable", "Enrollment storage unavailable"));
                }
            };
        });
    }
    async #read(name) {
        id(name);
        browserOrigin();
        const state = await this.#transaction("readonly", (store, done) => {
            const get = store.get(name);
            get.onsuccess = () => done(get.result);
        });
        if (!state)
            throw new ConnectError("not-found", "Saved enrollment unavailable");
        if (state.version !== 1 || origin(state.origin) !== state.origin)
            invalid();
        const request = normalizeRequest(state.request);
        if (request.browser_origin !== browserOrigin())
            invalid();
        const key = await BrowserEnrollmentKey.restore(state.key);
        if (key.publicKey !== request.public_key ||
            key.algorithm !== request.algorithm)
            invalid();
        return { ...state, request, key: key.snapshot() };
    }
    /** Complete original request and its client-owned key must be saved before
     * asking a trusted backend to approve. Name collisions never replace state. */
    async prepare(name, serverOrigin, input, key) {
        id(name);
        const contact = origin(serverOrigin);
        const request = normalizeRequest(input);
        const now = Math.floor(Date.now() / 1000);
        if (request.browser_origin !== browserOrigin() ||
            request.algorithm !== key.algorithm ||
            request.public_key !== key.publicKey ||
            request.challenge_expires_at <= now ||
            request.challenge_expires_at - now > 600 ||
            request.credential_expires_at <= now ||
            request.credential_expires_at - now > 3600)
            invalid();
        const state = {
            version: 1,
            origin: contact,
            request,
            key: key.snapshot(),
        };
        await this.#transaction("readwrite", (store, done, refuse) => {
            const get = store.get(name);
            get.onsuccess = () => {
                if (get.result !== undefined) {
                    refuse(new ConnectError("conflict", "Enrollment name already owns an original intent"));
                    return;
                }
                store.add(state, name);
                done();
            };
        });
        return snapshot(request);
    }
    async request(name) {
        return snapshot((await this.#read(name)).request);
    }
    /** Saves only an offer matching the complete saved original intent. A
     * different offer cannot replace an approved challenge, even after expiry. */
    async acceptOffer(name, input) {
        const offer = normalizeOffer(input);
        const state = await this.#read(name);
        const approved = await BrowserEnrollmentSession.restore({
            ...state,
            offer,
        });
        await this.#transaction("readwrite", (store, done, refuse) => {
            const get = store.get(name);
            get.onsuccess = () => {
                const current = get.result;
                if (!current ||
                    current.version !== 1 ||
                    current.origin !== state.origin ||
                    json(current.request) !== json(state.request) ||
                    (current.offer !== undefined &&
                        json(current.offer) !== json(offer))) {
                    refuse(new ConnectError("conflict", "Saved original enrollment changed"));
                    return;
                }
                store.put({ ...current, offer }, name);
                done();
            };
        });
        return approved;
    }
    async load(name) {
        return BrowserEnrollmentSession.restore(await this.#read(name));
    }
    /** Capture trusted contacts and saved profile names. Each physical attempt
     * recovers only its original approved profile under current authorization;
     * this never approves a grant or invents replacement intent after failure. */
    resolveProfiles(profiles) {
        if (!profiles ||
            typeof profiles !== "object" ||
            Array.isArray(profiles) ||
            !Object.keys(profiles).length ||
            Object.keys(profiles).some((p) => p !== "webtransport" && p !== "wss-webrtc"))
            invalid();
        const captured = new Map();
        for (const path of ["webtransport", "wss-webrtc"]) {
            const profile = profiles[path];
            if (profile === undefined)
                continue;
            id(profile.name);
            const contact = profile.contact;
            if (!contact ||
                typeof contact.url !== "string" ||
                contact.url.length > 2048)
                invalid();
            let url;
            try {
                url = new URL(contact.url);
            }
            catch {
                return invalid();
            }
            if (url.href !== contact.url ||
                url.protocol !==
                    (path === "webtransport" ? "https:" : "wss:") ||
                url.username ||
                url.password ||
                url.search ||
                url.hash ||
                url.port === "0")
                invalid();
            validateSigningIdentity(contact.server, "verify");
            const pins = contact.serverCertificateHashes;
            if (pins !== undefined &&
                (path !== "webtransport" ||
                    !Array.isArray(pins) ||
                    pins.length > 8))
                invalid();
            const copied = pins?.map((pin) => {
                if (pin.algorithm !== "sha-256" ||
                    !(pin.value instanceof ArrayBuffer ||
                        ArrayBuffer.isView(pin.value)) ||
                    pin.value.byteLength !== 32)
                    invalid();
                return {
                    algorithm: "sha-256",
                    value: ArrayBuffer.isView(pin.value)
                        ? Uint8Array.from(new Uint8Array(pin.value.buffer, pin.value.byteOffset, pin.value.byteLength)).buffer
                        : pin.value.slice(0),
                };
            });
            captured.set(path, {
                name: profile.name,
                contact: {
                    url: url.href,
                    server: { ...contact.server },
                    serverCertificateHashes: copied,
                },
            });
        }
        return async ({ transport, signal }) => {
            if (signal.aborted)
                throw new ConnectError("cancelled", "Enrollment resolution cancelled; retain original state");
            const profile = captured.get(transport);
            if (!profile)
                throw new ConnectError("unsupported-transport", "No saved enrollment for this transport");
            const saved = await this.load(profile.name);
            if (saved.offer.request.transport !== transport)
                invalid();
            const credential = await saved.claim({ signal });
            return saved.clientOptions(credential, profile.contact);
        };
    }
    /** Explicit local forgetting only; does not revoke any server credential. */
    async forget(name) {
        id(name);
        browserOrigin();
        return this.#transaction("readwrite", (store, done) => {
            store.delete(name);
            done();
        });
    }
}
