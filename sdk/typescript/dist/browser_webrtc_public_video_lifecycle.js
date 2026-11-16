import { ConnectError } from "./errors.js";
import { jsonObject } from "./uctp_json.js";
import { recoverableReadinessFailure } from "./browser_webrtc_replacement.js";
import { WebRtcPublicVideoPhaseOwner } from "./browser_webrtc_public_video_phase.js";
import { observeWebRtcPublicVideoReadiness } from "./browser_webrtc_public_video_readiness.js";
const invalid = () => new ConnectError("protocol", "Original reserved public video workflow changed");
const cancelled = () => new ConnectError("cancelled", "Original reserved public video wait cancelled");
class CompletedPublicVideo {
    picture;
    track;
    loan;
    floor;
    constructor(picture, track) {
        this.picture = picture;
        this.track = track;
        this.floor = picture.presentedFloor;
    }
    borrow(track) {
        if (!this.picture || !this.track || this.track !== track || track.readyState !== "live" || this.loan)
            throw invalid();
        const picture = this.picture, token = {};
        this.loan = token;
        return Object.freeze({ element: picture.element, presentedFloor: this.floor,
            presented: (value) => {
                if (this.loan !== token || !Number.isSafeInteger(value) || value <= this.floor)
                    throw invalid();
                this.floor = value;
            },
            release: () => { if (this.loan === token)
                this.loan = undefined; }, });
    }
    release() {
        const picture = this.picture;
        this.picture = undefined;
        this.track = undefined;
        this.loan = undefined;
        if (!picture)
            return Promise.resolve();
        let failure;
        try {
            picture.element.pause();
        }
        catch (error) {
            failure = error;
        }
        try {
            picture.element.srcObject = null;
        }
        catch (error) {
            failure ??= error;
        }
        return failure === undefined ? Promise.resolve() : Promise.reject(failure);
    }
}
/** One original reserved B workflow. Verified operation.get observations are
 * supplied by the same signaling owner; plain caller phase/options never enter
 * this state machine. It creates no server authority, source cut or media grant. */
export class BrowserWebRtcPublicVideoLifecycle {
    original;
    peer;
    roles;
    budget;
    trackTask;
    phaseOwner;
    cancel = new AbortController();
    signal;
    running;
    observation;
    completed;
    stopped = false;
    constructor(original, peer, roles, scope, budget, signal, trackTask) {
        this.original = original;
        this.peer = peer;
        this.roles = roles;
        this.budget = budget;
        this.trackTask = trackTask;
        const context = original.reservedContext();
        this.phaseOwner = new WebRtcPublicVideoPhaseOwner(context.root, scope, context.reservation, context.challenge.target_generation, context.challenge.nonce);
        this.signal = AbortSignal.any([signal, this.cancel.signal]);
    }
    check() {
        if (this.stopped || this.signal.aborted)
            throw cancelled();
        this.budget.remaining();
        this.phaseOwner.check();
        if (!this.peer || !this.roles || !this.phaseOwner.exactLocalPeer(this.peer) ||
            ["closed", "failed"].includes(this.peer.connectionState))
            throw invalid();
    }
    options() { this.check(); return { timeoutMs: Math.min(1000, Math.max(1, Math.floor(this.budget.remaining() / 2))), signal: this.signal }; }
    run(work) {
        this.check();
        return this.budget.run(() => this.trackTask(work()), this.signal);
    }
    async delay() {
        await this.run(() => new Promise(resolve => setTimeout(resolve, Math.min(20, this.budget.remaining()))));
    }
    async read() {
        const value = await this.run(() => this.original.reservedStatus(this.options()));
        this.check();
        if (["failed", "cancelled"].includes(String(value.state)))
            throw new ConnectError("refused", "Original reserved replacement did not complete");
        if (!["accepted", "preparing", "ready", "committing", "unknown", "interrupted", "completed"].includes(String(value.state)))
            throw invalid();
        return value;
    }
    phase(value, required) {
        const raw = value.webrtc_public_video_phase;
        if (raw === undefined) {
            if (required || value.state === "completed")
                throw invalid();
            return undefined;
        }
        if (!jsonObject(raw))
            throw invalid();
        const receipt = value.state === "completed" ? this.original.reservedCompleted(value) : undefined;
        const context = this.original.reservedContext();
        const phase = this.phaseOwner.observe(raw, { recordId: context.root.recordId, operationId: context.root.operationId,
            targetConnectionId: context.root.targetConnectionId, targetGeneration: context.challenge.target_generation, rootCompleted: receipt !== undefined });
        if (phase.phase === "discarded")
            throw new ConnectError("refused", "Original reserved public video was discarded");
        return phase;
    }
    complete(observePrivate, privateCurrent) {
        if (this.running)
            return this.running;
        let resolve, reject;
        const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
        this.running = promise;
        void this.perform(observePrivate, privateCurrent).then(resolve, reject);
        return promise;
    }
    async perform(observePrivate, privateCurrent) {
        let adopted = false, terminal;
        try {
            let current, phase;
            for (;;) {
                try {
                    current = await this.read();
                }
                catch (error) {
                    if (!recoverableReadinessFailure(error))
                        throw error;
                    await this.delay();
                    continue;
                }
                phase = this.phase(current, false);
                if (phase) {
                    if (phase.phase !== "negotiated-never-fed")
                        throw invalid();
                    break;
                }
                await this.delay();
            }
            const evidence = await this.run(observePrivate);
            this.check();
            if (!privateCurrent())
                throw invalid();
            let attempts = 0, replay = false, next = 0;
            const sendPrivate = async (original) => {
                this.check();
                if (!privateCurrent())
                    throw invalid();
                attempts++;
                try {
                    await this.run(() => this.original.reservedPrivateReady(evidence, this.options(), original));
                    replay = false;
                }
                catch (error) {
                    if (!recoverableReadinessFailure(error))
                        throw error;
                    replay = true;
                    next = performance.now() + Math.min(1000, 50 * 2 ** (attempts - 1));
                }
            };
            await sendPrivate();
            for (;;) {
                try {
                    current = await this.read();
                }
                catch (error) {
                    if (!recoverableReadinessFailure(error))
                        throw error;
                    await this.delay();
                    continue;
                }
                phase = this.phase(current, true);
                if (phase.phase === "cut-bound-post-f")
                    break;
                if (phase.phase !== "negotiated-never-fed")
                    throw invalid();
                if (replay && attempts < 8 && performance.now() >= next && current.state === "preparing" && current.native_replacement_prepared === true)
                    await sendPrivate(current);
                else
                    await this.delay();
            }
            this.observation = await observeWebRtcPublicVideoReadiness(this.phaseOwner, this.peer, this.roles, this.budget, this.signal, this.trackTask);
            this.check();
            const marker = this.observation.readiness;
            attempts = 0;
            replay = false;
            next = 0;
            let decodedReady = false;
            const sendPublic = async (original) => {
                this.check();
                this.observation.assertCurrent();
                attempts++;
                try {
                    await this.run(() => this.original.reservedPublicReady(marker, this.options(), original));
                    replay = false;
                }
                catch (error) {
                    if (!recoverableReadinessFailure(error))
                        throw error;
                    replay = true;
                    next = performance.now() + Math.min(1000, 50 * 2 ** (attempts - 1));
                }
            };
            await sendPublic();
            for (;;) {
                try {
                    current = await this.read();
                }
                catch (error) {
                    if (!recoverableReadinessFailure(error))
                        throw error;
                    await this.delay();
                    continue;
                }
                phase = this.phase(current, true);
                if (phase.phase === "activated") {
                    if (current.state !== "completed")
                        throw invalid();
                    const receipt = this.original.reservedCompleted(current);
                    this.check();
                    const binding = this.roles.binding("public");
                    if (!binding)
                        throw invalid();
                    const picture = this.observation.take(phase, binding.track);
                    if (!picture)
                        throw invalid();
                    this.completed = new CompletedPublicVideo(picture, binding.track);
                    adopted = true;
                    this.observation = undefined;
                    this.peer = undefined;
                    this.roles = undefined;
                    return receipt;
                }
                if (current.state === "completed")
                    throw invalid();
                if (phase.phase === "decoded-ready") {
                    decodedReady = true;
                    replay = false;
                }
                else if (phase.phase !== "cut-bound-post-f")
                    throw invalid();
                if (!decodedReady && replay && attempts < 8 && performance.now() >= next && current.state === "preparing" && current.native_replacement_prepared === true)
                    await sendPublic(current);
                else
                    await this.delay();
            }
        }
        catch (error) {
            terminal = error;
            throw error;
        }
        finally {
            if (!adopted) {
                const observation = this.observation;
                this.observation = undefined;
                this.phaseOwner.close();
                if (observation)
                    try {
                        await this.trackTask(observation.release(), true);
                    }
                    catch (error) {
                        if (terminal === undefined)
                            throw error;
                    }
                this.peer = undefined;
                this.roles = undefined;
            }
        }
    }
    /** No original operation budget check: this lease was transferred while
     * actual Completed+Activated and that deadline were valid. */
    assertActivated() {
        if (this.stopped || !this.completed)
            throw new ConnectError("unsupported-media-profile", "Reserved B has no original activated decoded lease");
        this.original.reservedContext();
    }
    borrow(track) {
        this.assertActivated();
        return this.completed.borrow(track);
    }
    close() {
        if (this.stopped)
            return Promise.resolve();
        this.stopped = true;
        this.cancel.abort();
        this.phaseOwner.close();
        const pending = this.observation, completed = this.completed;
        this.observation = undefined;
        this.completed = undefined;
        this.peer = undefined;
        this.roles = undefined;
        return Promise.all([pending?.release(), completed?.release()]).then(() => { });
    }
}
