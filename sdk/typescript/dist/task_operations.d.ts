import { type TaskCommand, type TaskOutcome, type TaskRecord } from "./tasks.js";
import type { Operation, Outcome } from "./types.js";
/** Explicit task negotiation under backing credentials or narrow Native grants. */
export declare abstract class TaskOperations {
    private taskProfile;
    abstract request(operation: Operation, taskBinding?: string): Promise<Outcome>;
    protected observeOutcome(outcome: Outcome): Outcome;
    protected requireTaskProfile(operation: Operation, taskBinding?: string): void;
    /** Discovery never expands the current credential, task or recipient ceiling. */
    negotiateTasks(address: string, conversationId?: string | null): Promise<void>;
    /** Read one bound task without directory permission or authority issuance. */
    negotiateTask(taskId: string): Promise<TaskRecord>;
    /** Persist mutation IDs and inputs across uncertainty. Pending is not completion. */
    task(command: TaskCommand): Promise<TaskOutcome>;
}
/** Valid waits receive their bounded server wait in addition to the exchange budget. */
export declare function exchangeTimeout(baseMs: number, operation: Operation): number;
