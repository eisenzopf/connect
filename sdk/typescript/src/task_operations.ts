import { ConnectError } from "./errors.js";
import {
    TASK_PROFILE,
    type TaskCommand,
    type TaskOutcome,
    type TaskRecord,
} from "./tasks.js";
import type { Operation, Outcome } from "./types.js";

/** Explicit task negotiation under backing credentials or narrow Native grants. */
export abstract class TaskOperations {
    private taskProfile = false;
    abstract request(
        operation: Operation,
        taskBinding?: string,
    ): Promise<Outcome>;
    protected observeOutcome(outcome: Outcome): Outcome {
        if (outcome.kind === "capabilities")
            this.taskProfile =
                outcome.data.extensions?.includes(TASK_PROFILE) ?? false;
        return outcome;
    }
    protected requireTaskProfile(
        operation: Operation,
        taskBinding?: string,
    ): void {
        if (taskBinding !== undefined) {
            if (
                typeof taskBinding !== "string" ||
                !/^[A-Za-z0-9_.:-]{1,128}$/.test(taskBinding) ||
                operation.op !== "task" ||
                operation.request.task_profile !== TASK_PROFILE ||
                operation.request.command.action !== "get" ||
                operation.request.command.task_id !== taskBinding
            )
                throw new ConnectError(
                    "invalid-request",
                    "Task bootstrap must read its exact bound task",
                );
            return;
        }
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
    /** Read one bound task without directory permission or authority issuance. */
    async negotiateTask(taskId: string): Promise<TaskRecord> {
        const outcome = await this.request(
            {
                op: "task",
                request: {
                    task_profile: TASK_PROFILE,
                    command: { action: "get", task_id: taskId },
                },
            },
            taskId,
        );
        if (
            outcome.kind !== "task" ||
            outcome.data.kind !== "record" ||
            outcome.data.data.id !== taskId
        )
            throw new ConnectError(
                "protocol",
                "Task bootstrap returned a different task",
            );
        this.taskProfile = true;
        return outcome.data.data;
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
    if (
        operation.op === "conversation_events" ||
        operation.op === "session_observations"
    ) {
        const wait = operation.query.wait_ms;
        return (
            baseMs +
            (Number.isSafeInteger(wait) && Number(wait) >= 0
                ? Math.min(Number(wait), 30000)
                : 0)
        );
    }
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
