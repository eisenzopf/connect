import { ConnectError } from "./errors.js";
import { canonicalJson, parseCanonicalJson } from "./uctp_json.js";
import { identifier, type Envelope } from "./uctp_wire.js";
import { sessionCapabilities, sessionMedium } from "./browser_sdk_sessions.js";

interface Seen {
    readonly key: string;
    readonly payload: string;
    readonly bytes: number;
    readonly frame: Envelope;
    readonly epoch: number;
    readonly wires: string[];
}
export interface BrowserInvitationHint {
    readonly frame: Envelope;
    readonly epoch: number;
}
const bad = (): never => {
    throw new ConnectError("protocol", "Invalid original invitation source");
};
const size = (text: string) => new TextEncoder().encode(text).length;
const text = (v: unknown, maximum: number, empty = false): v is string =>
    typeof v === "string" &&
    (empty || !!v) &&
    size(v) <= maximum &&
    !/[\x00-\x1f\x7f-\x9f]/u.test(v);

/** Physical-local notification metadata. No authority, media or durable inbox. */
export class BrowserInvitationInbox {
    private queue: Seen[] = [];
    private queuedBytes = 0;
    private seen = new Map<string, Seen>();
    private seenBytes = 0;
    clear(): void {
        this.queue = [];
        this.queuedBytes = 0;
        this.seen.clear();
        this.seenBytes = 0;
    }
    peek(): BrowserInvitationHint | undefined {
        return this.queue[0];
    }
    consume(hint: BrowserInvitationHint): boolean {
        if (this.queue[0] !== hint) return false;
        this.queuedBytes -= this.queue.shift()!.bytes;
        return true;
    }
    record(frame: Envelope, epoch: number): boolean {
        if (frame.type === "session.cancel") return this.cancel(frame);
        if (frame.type !== "session.invite") return false;
        const p = frame.payload;
        if (
            frame.connid !== undefined ||
            frame.in_reply_to !== undefined ||
            frame.expected_revision !== undefined ||
            !identifier(frame.cid) ||
            !identifier(frame.sid) ||
            !identifier(frame.operation_id) ||
            !identifier(p.from) ||
            !Array.isArray(p.to) ||
            p.to.length !== 1 ||
            !identifier(p.to[0]) ||
            p.from === p.to[0] ||
            !sessionMedium(p.medium) ||
            !text(p.intent, 256)
        )
            return bad();
        sessionCapabilities(p.capabilities_offer, bad);
        const payload = canonicalJson(p, 65536),
            bytes = size(payload);
        const key = JSON.stringify([
            frame.cid,
            frame.sid,
            frame.operation_id,
            p.to[0],
        ]);
        const previous =
            this.seen.get(key) ?? this.queue.find((v) => v.key === key);
        if (previous) {
            if (previous.epoch !== epoch || previous.payload !== payload)
                return bad();
            if (!previous.wires.includes(frame.id)) {
                if (previous.wires.length === 4) previous.wires.shift();
                previous.wires.push(frame.id);
            }
            return false;
        }
        if (this.queue.length >= 64 || this.queuedBytes + bytes > 1048576)
            throw new ConnectError(
                "resync-required",
                "Invitation notification queue exhausted",
            );
        // The payload is compared exactly; retain at most 256 sources / 1 MiB.
        // Queued sources remain independently pinned until consumed/cancelled.
        while (this.seen.size >= 256 || this.seenBytes + bytes > 1048576) {
            const first = this.seen.values().next().value!;
            this.seen.delete(first.key);
            this.seenBytes -= first.bytes;
        }
        const captured = {
            v: 2,
            type: frame.type,
            id: frame.id,
            ts: frame.ts,
            cid: frame.cid,
            sid: frame.sid,
            operation_id: frame.operation_id,
            payload: parseCanonicalJson(
                new TextEncoder().encode(payload),
                65536,
            ),
        } as Envelope;
        const value: Seen = {
            key,
            payload,
            bytes,
            frame: captured,
            epoch,
            wires: [frame.id],
        };
        this.seen.set(key, value);
        this.seenBytes += bytes;
        this.queue.push(value);
        this.queuedBytes += bytes;
        return true;
    }
    private cancel(frame: Envelope): boolean {
        if (!identifier(frame.in_reply_to)) return false;
        const original = [...this.queue, ...this.seen.values()].find((v) =>
            v.wires.includes(frame.in_reply_to as string),
        );
        if (!original) return false;
        const p = frame.payload,
            source = original.frame;
        if (
            frame.cid !== source.cid ||
            frame.sid !== source.sid ||
            frame.operation_id !== source.operation_id ||
            frame.connid !== undefined ||
            frame.expected_revision !== undefined ||
            !Number.isSafeInteger(p.reason_code) ||
            Number(p.reason_code) < 0 ||
            Number(p.reason_code) > 65535 ||
            !text(p.reason, 256, true) ||
            (p.by !== undefined && p.by !== null && !identifier(p.by))
        )
            return bad();
        const index = this.queue.indexOf(original);
        if (index < 0) return false;
        this.queue.splice(index, 1);
        this.queuedBytes -= original.bytes;
        return true;
    }
}
