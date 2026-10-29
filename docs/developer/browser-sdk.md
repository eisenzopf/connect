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
complete media parity, broader Session controls and full app integration remain
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
is no opaque continuation support in these eight mutation kinds. Saved references
and receipt snapshots do not confer current authorization. Explicit reconnect
resolves the selected transport's original enrollment again; revoked preferred
credentials refuse the round before any fallback attempt.

[Typed browser control evidence](../evidence/gate-6/browser-sdk-control.json)
records the signed boundary and owned Chrome checks over direct WT and bounded
WSS fallback. Later checkpoints below add explicit media owners and basic
source/progress UI. Screen capture, Linux, remote hosting and complete G6-061
acceptance remain open.


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
profile. WSS uses WebRTC with required Opus audio and one VP8 camera or screen
when the explicit AV host is configured. Camera and screen cannot share that
single physical video slot; WSS data and audio-free profiles refuse locally. The same typed
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
tracks stop when the capture or Connection retires. Both transports support
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
request personal devices. The UI checkpoint below adds actual browser permission
checks with synthetic devices. The screen checkpoint below adds owned-tab
display selection. Received decoded quality and complete media/API/lifecycle
qualification remain required before G6-061 can close. RTC renderer `diagnostics.audio` is currently
unmeasured (zero); its video counter records presentation callbacks only.

[Native media ownership evidence](../evidence/gate-6/browser-sdk-native-owners.json)
records the current typed WT Connection/Stream and local owner checks.

[WSS composition evidence](../evidence/gate-6/browser-sdk-wss-owners.json)
records this increment and its remaining qualification work.

## Source and progress controls

`BrowserSourcePicker` provides labeled local microphone/camera list boxes and a
video-source choice. Construct it with the sources the application can offer;
unsupported sources are disabled. `refresh()` enumerates input devices under a
finite read budget and never requests permission. Labels may be unavailable
until the browser grants permission. The picker retains no group IDs and adds
no device data to Connect requests. Device IDs are local capture constraints.

```ts
import {
    BrowserSourcePicker, BrowserCaptureControls, BrowserOperationProgress,
} from "@rudeless/connect/browser";

const picker = new BrowserSourcePicker({ audio: true, video: ["camera"] });
container.append(picker.element);
await picker.refresh({ timeoutMs: 3000 });
// On the application's explicit offer action, persist the selected sources,
// original Connection/Stream IDs and operation ID before submitting the offer.
const selection = picker.selection();
await saveMediaIntent({ selection, input: savedOfferInput, mutation: savedMutation });
const offer = await session.connections.offer(savedOfferInput, savedMutation);
await saveOriginalReference(offer.referenceJson());
const connection = session.connections.byId(savedOfferInput.connectionId);
const captureControls = new BrowserCaptureControls(connection, selection, {
    timeoutMs: 30000,
});
const progress = new BrowserOperationProgress(offer, { timeoutMs: 10000 });
container.append(captureControls.element, progress.element);
```

The capture button explicitly starts permission for this Connection. A denied
permission shows a fixed, announced error and enables retry on the same
Connection. Pending duplicate actions share the same capture attempt. Closing
or stopping the controls cancels their local permission/capture work and stops
late tracks; a retired owner cannot publish a late ready state. Source changes
that require different negotiated Streams need a separate saved offer intent.
A logical screen selection still invokes the browser's separate display picker;
no stored camera device ID selects a screen. The app must offer logical
`kind: "screen"` for this choice, with current Screen authorization and capabilities.

The progress button observes the original Operation with current authorization.
It can stop waiting or recheck after a failed read without submitting a new
mutation. Both controls use native buttons, live status text and `aria-busy`;
focus moves to the stop button during a pending action and returns after failure
or completion if it is still there. Closing a panel prevents late UI updates.
The caller owns mounting/removing elements and persists references independently
of the panels' lifetime. Call `close()` on the picker and both panels on teardown.

[Source and UI evidence](../evidence/gate-6/browser-sdk-ui-permissions.json)
records actual owned Chrome permission denial/grant with Chrome's synthetic
microphone/camera, trusted keyboard source/capture/retry actions and current
operation progress over WT and WSS. This qualifies these basic interactions;
it does not qualify a complete reference app, assistive-technology matrix,
personal devices or received decoded media quality. The screen checkpoint below
records its separate display-capture scope.

## Logical screen media

A Screen Stream retains its original kind and ID through WT or WSS offer,
answer, current Stream observation and recorded end. RTC SDP uses its one
physical video section; it cannot turn a camera permission into Screen
capability or admit camera and screen together. The server `opus-vp8` host
validates audio plus exactly one logical camera or screen against that SDP.
Current Session media, offered and accepted `screen: ["vp8"]` capabilities, and
an explicitly verified Screen-capable Endpoint remain separate requirements.
The fixture uses an explicitly `mixed` Session so both separate camera and
screen Connections fit the original Session media intent.

Select `video: "screen"` on the pending scoped Connection to invoke
`getDisplayMedia` through the capture controls' explicit user action. The browser
selects the display source; `videoDeviceId` is forbidden for this selection.
Microphone capture remains separate. Selecting a tab may move focus away from
the controls page, so automatic background pause is reported as `paused`. Return
to the controls and use **Resume capture** explicitly; the SDK does not silently
re-enable hidden capture. Both owners stop their tracks on capture,
Connection or physical retirement. Observing the saved original offer after
reconnect grants no display capture custody and cannot reopen the picker.

The bounded fixture uses Chrome's
[tab-only test selection switch](https://chromium.googlesource.com/chromium/src/+/HEAD/chrome/common/chrome_switches.h)
to select a titled tab created in its fresh owned profile. It verifies
`displaySurface: "browser"` and a distinctive pixel color from that owned tab.
This is local source-capture evidence; remote decoded playback remains a
separate observation. No personal tab, display or application window is selected.

RTC Screen subscriptions and gates through the typed facade, full recovery/fault
coverage and received decoded quality remain in the G6-061 audit. The current
increment implements initial offer/capture/render/end and original observation.

[Typed Screen evidence](../evidence/gate-6/browser-sdk-screen-owners.json)
records this bounded initial Screen lifecycle and the remaining qualifications.
