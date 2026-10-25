// Compile-only contract outline. Calls are targets for G6-060/061/062/063.
import {
    ConnectError,
    type ParticipantClient,
    type BrowserClient,
    type NodeClient,
    type Conversation,
    type Session,
    type Connection,
    type Participant,
    type EndpointId,
    type OperationId,
    type MessageId,
    type OperationReference,
    type MoveResult,
    type WaitOptions,
    type Cursor,
    type Json,
} from "./contract.js";
export interface SavedJourney {
    // Already committed by the application's intent adapter before any call.
    readonly send: OperationId;
    readonly message: MessageId;
    readonly consult: OperationId;
    readonly handoff: OperationId;
    readonly move: OperationId;
    readonly cancel: OperationId;
    readonly accept: OperationId;
    readonly reject: OperationId;
    readonly returnConsult: OperationId;
    readonly cursor?: Cursor;
}
export interface Context {
    readonly conversation: Conversation;
    readonly session: Session;
    readonly control: Connection;
    readonly retained: Connection;
    readonly owner: Participant;
    readonly assistant: Participant;
    readonly phone: EndpointId;
}
export async function journey(
    client: ParticipantClient,
    context: Context,
    saved: SavedJourney,
    wait: WaitOptions,
): Promise<OperationReference<MoveResult>> {
    const { conversation, session, control, retained, owner, assistant } =
        context;
    const send = await conversation.send(
        {
            messageId: saved.message,
            to: { first: owner.id, rest: [] },
            text: "Can we talk?",
        },
        { ...wait, operationId: saved.send },
    );
    const acceptance = await send.wait(wait);
    if (acceptance.value.messageId !== saved.message)
        throw Error("wrong original message");
    // Recipient/channel/provenance observations remain separate from acceptance.
    await conversation.receipts(saved.message, wait);
    const beforeConsult = await session.refresh(wait);
    const consultation = await session.consult(
        {
            via: control.id,
            retain: retained.id,
            with: { first: owner.id, rest: [assistant.id] },
            hold: "silence",
            expectedTopologyRevision: beforeConsult.topologyRevision,
        },
        { ...wait, operationId: saved.consult },
    );
    await consultation.waitUntilReady(wait); // State observation, not decoded audio.
    const beforeHandoff = await session.refresh(wait);
    const handoff = await session.handoff(
        {
            via: control.id,
            from: assistant.id,
            to: owner.id,
            retain: retained.id,
            outgoing: "leave",
            role: { scope: "session" },
            consultation,
            expectedTopologyRevision: beforeHandoff.topologyRevision,
        },
        { ...wait, operationId: saved.handoff },
    );
    await handoff.wait(wait);
    const beforeMove = await session.refresh(wait);
    const movement = await control.replaceDevice(
        {
            retain: retained.id,
            target: { endpoint: context.phone },
            expectedTopologyRevision: beforeMove.topologyRevision,
        },
        { ...wait, operationId: saved.move },
    );
    // The caller persists this reference even if waiting is cancelled or uncertain.
    const reference = movement.reference;
    await movement.wait(wait);
    return reference;
}
export async function recover(
    client: ParticipantClient,
    reference: OperationReference<MoveResult>,
    wait: WaitOptions,
): Promise<void> {
    const original = client.operations.recover(reference); // No submission and no I/O.
    const observed = await original.status(wait); // Query the original first.
    if (observed.state === "unknown" || observed.state === "interrupted")
        return;
    if (observed.state === "completed") await original.wait(wait);
    // No replaceDevice/send/invite call and no new logical mutation ID here.
}
export async function cancelWaitThenRequestCancellation(
    client: ParticipantClient,
    reference: OperationReference<MoveResult>,
    saved: SavedJourney,
    wait: WaitOptions,
): Promise<void> {
    const original = client.operations.recover(reference),
        local = new AbortController();
    local.abort();
    try {
        await original.wait({ ...wait, signal: local.signal });
    } catch (error) {
        if (!(error instanceof ConnectError) || error.kind !== "cancelled")
            throw error;
    }
    await original.status(wait); // Cancelling that wait made no remote decision.
    const decision = await original.cancel({
        ...wait,
        operationId: saved.cancel,
    });
    await decision.wait(wait); // Decision receipt still is not a fabricated rollback.
}
export async function invitations(
    client: ParticipantClient,
    saved: SavedJourney,
    wait: WaitOptions,
    accept: boolean,
): Promise<void> {
    const stream = client.incomingInvitations(wait);
    try {
        for await (const invitation of stream) {
            const current = await invitation.session.refresh(wait);
            if (current.state !== "invited") continue;
            const options = {
                ...wait,
                operationId: accept ? saved.accept : saved.reject,
            };
            if (accept)
                await (
                    await invitation.session.accept(
                        {
                            capabilities: { audio: ["opus"] },
                            expectedRevision: current.revision,
                        },
                        options,
                    )
                ).wait(wait);
            else
                await (
                    await invitation.session.reject(
                        {
                            reasonCode: 486,
                            reason: "Unavailable",
                            expectedRevision: current.revision,
                        },
                        options,
                    )
                ).wait(wait);
            break;
        }
    } finally {
        await stream.close();
    }
}
export async function events(
    context: Context,
    saved: SavedJourney,
    wait: WaitOptions,
): Promise<Cursor | undefined> {
    const stream = context.conversation.events({ after: saved.cursor }, wait);
    let cursor = saved.cursor;
    try {
        for await (const event of stream) {
            await context.conversation.refresh(wait);
            cursor = event.cursor;
            break;
        }
    } finally {
        await stream.close();
    }
    return cursor; // Persist after processing; a notification granted no authority.
}
export async function capabilityRefusal(
    client: ParticipantClient,
    wait: WaitOptions,
): Promise<void> {
    try {
        await client.capabilities.require("native-programmatic-media", wait);
    } catch (error) {
        if (!(error instanceof ConnectError) || error.kind !== "unsupported")
            throw error;
        return;
    }
}
export async function returnConsultation(
    context: Context,
    saved: SavedJourney,
    wait: WaitOptions,
): Promise<void> {
    const current = await context.session.refresh(wait);
    const consult = await context.session.consult(
        {
            via: context.control.id,
            retain: context.retained.id,
            with: { first: context.owner.id, rest: [] },
            hold: "silence",
            expectedTopologyRevision: current.topologyRevision,
        },
        { ...wait, operationId: saved.consult },
    );
    await consult.waitUntilReady(wait);
    const beforeReturn = await context.session.refresh(wait);
    await (
        await consult.returnToRetained(
            { expectedTopologyRevision: beforeReturn.topologyRevision },
            { ...wait, operationId: saved.returnConsult },
        )
    ).wait(wait);
}
// Both runtime entry points must type-check the same complete control workflow.
export const browserJourney = (
    client: BrowserClient,
    context: Context,
    saved: SavedJourney,
    wait: WaitOptions,
) => journey(client, context, saved, wait);
export const nodeJourney = (
    client: NodeClient,
    context: Context,
    saved: SavedJourney,
    wait: WaitOptions,
) => journey(client, context, saved, wait);

export async function managedLifetime(
    client: ParticipantClient,
    context: Context,
    saved: SavedJourney,
    wait: WaitOptions,
): Promise<OperationReference<MoveResult>> {
    try {
        return await journey(client, context, saved, wait);
    } finally {
        await client.close(wait);
    }
}

export async function recoverSaved(
    client: ParticipantClient,
    saved: Json,
    wait: WaitOptions,
): Promise<void> {
    const original = client.operations.restore(
        saved,
        client.operationKinds.connection_replace,
    );
    const observed = await original.status(wait);
    if (observed.state === "unknown" || observed.state === "interrupted")
        return;
    if (observed.state === "completed") await original.wait(wait);
}
