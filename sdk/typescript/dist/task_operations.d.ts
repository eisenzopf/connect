import { type TaskCommand, type TaskOutcome } from "./tasks.js";
import type { Operation, Outcome } from "./types.js";
/** Backing-credential task control. Narrow model grants use the protected adapter. */
export declare abstract class TaskOperations {
    private taskProfile;
    abstract request(operation: Operation): Promise<Outcome>;
    protected observeOutcome(outcome: Outcome): Outcome;
    protected requireTaskProfile(operation: Operation): void;
    /** Discovery never expands the current credential, task or recipient ceiling. */
    negotiateTasks(address: string, conversationId?: string | null): Promise<void>;
    /** Persist mutation IDs and inputs across uncertainty. Pending is not completion. */
    task(command: TaskCommand): Promise<TaskOutcome>;
}
/** Valid waits receive their bounded server wait in addition to the exchange budget. */
export declare function exchangeTimeout(baseMs: number, operation: Operation): number;
