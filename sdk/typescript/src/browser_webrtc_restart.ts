import { BrowserClient, type RequestOptions } from "./browser_client.js";
import { BrowserDeadline } from "./browser_deadline.js";
import { BrowserTasks } from "./browser_tasks.js";
import { ConnectError } from "./errors.js";
import {
    canonicalJson,
    jsonObject,
    parseCanonicalJson,
    type JsonObject,
} from "./uctp_json.js";
import { identifier, type Scope } from "./uctp_wire.js";

export interface WebRtcIceRestartReceipt {
    readonly operationId: string;
    readonly iceGeneration: number;
    readonly streamRevision: number;
    readonly connectionGeneration: number;
}
interface Attempt {
    scope: Scope;
    expected: number;
    preparation: Promise<JsonObject>;
    payload?: JsonObject;
    submitted: boolean;
    answer?: string;
    application?: Promise<void>;
    receipt?: WebRtcIceRestartReceipt;
}
const copy = (value: JsonObject) =>
    parseCanonicalJson(
        new TextEncoder().encode(canonicalJson(value)),
    ) as JsonObject;
const unresolved = () =>
    new ConnectError(
        "unresolved-operation",
        "Retain the original ICE operation and peer; recover its original answer/status",
    );
const validSdp = (value: unknown): value is string =>
    typeof value === "string" &&
    value.startsWith("v=0\r\n") &&
    !value.includes("\0") &&
    new TextEncoder().encode(value).byteLength <= 512 * 1024;
const exact = (value: JsonObject, keys: string[]) =>
    Object.keys(value).length === keys.length &&
    keys.every((k) => Object.hasOwn(value, k));

/** Internal same-peer owner. Retries reuse both the gathered offer and the
 * original local description application promises. Status never creates RTC. */
export class BrowserIceRestart {
    private readonly original: Scope;
    private readonly tasks = new BrowserTasks();
    private current?: Attempt;
    private running?: Promise<WebRtcIceRestartReceipt>;
    private generation = 0;
    constructor(
        private readonly client: BrowserClient,
        private readonly peer: RTCPeerConnection,
        original: Scope,
        private readonly lifetime: AbortSignal,
        private readonly check: () => void,
        private readonly fail: (error: Error) => void,
    ) {
        this.original = { ...original };
    }
    get saved():
        | { scope: Scope; payload?: JsonObject; submissionAttempted: boolean }
        | undefined {
        const a = this.current;
        return a
            ? {
                  scope: { ...a.scope },
                  payload: a.payload ? copy(a.payload) : undefined,
                  submissionAttempted: a.submitted,
              }
            : undefined;
    }
    restart(
        operationId: string,
        revision: number,
        expected: number,
        options: RequestOptions,
    ): Promise<WebRtcIceRestartReceipt> {
        this.check();
        const a = this.current;
        if (
            !identifier(operationId) ||
            operationId === this.original.operation_id ||
            !Number.isSafeInteger(revision) ||
            revision < 1 ||
            !Number.isSafeInteger(expected) ||
            expected < 0 ||
            expected >= Number.MAX_SAFE_INTEGER
        )
            throw new ConnectError(
                "invalid-request",
                "Explicit original scope, media revision and ICE counter required",
            );
        const timeout = options.timeoutMs ?? 10000;
        if (!Number.isFinite(timeout) || timeout < 1 || timeout > 30000)
            throw new ConnectError(
                "invalid-request",
                "Use a bounded ICE deadline up to30seconds",
            );
        if (options.signal?.aborted)
            return Promise.reject(
                new ConnectError("cancelled", "ICE wait cancelled"),
            );
        if (this.running) {
            if (
                a?.scope.operation_id === operationId &&
                a.scope.expected_revision === revision &&
                a.expected === expected
            )
                return this.running;
            throw new ConnectError(
                "conflict",
                "Original ICE preparation is still owned",
            );
        }
        if (a?.scope.operation_id === operationId) {
            if (
                a.scope.expected_revision !== revision ||
                a.expected !== expected
            )
                throw new ConnectError(
                    "conflict",
                    "Original ICE intent is immutable",
                );
            if (a.receipt) return Promise.resolve(a.receipt);
        } else {
            if (
                (a && !a.receipt) ||
                expected !== this.generation ||
                this.peer.signalingState !== "stable"
            )
                throw new ConnectError(
                    "conflict",
                    "Recover the original ICE attempt before starting another",
                );
            const next: Attempt = {
                scope: {
                    ...this.original,
                    operation_id: operationId,
                    expected_revision: revision,
                },
                expected,
                submitted: false,
                preparation: undefined!,
            };
            this.current = next;
            // Attribution is installed before browser IO. This promise is never
            // replaced on timeout, cancellation or an uncertain request.
            next.preparation = this.tasks.track(
                this.prepare(next).catch((error) => {
                    this.fail(error instanceof Error ? error : unresolved());
                    throw error;
                }),
            );
        }
        const active = this.run(
            this.current!,
            new BrowserDeadline(timeout),
            options.signal,
        ).catch((error) => {
            if (
                error instanceof ConnectError &&
                ["protocol", "forbidden", "operation-failed"].includes(
                    error.code,
                )
            )
                this.fail(error);
            throw error;
        });
        this.running = active;
        void active
            .finally(() => {
                if (this.running === active) this.running = undefined;
            })
            .catch(() => {});
        return active;
    }
    private async prepare(a: Attempt): Promise<JsonObject> {
        this.check();
        const offer = await this.peer.createOffer({ iceRestart: true });
        this.check();
        await this.peer.setLocalDescription(offer);
        await this.gather();
        this.check();
        const sdp = this.peer.localDescription?.sdp;
        if (!validSdp(sdp))
            throw new ConnectError(
                "protocol",
                "Invalid complete original ICE offer",
            );
        a.payload = {
            action: "ice-restart",
            details: {
                expected_ice_generation: a.expected,
                setup: { kind: "websocket+webrtc", sdp },
            },
        };
        if (
            new TextEncoder().encode(canonicalJson(a.payload)).byteLength >
            544 * 1024
        )
            throw new ConnectError(
                "invalid-request",
                "ICE intent exceeds its bounded profile",
            );
        return a.payload;
    }
    private gather(): Promise<void> {
        if (this.lifetime.aborted) return Promise.reject(unresolved());
        if (this.peer.iceGatheringState === "complete")
            return Promise.resolve();
        return new Promise((resolve, reject) => {
            const cleanup = () => {
                this.peer.removeEventListener(
                    "icegatheringstatechange",
                    changed,
                );
                this.lifetime.removeEventListener("abort", aborted);
            };
            const changed = () => {
                if (this.peer.iceGatheringState === "complete") {
                    cleanup();
                    resolve();
                }
            };
            const aborted = () => {
                cleanup();
                reject(unresolved());
            };
            this.peer.addEventListener("icegatheringstatechange", changed);
            this.lifetime.addEventListener("abort", aborted, { once: true });
            if (this.lifetime.aborted) aborted();
            else changed();
        });
    }
    private async run(
        a: Attempt,
        budget: BrowserDeadline,
        caller?: AbortSignal,
    ): Promise<WebRtcIceRestartReceipt> {
        const signal = AbortSignal.any([
            this.lifetime,
            ...(caller ? [caller] : []),
        ]);
        const wait = <T>(work: Promise<T>) => budget.run(() => work, signal);
        const payload = await wait(a.preparation);
        this.check();
        if (!a.application) {
            a.submitted = true;
            const frames = await wait(
                this.client.request(
                    "connection.update",
                    copy(payload),
                    { ...a.scope },
                    { timeoutMs: budget.remaining(), signal },
                ),
            );
            this.check();
            if (
                frames.some(
                    (f) =>
                        f.type === "operation.updated" &&
                        f.payload.operation_id === a.scope.operation_id &&
                        ["failed", "cancelled"].includes(
                            String(f.payload.state),
                        ),
                )
            )
                throw new ConnectError(
                    "operation-failed",
                    "Original ICE attempt was refused",
                );
            const acknowledgements = frames.filter((f) => f.type === "ack");
            if (acknowledgements.length !== 1) throw unresolved();
            const ack = acknowledgements[0];
            if (
                ["cid", "sid", "connid", "operation_id"].some(
                    (k) => ack[k] !== a.scope[k as keyof Scope],
                )
            )
                throw new ConnectError(
                    "protocol",
                    "Foreign ICE acknowledgment",
                );
            const details = ack.payload.details;
            if (
                !jsonObject(details) ||
                details.reply_complete !== true ||
                !jsonObject(details.ice_restart)
            )
                throw unresolved();
            if (!exact(details, ["reply_complete", "ice_restart"]))
                throw new ConnectError(
                    "protocol",
                    "Invalid ICE acknowledgment fields",
                );
            const answer = details.ice_restart,
                setup = answer.setup;
            if (
                !exact(answer, ["proposed_ice_generation", "setup"]) ||
                answer.proposed_ice_generation !== a.expected + 1 ||
                !jsonObject(setup) ||
                !exact(setup, ["kind", "sdp"]) ||
                setup.kind !== "websocket+webrtc" ||
                !validSdp(setup.sdp)
            )
                throw new ConnectError(
                    "protocol",
                    "Invalid original ICE answer",
                );
            if (a.answer !== undefined && a.answer !== setup.sdp)
                throw new ConnectError(
                    "protocol",
                    "Original ICE answer changed",
                );
            a.answer = setup.sdp;
            a.application = this.tasks.track(
                this.peer.setRemoteDescription({
                    type: "answer",
                    sdp: a.answer,
                }),
            );
        }
        await wait(a.application);
        this.check();
        if (
            !jsonObject(payload.details) ||
            !jsonObject(payload.details.setup) ||
            this.peer.localDescription?.sdp !== payload.details.setup.sdp ||
            this.peer.remoteDescription?.sdp !== a.answer ||
            this.peer.signalingState !== "stable"
        )
            throw new ConnectError(
                "protocol",
                "Original ICE peer descriptions changed",
            );
        for (;;) {
            const state = await wait(
                this.client.operationStatus(
                    String(a.scope.cid),
                    String(a.scope.operation_id),
                    { timeoutMs: budget.remaining(), signal },
                ),
            );
            this.check();
            if (state.operation_id !== a.scope.operation_id)
                throw new ConnectError(
                    "protocol",
                    "Foreign ICE operation status",
                );
            if (state.state === "completed") {
                const result = state.result;
                if (
                    !jsonObject(result) ||
                    result.action !== "ice-restart" ||
                    result.connection_id !== a.scope.connid ||
                    result.ice_generation !== a.expected + 1 ||
                    result.stream_revision !== a.scope.expected_revision ||
                    result.resources_changed !== false ||
                    !Number.isSafeInteger(result.connection_generation) ||
                    Number(result.connection_generation) < 1
                )
                    throw new ConnectError(
                        "protocol",
                        "Invalid captured ICE completion",
                    );
                this.generation = a.expected + 1;
                a.receipt = Object.freeze({
                    operationId: String(a.scope.operation_id),
                    iceGeneration: this.generation,
                    streamRevision: Number(a.scope.expected_revision),
                    connectionGeneration: Number(result.connection_generation),
                });
                return a.receipt;
            }
            if (state.state === "failed" || state.state === "cancelled")
                throw new ConnectError(
                    "operation-failed",
                    "Original ICE attempt was refused",
                );
            if (state.state !== "preparing" && state.state !== "accepted")
                throw unresolved();
            await wait(new Promise<void>((resolve) => setTimeout(resolve, 25)));
        }
    }
    async shutdown(): Promise<void> {
        await this.tasks.join();
    }
}
