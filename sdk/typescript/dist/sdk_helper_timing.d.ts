import type { JsonObject } from "./uctp_json.js";
export interface HelperTiming {
    readonly sequence: number;
    readonly createdAt: number;
    dispatchedAt?: number;
}
interface TimedJob {
    action: JsonObject;
    expires: number;
    sent: boolean;
    timing?: HelperTiming;
}
type Phase = "queued" | "dispatched" | "reply" | "cancelled" | "fenced" | "expired-queued" | "aborted-queued";
export declare class HelperTimingLog {
    private readonly enabled;
    private readonly owner;
    private sequence;
    private events;
    constructor(configPath: string);
    capture(createdAt: number): HelperTiming | undefined;
    record(phase: Phase, job: TimedJob, pid: number | undefined, queued: number, reason?: string): void;
}
export {};
