// The original packet window and final control/capture joins have distinct
// completion paths. Failure still cancels every owner and never renews a bound.
import assert from "node:assert/strict";

export async function joinMediaProgramme({ startedAt, signal, abort, owners,
    packetTasks, controlTask, now = () => performance.now(),
    setTimer = (fn, milliseconds) => setTimeout(fn, milliseconds), clearTimer = clearTimeout }) {
    assert.ok(Number.isFinite(startedAt) && startedAt >= 0);
    assert.equal(typeof abort, "function");
    assert.ok(owners.length > 0 && packetTasks.length > 0);
    const readUntil = startedAt + 62200;
    let joinUntil = startedAt + 67000, readTimer, joinTimer, joining, endReads, rejectDeadline;
    const readEnd = new Promise(done => { endReads = done; });
    const deadline = new Promise((_, reject) => { rejectDeadline = reject; });
    const armDeadline = () => {
        if (joinTimer !== undefined) clearTimer(joinTimer);
        joinTimer = setTimer(() => {
            const error = Object.assign(Error("Original media programme owners did not join within five seconds"),
                { code: "media-programme-join-deadline" });
            abort(error);
            rejectDeadline(error);
        }, Math.max(0, joinUntil - now()));
    };
    const onAbort = () => {
        endReads();
        const narrowed = Math.min(joinUntil, now() + 5000);
        if (narrowed < joinUntil) { joinUntil = narrowed; armDeadline(); }
    };
    armDeadline();
    signal.addEventListener("abort", onAbort, { once: true });
    if (signal.aborted) onAbort();
    else readTimer = setTimer(endReads, Math.max(0, readUntil - now()));
    const settled = promise => Promise.resolve(promise).then(value => ({ status: "fulfilled", value }), reason => {
        abort(reason);
        return { status: "rejected", reason };
    });
    const stopOwners = () => joining ??= Promise.all(owners.map(owner => settled(
        Promise.resolve().then(() => owner.stopAndJoin()))));
    // Attach rejection handlers immediately. A failed child still owns its
    // siblings until cancellation and joining have completed.
    const packets = Promise.all(packetTasks.map(settled));
    const control = controlTask === undefined ? Promise.resolve({ status: "fulfilled" }) : settled(controlTask);
    const work = (async () => {
        await Promise.race([packets, readEnd]);
        // Normal read completion preserves the control owner while it seals
        // final physical captures after the original62s source programme.
        const stopped = await stopOwners();
        const [packetResults, controlResult] = await Promise.all([packets, control]);
        if (signal.aborted) throw signal.reason;
        const failure = [...packetResults, controlResult, ...stopped].find(v => v.status === "rejected");
        if (failure) throw failure.reason;
    })();
    try {
        await Promise.race([work, deadline]);
    } catch (error) {
        abort(error);
        // Deadline failure cannot abandon an original control mutation or a
        // live physical owner. Preserve the first refusal/unknown/cancellation.
        await Promise.allSettled([work, packets, control, stopOwners()]);
        throw signal.aborted ? signal.reason : error;
    } finally {
        if (readTimer !== undefined) clearTimer(readTimer);
        if (joinTimer !== undefined) clearTimer(joinTimer);
        signal.removeEventListener("abort", onAbort);
    }
}
