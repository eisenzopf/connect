import { type Vp8Frame } from "./browser_rtp.js";
export interface OrderedVideoFrame {
    frame: Vp8Frame;
    timestampUs: number;
}
/** Reorder complete frames within one authenticated receive history. Sequence
 * rollback never creates a new history. In-order frames have no added delay. */
export declare class BrowserVideoOrder {
    private readonly ready;
    private pending;
    private last?;
    private timer?;
    private stopped;
    private counts;
    constructor(ready: (frames: OrderedVideoFrame[], expired: number) => void);
    get queuedFrames(): number;
    get queuedFragments(): number;
    get diagnostics(): {
        held: number;
        late: number;
        expired: number;
        gaps: number;
        queuedFrames: number;
        queuedFragments: number;
    };
    push(frame: Vp8Frame, timestampUs: number, now?: number): void;
    /** Capacity pressure may end a reorder wait, never renew a frame's age. */
    flush(now?: number, force?: boolean): void;
    clear(): void;
    close(): void;
}
