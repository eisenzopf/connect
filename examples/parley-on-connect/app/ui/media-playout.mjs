// Own a bounded set of decoded frames until their original presentation time.
// Later arrivals must never postpone the head frame's timer.
export class VideoPlayout {
    #frames = []; #timer; #head; #closed = false;
    #now; #paint; #drop; #setTimer; #clearTimer;
    constructor({ now, paint, drop, setTimer = (fn, ms) => setTimeout(fn, ms), clearTimer = timer => clearTimeout(timer) }) {
        this.#now = now; this.#paint = paint; this.#drop = drop;
        this.#setTimer = setTimer; this.#clearTimer = clearTimer;
    }
    get size() { return this.#frames.length; }
    enqueue(frame, desired) {
        if (this.#closed) return;
        const now = this.#now();
        if (!Number.isFinite(desired) || desired < now - 0.02 || desired > now + 0.2) {
            this.#drop("timing"); return;
        }
        if (this.#frames.length >= 6) { this.#drop("capacity"); return; }
        this.#frames.push({ frame: frame.clone(), desired });
        this.#frames.sort((a, b) => a.desired - b.desired);
        this.#arm();
    }
    #arm() {
        const head = this.#frames[0];
        if (head === this.#head) return;
        if (this.#timer !== undefined) this.#clearTimer(this.#timer);
        this.#timer = undefined; this.#head = head;
        if (!head || this.#closed) return;
        this.#timer = this.#setTimer(() => {
            this.#timer = undefined; this.#head = undefined;
            // The audio clock can advance differently from the timer clock.
            // An early wake retains the same target; it never paints early.
            if (this.#now() < head.desired) { this.#arm(); return; }
            this.#frames.shift();
            try {
                if (this.#now() - head.desired < 0.05) this.#paint(head.frame);
                else this.#drop("late");
            } finally { head.frame.close(); this.#arm(); }
        }, Math.max(1, Math.ceil((head.desired - this.#now()) * 1000)));
    }
    clear() {
        if (this.#timer !== undefined) this.#clearTimer(this.#timer);
        this.#timer = undefined; this.#head = undefined;
        for (const { frame } of this.#frames) frame.close();
        this.#frames.length = 0;
    }
    close() { this.#closed = true; this.clear(); }
}
