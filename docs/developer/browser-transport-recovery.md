# Browser transport selection and recovery (G6-052)

`BrowserTransportClient` owns a browser's authenticated physical control
connection. It exposes the preferred transport, selected path, fallback reason
and local physical lifetime counter through `status`. The path labels are
`webtransport` and `wss-webrtc`; a selected path does not establish media
readiness. The low-level `BrowserClient` remains available through `client`.

```ts
import { BrowserTransportClient } from "@rudeless/connect/browser";

const connection = await BrowserTransportClient.connect({
    identityId: enrolledIdentityId,
    deviceId: enrolledDeviceId,
    endpoints: {
        webtransport: "https://connect.example/connect",
        "wss-webrtc": "wss://connect.example/connect",
    },
    preferredTransport: "webtransport",
    allowFallback: true,
    timeoutMs: 10000,
    attemptTimeoutMs: 3000,
    resolveEnrollment: async ({ signal }) => {
        // Trusted application enrollment flow: scoped bearer, enrolled Device,
        // private signing CryptoKey and current server verification trust.
        return await loadCurrentBrowserEnrollment({ signal });
    },
});
console.log(connection.status.selectedTransport);
```

Enrollment resolution and both possible physical attempts share one original
deadline. Each round resolves enrollment once and captures its values before
contacting either endpoint. A missing preferred endpoint/API or a physical
opening failure can select the other configured secure contact. Authentication
refusal, signature failure, invalid configuration and authentication-stage
timeout terminate the round. Browsers can hide the cause of a physical network
failure; the SDK cannot classify every opaque browser TLS/network rejection.
The alternate contact still requires platform TLS and the captured enrolled
server signing trust. Development certificate hashes apply only to WT.

There is no automatic reconnection, mutation queue or mutation replay. A lost
physical connection clears the selected path. Explicit `reconnect()` first
closes the old peer, resolves fresh enrollment and authenticates again. Actor
and Device IDs must match the owner. A concurrent reconnect is refused; closing
the owner cancels pending enrollment/opening and prevents late adoption.

Save a logical operation reference before submitting a mutation. After an
uncertain reply, reconnect explicitly and observe the original operation:

```ts
await connection.reconnect({ signal: controller.signal });
const operation = await connection.recoverOperation({
    operationId: savedOperationId,
    command: "session.handoff",
    scope: {
        conversationId: savedConversationId,
        sessionId: savedSessionId,
        connectionId: savedConnectionId,
    },
});
// Keep preparing/interrupted/unknown visible; status reads never resend handoff.
```

`command` is the original canonical ledger kind. For example, a Conversation
creation is `create`, an unscoped Conversation message is `send`, and a Session
handoff is `session.handoff`. Supply the original command's asserted scope.
`recoverOperation` checks the returned original ID, kind, scope, state and
revision. The server applies current authorization; reconnect grants no access
to a previously visible operation after permission is withdrawn. Malformed or
ambiguous status responses close the physical peer.

`recoverConversation({ conversationId, cursor, limit })` reads one finite
journal page with `live:false`. It returns authoritative `conversation.event`
frames and summary metadata, excluding advisory notifications. With no cursor
it reads one snapshot page; pass the returned `nextPage` as `page` to continue.
Persist the returned cursor/page with the application's recovery state. An
explicit resync response obtains a fresh authorized snapshot. An expired page
restarts at the first snapshot page only for the exact signed
`409/state/resync_required` error, once, within the original read deadline.
Other refusals propagate. Limits are 1–63 items, default 32. A read started on a
retired physical lifetime cannot populate the new one.

The physical counter is a local stale-response fence. QUIC path migration can
retain the same physical connection. A new authenticated connection retires
old capture/media owners; applications must explicitly establish and observe
new media. Replacing a Device/Connection uses its distinct authorized operation.
Neither control reconnect nor selected path promises uninterrupted media after
a server crash. High-level SDK resource composition remains G6-062 work.

Current verification is the signed WebCrypto component fixture recorded in
[transport owner evidence](../evidence/gate-6/transport-reconnect-owner.json).
Actual installed-browser outages, expired service credentials and repeated
reconnect during live consult/handoff remain required before G6-052 closure.
