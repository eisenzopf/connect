// Own a bounded set of decoded frames until their original presentation time.
// Later arrivals must never postpone the head frame's timer.
export class VideoPlayout {
    #frames = []; #timer; #head; #closed = false; #draining = false;
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
        if (this.#draining) return;
        const head = this.#frames[0];
        if (head === this.#head) return;
        if (this.#timer !== undefined) this.#clearTimer(this.#timer);
        this.#timer = undefined; this.#head = head;
        if (!head || this.#closed) return;
        this.#timer = this.#setTimer(() => {
            this.#timer = undefined; this.#head = undefined;
            // One overdue wake can make several frames due. Release each in
            // this bounded queue before returning to unrelated timer callbacks.
            // Recheck the audio clock for each original target: an early wake
            // retains future frames, and a delayed wake never refreshes age.
            this.#draining = true;
            let remaining = this.#frames.length;
            try {
                while (!this.#closed && this.#frames.length && remaining-- > 0) {
                    const due = this.#frames[0], now = this.#now();
                    if (now < due.desired) break;
                    this.#frames.shift();
                    try {
                        if (now - due.desired < 0.05) this.#paint(due.frame);
                        else this.#drop("late");
                    } finally { due.frame.close(); }
                }
            } finally { this.#draining = false; this.#arm(); }
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
