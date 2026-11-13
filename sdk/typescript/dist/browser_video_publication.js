const lookups = new WeakMap();
const connections = new WeakMap();
const owned = new WeakMap();
const inputs = new WeakMap();
const encoded = new WeakMap();
const packets = new WeakMap();
export function videoPublication() {
    return { active: true, epoch: 0, needed: false, boundaryMs: Infinity };
}
export function completeVideoPublication(value, at = performance.now()) {
    if (!value.active || value.epoch !== 0 || !Number.isFinite(at))
        throw new Error("Original replacement publication boundary changed");
    value.boundaryMs = at;
    value.epoch = 1;
    value.needed = true;
}
export function closeVideoPublication(value) {
    if (value)
        value.active = false;
}
export function publicationLookup(peer, lookup) {
    lookups.set(peer, lookup);
}
export function publicationConnection(connection, peer, id) {
    connections.set(connection, { peer, id });
}
export function capturePublicationRegistration(connection, stream) {
    const source = connections.get(connection);
    const publication = source
        ? lookups.get(source.peer)?.(source.id, stream)
        : undefined;
    return Object.freeze({
        publication,
        epoch: publication?.epoch,
        registeredMs: performance.now(),
    });
}
export function ownPublicationFrame(frame, registration, captureTimeMs, presentationTimeMs) {
    owned.set(frame, {
        registration,
        captureTimeMs,
        presentationTimeMs,
        deliveredMs: performance.now(),
    });
}
// false refuses the SAME owned input. undefined preserves ordinary publishers.
export function publicationInput(publication, frame) {
    const original = owned.get(frame), stamp = original?.registration;
    if (stamp?.publication &&
        (!stamp.publication.active || stamp.epoch !== stamp.publication.epoch))
        return false;
    if (stamp?.publication && stamp.publication !== publication)
        return false;
    if (!publication)
        return undefined;
    if (!publication.active)
        return false;
    if (original &&
        (!Number.isFinite(stamp.registeredMs) ||
            !Number.isFinite(original.deliveredMs) ||
            (original.captureTimeMs !== undefined &&
                !Number.isFinite(original.captureTimeMs)) ||
            (original.presentationTimeMs !== undefined &&
                !Number.isFinite(original.presentationTimeMs))))
        return false;
    if (original &&
        (stamp?.publication !== publication ||
            stamp.epoch !== publication.epoch))
        return false;
    if (publication.epoch === 1 &&
        original &&
        (stamp.registeredMs < publication.boundaryMs ||
            original.deliveredMs < publication.boundaryMs ||
            (original.captureTimeMs !== undefined &&
                original.captureTimeMs < publication.boundaryMs) ||
            (original.captureTimeMs === undefined &&
                (original.presentationTimeMs === undefined ||
                    original.presentationTimeMs < publication.boundaryMs))))
        return false;
    const value = {
        publication,
        epoch: publication.epoch,
        timestampUs: frame.timestamp,
        basis: !original
            ? "explicit-sdk-submission"
            : original.captureTimeMs !== undefined
                ? "owned-capture-clock"
                : "owned-new-callback-presentation-only",
        captureTimeMs: original?.captureTimeMs,
        accepted: false,
        output: false,
    };
    inputs.set(frame, value);
    return value;
}
export function originalPublicationInput(frame) {
    return inputs.get(frame);
}
export function acceptPublicationInput(input) {
    if (input)
        input.accepted = true;
}
export function encodedPublication(bytes, input) {
    if (input?.accepted && !input.output) {
        input.output = true;
        encoded.set(bytes, input);
    }
}
export function takeEncodedPublication(bytes) {
    const value = encoded.get(bytes);
    encoded.delete(bytes);
    return value;
}
export function currentPublicationOutput(publication, input, timestampUs, key) {
    return (!!input &&
        input.accepted &&
        input.publication === publication &&
        publication.active &&
        input.epoch === publication.epoch &&
        input.timestampUs === timestampUs &&
        !(publication.epoch === 1 && publication.needed && !key));
}
export function publicationPackets(input, key, bytes, createdMs) {
    const group = {
        input,
        key,
        remaining: bytes.length,
        failed: false,
        deadlineMs: createdMs + 200,
    };
    for (const value of bytes)
        packets.set(value, { group, settled: false });
}
// Move metadata through the EXISTING clone; no payload copy or public API added.
export function clonePublicationPacket(original, clone) {
    const value = packets.get(original);
    if (value) {
        packets.delete(original);
        packets.set(clone, value);
    }
}
export function currentPublicationPacket(bytes, expected) {
    const value = packets.get(bytes);
    if (!value)
        return !expected;
    return (!!expected &&
        value.group.input.publication === expected &&
        expected.active &&
        value.group.input.epoch === expected.epoch);
}
export function settlePublicationPacket(bytes, sent, at = performance.now()) {
    const value = packets.get(bytes);
    packets.delete(bytes);
    if (!value || value.settled)
        return;
    value.settled = true;
    const group = value.group;
    group.failed ||= !sent;
    if (--group.remaining !== 0)
        return;
    const input = group.input, publication = input.publication;
    if (!group.failed &&
        group.key &&
        at < group.deadlineMs &&
        publication.active &&
        input.accepted &&
        input.epoch === 1 &&
        publication.epoch === 1)
        publication.needed = false;
}
