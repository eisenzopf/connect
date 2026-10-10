import { ConnectError } from "./errors.js";
import { TASK_PROFILE, type TaskCommand, type TaskOutcome } from "./tasks.js";
import type { Operation, Outcome } from "./types.js";

/** Backing-credential task control. Narrow model grants use the protected adapter. */
export abstract class TaskOperations {
    private taskProfile = false;
    abstract request(operation: Operation): Promise<Outcome>;
    protected observeOutcome(outcome: Outcome): Outcome {
        if (outcome.kind === "capabilities")
            this.taskProfile =
                outcome.data.extensions?.includes(TASK_PROFILE) ?? false;
        return outcome;
    }
    protected requireTaskProfile(operation: Operation): void {
        if (operation.op === "task" && !this.taskProfile)
            throw new ConnectError(
                "unsupported-profile",
                "Discover connect-task/1 through capabilities before task requests",
            );
    }
    /** Discovery never expands the current credential, task or recipient ceiling. */
    async negotiateTasks(
        address: string,
        conversationId: string | null = null,
    ): Promise<void> {
        const outcome = await this.request({
            op: "capabilities",
            address,
            conversation_id: conversationId,
        });
        this.observeOutcome(outcome);
        if (!this.taskProfile)
            throw new ConnectError(
                "unsupported-profile",
                "Server does not advertise connect-task/1",
            );
    }
    /** Persist mutation IDs and inputs across uncertainty. Pending is not completion. */
    async task(command: TaskCommand): Promise<TaskOutcome> {
        const operation: Operation = {
            op: "task",
            request: { task_profile: TASK_PROFILE, command },
        };
        this.requireTaskProfile(operation);
        const result = await this.request(operation);
        if (result.kind !== "task")
            throw new ConnectError("protocol", "Unexpected task outcome");
        return result.data;
    }
}
/** Valid waits receive their bounded server wait in addition to the exchange budget. */
export function exchangeTimeout(baseMs: number, operation: Operation): number {
    if (operation.op !== "task") return baseMs;
    const command = operation.request.command;
    if (command.action !== "inbox" && command.action !== "wait") return baseMs;
    return (
        baseMs +
        (Number.isSafeInteger(command.wait_ms) && command.wait_ms >= 0
            ? Math.min(command.wait_ms, 10000)
            : 0)
    );
}
