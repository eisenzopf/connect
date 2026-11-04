import { MEDIA_LIMITS, type Vp8Frame } from "./browser_rtp.js";

export interface OrderedVideoFrame {
    frame: Vp8Frame;
    timestampUs: number;
}

/** Reorder complete frames within one authenticated receive history. Sequence
 * rollback never creates a new history. In-order frames have no added delay. */
export class BrowserVideoOrder {
    private pending: OrderedVideoFrame[] = [];
    private last?: Pick<Vp8Frame, "ssrc" | "timestamp" | "lastSequence">;
    private timer?: ReturnType<typeof setTimeout>;
    private stopped = false;
    private counts = { held: 0, late: 0, expired: 0, gaps: 0 };
    constructor(
        private readonly ready: (
            frames: OrderedVideoFrame[],
            expired: number,
        ) => void,
    ) {}
    get queuedFrames(): number {
        return this.pending.length;
    }
    get queuedFragments(): number {
        return this.pending.reduce(
            (n, x) =>
                n +
                ((x.frame.lastSequence - x.frame.firstSequence + 65536) &
                    65535) +
                1,
            0,
        );
    }
    get diagnostics() {
        return {
            ...this.counts,
            queuedFrames: this.queuedFrames,
            queuedFragments: this.queuedFragments,
        };
    }
    push(frame: Vp8Frame, timestampUs: number, now = performance.now()): void {
        if (this.stopped) return;
        if (
            !Number.isFinite(frame.arrived) ||
            frame.arrived > now ||
            now - frame.arrived >= MEDIA_LIMITS.assemblyMs
        ) {
            this.counts.expired++;
            this.ready([], 1);
            return;
        }
        // SSRC authority was checked by the enclosing signed epoch owner.
        // A different source cannot inherit another source's pending frames.
        if (this.last && this.last.ssrc !== frame.ssrc) this.clear();
        if (
            (this.last && ((frame.timestamp - this.last.timestamp) | 0) <= 0) ||
            this.pending.some(
                (x) =>
                    x.frame.ssrc === frame.ssrc &&
                    x.frame.timestamp === frame.timestamp,
            )
        ) {
            this.counts.late++;
            return;
        }
        const fragments =
            ((frame.lastSequence - frame.firstSequence + 65536) & 65535) + 1;
        if (fragments > MEDIA_LIMITS.fragments) return;
        if (
            this.queuedFrames >= MEDIA_LIMITS.frames ||
            this.queuedFragments + fragments > MEDIA_LIMITS.fragments
        )
            this.flush(now, true);
        this.pending.push({ frame, timestampUs });
        this.pending.sort(
            (a, b) => (a.frame.timestamp - b.frame.timestamp) | 0,
        );
        this.flush(now);
        if (this.pending.length) this.counts.held++;
    }
    /** Capacity pressure may end a reorder wait, never renew a frame's age. */
    flush(now = performance.now(), force = false): void {
        clearTimeout(this.timer);
        this.timer = undefined;
        if (this.stopped) return;
        const ready: OrderedVideoFrame[] = [];
        let expired = 0;
        while (this.pending.length) {
            const head = this.pending[0],
                frame = head.frame;
            if (now - frame.arrived >= MEDIA_LIMITS.assemblyMs) {
                this.pending.shift();
                this.counts.expired++;
                expired++;
                continue;
            }
            const gap =
                this.last &&
                frame.firstSequence !== ((this.last.lastSequence + 1) & 65535);
            if (gap && !force && now - frame.arrived < 20) break;
            this.pending.shift();
            if (gap) this.counts.gaps++;
            this.last = {
                ssrc: frame.ssrc,
                timestamp: frame.timestamp,
                lastSequence: frame.lastSequence,
            };
            ready.push(head);
        }
        if (ready.length || expired) this.ready(ready, expired);
        if (this.pending.length && !this.stopped) {
            const delay = Math.max(
                0,
                this.pending[0].frame.arrived + 20 - performance.now(),
            );
            this.timer = setTimeout(() => this.flush(), delay);
        }
    }
    clear(): void {
        clearTimeout(this.timer);
        this.timer = undefined;
        this.pending = [];
        this.last = undefined;
    }
    close(): void {
        this.clear();
        this.stopped = true;
    }
}
