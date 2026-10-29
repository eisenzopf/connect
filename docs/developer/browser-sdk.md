# Typed browser control SDK (G6-061 checkpoint)

Import `BrowserSdkClient` from `@rudeless/connect/browser`. It owns the same
transport selection, current enrollment and explicit reconnection rules as
[the transport owner](browser-transport-recovery.md). The browser entry point
loads no Node helper. Identity and revision brands are shared with the Node
SDK through a type-only leaf.

This checkpoint implements Client, Conversation, Participant, Session and
Operation control handles. Supported mutations are Conversation creation and
Session invitation, acceptance, rejection, cancellation and end. Connection,
Stream, broader Session controls, media composition and accessible UI remain
in progress. A successful Session control operation does not establish media.

```ts
import {
    BrowserSdkClient, browserOperationKinds, sdkId,
} from "@rudeless/connect/browser";

const client = await BrowserSdkClient.connect({
    tenantId: "tnt_default",
    identityId: enrolledIdentityId,
    deviceId: enrolledDeviceId,
    endpoints: trustedEndpoints,
    resolveEnrollment: vault.resolveProfiles(trustedProfiles),
    timeoutMs: 10000,
    attemptTimeoutMs: 2000,
});
const wait = { timeoutMs: 10000, signal: controller.signal };
// Persist this logical intent and operation ID before calling the SDK.
const create = await client.conversations.create({
    policy: "persistent",
    participants: [{ identityId: savedRecipientIdentity, role: "participant" }],
}, { ...wait, operationId: savedCreateOperationId });
const conversation = (await create.wait(wait)).value;
const target = await conversation.participants.byIdentity(savedRecipientIdentity, wait);
// The application saves a separate Session and mutation ID for a new invitation.
const invite = await conversation.sessions.invite({
    sessionId: savedSessionId,
    to: [target.id],
    medium: "text-chat",
    intent: "synchronous-engagement",
    capabilities: { data: ["reliable-ordered"] },
}, { ...wait, operationId: savedInviteOperationId });
const session = (await invite.wait(wait)).value;
```

Enrollment needs the explicit operations the application uses. In particular,
`conversation.get` permits Conversation snapshots, while `session_get` permits
visible Session facts within them. Receiving invitation metadata needs current
Session observation or decision authority. The synthetic control fixture uses
`create`, `conversation.get`, `operation.get`, `session_get`, `session.invite`,
`session.accept`, `session.reject`, `session.cancel` and `session.end`, with both
synthetic Identity IDs explicitly listed as permitted recipients. Real
applications should issue only their required subset.

Incoming notifications supply a scoped Session handle. The application does
not copy a Conversation or Session ID from diagnostics to accept or reject:

```ts
const incoming = client.incomingInvitations(wait);
try {
    const hint = (await incoming.next()).value;
    if (hint) {
        const current = await hint.session.refresh(wait);
        // Persist a separate decision operation ID before this mutation.
        const reject = await hint.session.reject({
            reasonCode: 486,
            reason: "Unavailable",
            expectedTopologyRevision: current.topologyRevision,
        }, { ...wait, operationId: savedRejectOperationId });
        await reject.wait(wait);
    }
} finally {
    await incoming.close();
}
```

The incoming source correlation is the server's stable invitation source,
distinct from the inviter's mutation ID. It grants no mutation or media
custody. The SDK rechecks current Conversation membership, the recipient
Identity and membership epoch, and the invited Session appearance before
yielding. Notifications remain hints: an idle notification stream is neither a
durable inbox nor evidence that no invitation exists. There is one reader per
client, a bounded queue and a finite deadline for each `next()`; close or abort
settles a pending wait without sending a decision.

Every mutation returns an observable Operation. `status()`, `wait()` and
`waitUntilReady()` read the original record under current authorization.
Cancellation of a wait does not cancel the server operation. Unknown or
interrupted state remains observable until the read deadline; polling never
resubmits the mutation. Record identity and revision rollback, foreign result
scope and malformed responses refuse and retire the physical peer. A malformed
mutation reply retains the original operation key with uncertain submission.

```ts
// A reference can also be constructed from the already saved original IDs
// through the frozen connect-operation-reference/1 JSON contract.
await saveOriginalReference(invite.referenceJson());
await client.reconnect();
const original = client.operations.restore(
    await loadOriginalReference(), browserOperationKinds.sessionInvite,
);
const completion = await original.wait(wait);
// In-memory references retain their invariant result type.
const same = client.operations.recover(original.reference);
```

`restore` checks the SDK-owned kind and original scope locally before IO. There
is no opaque continuation support in these six mutation kinds. Saved references
and receipt snapshots do not confer current authorization. Explicit reconnect
resolves the selected transport's original enrollment again; revoked preferred
credentials refuse the round before any fallback attempt.

[Typed browser control evidence](../evidence/gate-6/browser-sdk-control.json)
records the signed boundary and owned Chrome checks over direct WT and bounded
WSS fallback. This checkpoint does not qualify camera/microphone permissions,
source selection, capture/render handles, screen capture, keyboard UI, Linux,
remote hosting or complete G6-061 acceptance.
