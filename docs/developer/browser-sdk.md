# Typed browser control SDK (G6-061 checkpoint)

Import `BrowserSdkClient` from `@rudeless/connect/browser`. It owns the same
transport selection, current enrollment and explicit reconnection rules as
[the transport owner](browser-transport-recovery.md). The browser entry point
loads no Node helper. Identity and revision brands are shared with the Node
SDK through a type-only leaf.

This checkpoint implements Client, Conversation, Participant, Session,
Connection, Stream and Operation handles. Supported mutations are Conversation
creation, Session invitation/acceptance/rejection/cancellation/end, and native
WT/WSS audio/video Connection offer/end. Connection updates and subscriptions,
complete media parity, broader Session controls and accessible UI remain in
progress. A successful Session control operation does not establish media.

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
is no opaque continuation support in these eight mutation kinds. Saved references
and receipt snapshots do not confer current authorization. Explicit reconnect
resolves the selected transport's original enrollment again; revoked preferred
credentials refuse the round before any fallback attempt.

[Typed browser control evidence](../evidence/gate-6/browser-sdk-control.json)
records the signed boundary and owned Chrome checks over direct WT and bounded
WSS fallback. This checkpoint does not qualify camera/microphone permissions,
source selection, screen capture, keyboard UI, Linux,
remote hosting or complete G6-061 acceptance.


## Current physical media owners

Key enrollment deliberately leaves Endpoint assurance unverified. Before native
media, a trusted backend must separately attribute and verify the enrolled
Endpoint through the administration API. A successful claim or control
connection does not bypass this step. The owned fixture verifies only the
closed set of its own approved synthetic Endpoints, with an explicit evidence
reference; no browser receives administration credentials.

`session.connections.offer` accepts persisted Connection/Stream IDs and typed
stream intent. The current native browser profile supports duplex Opus audio,
VP8 video/screen and ordered text/JSON/binary data. Unsupported directions or
codecs refuse before submission; the SDK does not replace the requested
profile. WSS uses WebRTC with required Opus audio and optional VP8 camera video;
WSS screen/data and audio-free profiles refuse locally. The same typed
Session/Connection workflow selects the current transport's media owner.

An optional `media: { iceServers, iceTransportPolicy }` client configuration
supplies explicit WebRTC ICE services; the SDK adds no implicit STUN/TURN
service. Configuration is captured and validated before connecting. The RTC
offer creates null-track transceivers and never prompts for capture permission.

```ts
const offer = await session.connections.offer({
    connectionId: savedConnectionId,
    streams: [
        { id: savedAudioId, kind: "audio", direction: "sendrecv", codecs: ["opus"] },
        { id: savedVideoId, kind: "video", direction: "sendrecv", codecs: ["vp8"] },
    ],
}, { ...wait, operationId: savedOfferOperationId });
// Capture uses the already saved Connection intent, before waiting for the
// original offer's completion: WSS readiness requires actual primary RTP.
const connection = session.connections.byId(savedConnectionId);
const capture = await connection.capture({
    audio: true,
    video: "camera",
    audioDeviceId: selectedLocalMicrophone,
    videoDeviceId: selectedLocalCamera,
}, { timeoutMs: 30000, signal: captureAbort.signal });
await offer.wait(wait);
const current = await connection.refresh(wait);
const streams = await connection.streams.list(wait);
```

The mutation returns after validated current physical parameters arrive;
negotiation continues under the original offer budget. Operation completion and
local installation remain separate. `connection.capture(...)` and
`connection.render(...)` wait within their own finite local read budgets for
that physical installation. They never submit another offer. Codec preflight,
parameter exchange and local negotiation readiness share the original offer
deadline; preflight cancellation cannot submit late intent. Operation reads
have their own finite observation budgets and never reset that offer budget.

Connection and Stream `refresh()` return current authorized resource snapshots.
Their `resourceRevision` brands are distinct from negotiated stream revision;
use an independently observed negotiated revision for native control assertions.
The offer's `receiptStreamRevision` is historical receipt data. Stream snapshots
always report `ownership: "observation"`, including when another local owner
happens to be active.

The capture in the example selects this offer's senders. A matching, already
owned synthetic MediaStream can instead be transferred with `stream`; its
tracks stop when the capture or Connection retires. WT supports
`video: "screen"` for an explicitly offered screen Stream.

Device IDs remain local capture constraints. Screen selection uses the browser
permission UI and cannot take a camera device ID. Capture startup is bounded
by its supplied 1–60000 ms deadline; active capture continues until explicit
close, abort or owner retirement. One capture owner per Connection prevents
two local sources from writing the same negotiated senders. A renderer routes
native decoded frames or RTC receive tracks internally; the application supplies
scoped receive Stream handles and canvases. Pausing a hidden document keeps
capture tracks disabled until an explicit resume while visible. Cancelling an
uncertain RTC `replaceTrack` retires that RTC owner so a late replacement cannot
affect a new capture; the shared control client remains available.

`connection.end(input, mutation)` returns the original observable Operation;
observing its completed receipt retires local media owners while preserving
other Connections and the control client. Physical retirement also closes
capture/playback. Restoring an offer after reconnect yields a historical handle,
with no new local media resources, capture prompt or permission grant. A new
explicit media offer is a separate saved user intent.

The owned Chrome fixture exercises the same audio/video offer, explicit
synthetic capture, original completion, scoped observations, renderer startup,
exclusive ownership, end and reconnect recovery over WT and WSS. It animates
its canvas so the RTC host can observe fresh primary video RTP. It does not
request personal devices. Actual synthetic device permissions, owned-tab
screen selection, received decoded quality and complete media parity remain
required before G6-061 can close. RTC renderer `diagnostics.audio` is currently
unmeasured (zero); its video counter records presentation callbacks only.

[Native media ownership evidence](../evidence/gate-6/browser-sdk-native-owners.json)
records the current typed WT Connection/Stream and local owner checks.

[WSS composition evidence](../evidence/gate-6/browser-sdk-wss-owners.json)
records this increment and its remaining qualification work.
