import { ConnectError } from "./errors.js";
import { BrowserEnrollmentKey } from "./browser_enrollment_key.js";
import {
    BrowserEnrollmentSession,
    type BrowserEnrollmentState,
} from "./browser_enrollment.js";
import {
    browserOrigin,
    id,
    invalid,
    json,
    normalizeOffer,
    normalizeRequest,
    origin,
    snapshot,
    type BrowserEnrollmentOffer,
    type BrowserEnrollmentRequest,
} from "./browser_enrollment_contract.js";

/** Origin-local IndexedDB vault. Saves a nonextractable client key and complete
 * original intent before approval; it never persists an issued bearer. Each
 * name owns one immutable enrollment. Rotation uses a fresh name and key. */
export class BrowserEnrollmentStore {
    #database: string;
    constructor(database = "connect-enrollment-1") {
        browserOrigin();
        id(database);
        if (!globalThis.indexedDB)
            throw new ConnectError(
                "unsupported-storage",
                "IndexedDB required for resumable browser enrollment",
            );
        this.#database = database;
    }
    async #transaction<T>(
        mode: IDBTransactionMode,
        work: (
            store: IDBObjectStore,
            complete: (value: T) => void,
            refuse: (error: ConnectError) => void,
        ) => void,
    ): Promise<T> {
        return new Promise<T>((resolve, reject) => {
            let db: IDBDatabase | undefined,
                tx: IDBTransaction | undefined,
                value: T;
            let failure: ConnectError | undefined,
                settled = false;
            const fail = (error: ConnectError) => {
                failure = error;
                if (settled) return;
                settled = true;
                clearTimeout(timer);
                try {
                    tx?.abort();
                } catch {}
                db?.close();
                reject(error);
            };
            const timer = setTimeout(
                () =>
                    fail(
                        new ConnectError(
                            "timeout",
                            "Enrollment storage timed out; retain the original state",
                        ),
                    ),
                5000,
            );
            let opening: IDBOpenDBRequest;
            try {
                opening = indexedDB.open(this.#database, 1);
            } catch {
                fail(
                    new ConnectError(
                        "storage-unavailable",
                        "Enrollment storage unavailable",
                    ),
                );
                return;
            }
            opening.onupgradeneeded = () =>
                opening.result.createObjectStore("enrollments");
            opening.onerror = () =>
                fail(
                    new ConnectError(
                        "storage-unavailable",
                        "Enrollment storage unavailable",
                    ),
                );
            opening.onblocked = () =>
                fail(
                    new ConnectError(
                        "storage-unavailable",
                        "Enrollment storage blocked",
                    ),
                );
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
                        if (settled) return;
                        settled = true;
                        clearTimeout(timer);
                        db?.close();
                        resolve(value);
                    };
                    tx.onabort = () =>
                        fail(
                            failure ??
                                new ConnectError(
                                    "storage-unavailable",
                                    "Enrollment storage transaction refused",
                                ),
                        );
                    tx.onerror = () =>
                        fail(
                            failure ??
                                new ConnectError(
                                    "storage-unavailable",
                                    "Enrollment storage transaction refused",
                                ),
                        );
                    work(
                        tx.objectStore("enrollments"),
                        (result) => {
                            value = result;
                        },
                        fail,
                    );
                } catch {
                    fail(
                        new ConnectError(
                            "storage-unavailable",
                            "Enrollment storage unavailable",
                        ),
                    );
                }
            };
        });
    }
    async #read(name: string): Promise<BrowserEnrollmentState> {
        id(name);
        browserOrigin();
        const state = await this.#transaction<
            BrowserEnrollmentState | undefined
        >("readonly", (store, done) => {
            const get = store.get(name);
            get.onsuccess = () =>
                done(get.result as BrowserEnrollmentState | undefined);
        });
        if (!state)
            throw new ConnectError("not-found", "Saved enrollment unavailable");
        if (state.version !== 1 || origin(state.origin) !== state.origin)
            invalid();
        const request = normalizeRequest(state.request);
        if (request.browser_origin !== browserOrigin()) invalid();
        const key = await BrowserEnrollmentKey.restore(state.key);
        if (
            key.publicKey !== request.public_key ||
            key.algorithm !== request.algorithm
        )
            invalid();
        return { ...state, request, key: key.snapshot() };
    }
    /** Complete original request and its client-owned key must be saved before
     * asking a trusted backend to approve. Name collisions never replace state. */
    async prepare(
        name: string,
        serverOrigin: string,
        input: BrowserEnrollmentRequest,
        key: BrowserEnrollmentKey,
    ): Promise<BrowserEnrollmentRequest> {
        id(name);
        const contact = origin(serverOrigin);
        const request = normalizeRequest(input);
        const now = Math.floor(Date.now() / 1000);
        if (
            request.browser_origin !== browserOrigin() ||
            request.algorithm !== key.algorithm ||
            request.public_key !== key.publicKey ||
            request.challenge_expires_at <= now ||
            request.challenge_expires_at - now > 600 ||
            request.credential_expires_at <= now ||
            request.credential_expires_at - now > 3600
        )
            invalid();
        const state: BrowserEnrollmentState = {
            version: 1,
            origin: contact,
            request,
            key: key.snapshot(),
        };
        await this.#transaction<void>("readwrite", (store, done, refuse) => {
            const get = store.get(name);
            get.onsuccess = () => {
                if (get.result !== undefined) {
                    refuse(
                        new ConnectError(
                            "conflict",
                            "Enrollment name already owns an original intent",
                        ),
                    );
                    return;
                }
                store.add(state, name);
                done();
            };
        });
        return snapshot(request);
    }
    async request(name: string): Promise<BrowserEnrollmentRequest> {
        return snapshot((await this.#read(name)).request);
    }
    /** Saves only an offer matching the complete saved original intent. A
     * different offer cannot replace an approved challenge, even after expiry. */
    async acceptOffer(
        name: string,
        input: BrowserEnrollmentOffer,
    ): Promise<BrowserEnrollmentSession> {
        const offer = normalizeOffer(input);
        const state = await this.#read(name);
        const approved = await BrowserEnrollmentSession.restore({
            ...state,
            offer,
        });
        await this.#transaction<void>("readwrite", (store, done, refuse) => {
            const get = store.get(name);
            get.onsuccess = () => {
                const current = get.result as
                    BrowserEnrollmentState | undefined;
                if (
                    !current ||
                    current.version !== 1 ||
                    current.origin !== state.origin ||
                    json(current.request) !== json(state.request) ||
                    (current.offer !== undefined &&
                        json(current.offer) !== json(offer))
                ) {
                    refuse(
                        new ConnectError(
                            "conflict",
                            "Saved original enrollment changed",
                        ),
                    );
                    return;
                }
                store.put({ ...current, offer }, name);
                done();
            };
        });
        return approved;
    }
    async load(name: string): Promise<BrowserEnrollmentSession> {
        return BrowserEnrollmentSession.restore(await this.#read(name));
    }
    /** Explicit local forgetting only; does not revoke any server credential. */
    async forget(name: string): Promise<void> {
        id(name);
        browserOrigin();
        return this.#transaction<void>("readwrite", (store, done) => {
            store.delete(name);
            done();
        });
    }
}
