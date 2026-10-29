# Typed browser control SDK (G6-061 checkpoint)

Import `BrowserSdkClient` from `@rudeless/connect/browser`. It owns the same
transport selection, current enrollment and explicit reconnection rules as
[the transport owner](browser-transport-recovery.md). The browser entry point
loads no Node helper. Identity and revision brands are shared with the Node
SDK through a type-only leaf.

This checkpoint implements Client, Conversation, Participant, Session,
Connection, Stream and Operation handles. Supported mutations are Conversation
creation, private text messages, durable journal reads, Session
invitation/acceptance/rejection/cancellation/end, and native
WT/WSS audio/video/Screen Connection offer/end, typed Connection updates and
Stream subscribe/unsubscribe. Session controls include leave, role change,
additive media upgrade, codec policy and inviting new appearances on an existing
Session. Consultation/handoff, complete media parity and full app integration
remain in progress. A successful Session control operation does not establish media.

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
    participants: [savedRecipientIdentity],
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
client and a finite deadline for each `next()`. Closing or aborting a reader
settles its local wait without sending a decision. The recovery and notification
bounds are described below.

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
is no opaque continuation support in these mutation kinds. Saved references
and receipt snapshots do not confer current authorization. Explicit reconnect
resolves the selected transport's original enrollment again; revoked preferred
credentials refuse the round before any fallback attempt.

[Typed browser control evidence](../evidence/gate-6/browser-sdk-control.json)
records the signed boundary and owned Chrome checks over direct WT and bounded
WSS fallback. Later checkpoints below add explicit media owners and basic
source/progress UI and Screen capture. Linux, remote hosting and complete G6-061
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
    streams: {
        first: { id: savedAudioId, kind: "audio", direction: "sendrecv", codecs: { first: "opus", rest: [] } },
        rest: [{ id: savedVideoId, kind: "video", direction: "sendrecv", codecs: { first: "vp8", rest: [] } }],
    },
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

The original evidence below records initial Screen admission. The subsequent
controls checkpoint adds subscriptions and gates; full recovery/fault coverage,
received audio and frozen quality measurements remain in the G6-061 audit.

[Typed Screen evidence](../evidence/gate-6/browser-sdk-screen-owners.json)
records this bounded initial Screen lifecycle and the remaining qualifications.


## Connection controls and Stream selections

Enrollment must permit the requested `connection.update`, `stream.subscribe` or
`stream.unsubscribe` operation, plus original `operation.get` observation and
the relevant Conversation/Session reads.

Every control uses a caller-saved original mutation ID and an explicit negotiated
Stream revision. A current local media owner exposes its last committed
`streamRevision` through `connection.refresh(wait)`; observers without that
owner omit it. `resourceRevision` describes a Store resource and cannot be used
as a negotiated revision. A control receipt records its original revision;
restoring an older receipt never rolls the current local revision backward.
Concurrent changes can still conflict: observe the original result and refresh
current state before making a separate explicit decision.

```ts
const current = await connection.refresh(wait);
if (current.streamRevision === undefined) throw new Error("No local media revision");
// Persist selector, revision and original ID before IO.
const subscribe = await connection.streams.subscribe({
    selectors: { first: { stream: savedRemoteStreamId }, rest: [] },
    expectedStreamRevision: current.streamRevision,
}, { ...wait, operationId: savedSubscribeOperationId });
const selected = (await subscribe.wait(wait)).value;
// null means the original binding capture is unavailable; [] means known empty.
if (selected.bindings === null) throw new Error("Binding capture unavailable");
const screenBindings = selected.bindings.filter(v => v.parameters.medium === "screen");
const receiver = screenBindings.find(v => v.parameters.id === savedRemoteStreamId)
    ?? screenBindings[0];
if (receiver) {
    const stream = selected.connection.streams.byId(receiver.parameters.id);
    const renderer = await selected.connection.render({
        audio: false, video: [{ stream, canvas: remoteCanvas }],
    }, wait);
}
const mute = await connection.update({
    action: "mute", streams: { first: savedPublishedStreamId, rest: [] },
}, selected.streamRevision, { ...wait, operationId: savedMuteOperationId });
const muted = (await mute.wait(wait)).value;
// Later unsubscribe names the source logical Stream, which can differ from
// the fixed RTC receiver's own logical Stream ID.
const unsubscribe = await connection.streams.unsubscribe({
    streams: { first: savedRemoteStreamId, rest: [] },
    expectedStreamRevision: muted.streamRevision,
}, { ...wait, operationId: savedUnsubscribeOperationId });
```

`BrowserNonEmpty<T>` uses `{ first, rest }`. Selectors name one exact Stream or
one Participant with optional nonempty `kinds`; omitted kinds mean all supported
media for that Participant. WT supports correlated counteroffers, including
future Participant selection. The fixed WSS RTC profile supports exact sources
on already negotiated audio/camera/Screen ports; future nonaudio Participant
selection requires another profile and explicitly refuses. Camera and Screen
remain distinct logical media even though RTC uses one physical video slot.

Updates are the discriminated actions `mute`, `unmute`, `hold` with an explicit
`direction`, `resume`, `quality` with `intervalMs`, and `codec-renegotiate` with
nonempty `codecPreferences`. Typed outcomes distinguish gates, quality and
codecs. Backend support remains profile-specific: the current WSS RTC profile
supports gates, subscriptions and end, and refuses quality/codec renegotiation
that requires another profile. Gate observation directions belong to the server
binding and retain that orientation.

WT controls retain the original counteroffer owner, use one deadline including
queueing, and fence capture/render until local installation settles. An expired
queued request never submits later. These methods return observable Operations;
waiting, reconnecting and restoring only read the original record. Receipt
bindings are immutable observations and grant no new capture/render custody.
A logical Stream can appear independently on multiple Connections; each typed
Stream handle retains its Session and Connection scope.

[Typed browser stream control evidence](../evidence/gate-6/browser-sdk-stream-controls.json)
records the verified bounds and remaining Gate 6 work.


## Session policy and explicit re-invitation

`session.update(input, expectedTopologyRevision, mutation)` accepts four
`BrowserSessionUpdate` variants: `participant-leave` names a Participant;
`role-change` names a Participant and role; `medium-upgrade` supplies nonempty
`media`; and `codec-renegotiate` supplies nonempty `codecPreferences`.
Every method returns an observable original Operation. Its completed result
contains a Session handle with a historical `receiptSnapshot`; `refresh()` reads
current authorized state and never replaces it with an older receipt.

```ts
const current = await session.refresh(wait);
// Save this input, revision and original ID before IO.
const upgrade = await session.update({
    action: "medium-upgrade",
    media: { first: "screen", rest: [] },
}, current.topologyRevision, { ...wait, operationId: savedUpgradeOperationId });
await upgrade.wait(wait);
```

An upgrade needs current offered/accepted capabilities and authorization. A
Session policy result does not negotiate a new Connection or prove media flow.
The real browser fixture starts with audio/video media, adds Screen and data
policy, and then establishes and decodes Screen on separate original media
Connections. Its codec preference check runs after those Connections have ended;
live media reconciliation remains a separate qualification.

`session.inviteParticipants(...)` invites nonempty Participant targets into the
same Session. Supply the current `expectedTopologyRevision` and the existing
Session's medium, intent and capability offer. It is a separate explicit
mutation from the original invitation. A Participant that leaves can receive a
fresh invitation source and accept again without copying Session IDs from wire
frames or diagnostics. Current presence and management authorization still
apply; a handle or old receipt grants no permission to rejoin.

```ts
const current = await session.refresh(wait);
const reinvite = await session.inviteParticipants({
    to: { first: target.id, rest: [] },
    medium: "mixed",
    intent: savedSessionIntent,
    capabilities: savedSessionCapabilities,
    expectedTopologyRevision: current.topologyRevision,
}, { ...wait, operationId: savedReinviteOperationId });
await reinvite.wait(wait);
```

Enrollment must explicitly permit `session.update` or `session.invite` and the
required observations. `browserOperationKinds.sessionUpdate` restores only the
original update; `sessionInvite` also restores an existing Session invitation.

The pre-alpha façade now follows the frozen collection contract: Conversation
creation takes Identity IDs, and Connection offers take `{ first, rest }` for
both Streams and each Stream's codecs. No array-form offer or role-object
creation compatibility path is retained. Unknown or ambiguous update fields,
empty/repeated targets, media/codecs, malformed names and protocol size limits
refuse locally before IO.

[Browser Session control evidence](../evidence/gate-6/browser-sdk-session-controls.json)
records the exact implemented and tested scope.


## Invitation read recovery and notification limits

An incoming notification retains its original Conversation, Session, recipient
and server source correlation. Within the retained source window, fresh wire
IDs carrying the same source payload yield once; changed payload for that source
is a protocol failure. A cancellation must correlate to one of that source's
four retained wire IDs and match its Conversation, Session and source scope.
The signed cancellation may name a different current manager; the server owns
that authorization decision. Unknown cancellation replies have no effect on
other sources. A matched
cancellation removes a queued hint, including while a current-state read is
pending. It cannot cause a cached snapshot to yield the cancelled hint.

The queue permits at most 64 hints and 1 MiB of canonical payloads. Overflow
reports `resync-required` and retires the physical connection. Consumed/cancelled
source history retains up to 256 sources and 1 MiB of canonical payloads;
queued sources remain pinned independently. Deduplication is bounded by that
history window, with four recent wire IDs per source. Only required envelope
metadata is retained, and payload capture is isolated from caller mutation.
These are finite metadata bounds, not an exactly-once or durable inbox promise.

Before yielding, the SDK reads current authorized Conversation/Session facts
and checks current recipient membership epoch and invited appearance. It keeps
the hint queued until that read succeeds and the caller is still waiting.
Cancelling, closing or timing out the local wait does not consume the hint.
One client-owned snapshot read may continue under its separate, fixed two-second
deadline. A subsequent wait drains any earlier read and performs a fresh
current-authorized read; an abandoned result is never delivered as cached
permission. Repeated short waits cannot create parallel reads or reset that
original physical read's deadline. A permanent transport stall still retires
the physical connection at the owned deadline.

Physical retirement or explicit reconnect clears its old notification metadata.
A late read from an old physical cannot populate the new inbox. Reconnect never
replays a mutation, reopens capture, invents an invitation or turns an old
snapshot into current authorization. Applications should use the journal and
current state for recovery; this notification reader does not add offline
invitation discovery.

The actual Chrome fixture withholds an already received real snapshot at the
application promise boundary, cancels the local wait, then releases the original
read and verifies a second real authorized request before yielding. This is a
local scheduling/cancellation check, not an injected network outage. Signed
boundary checks separately cover late replies, current revocation, physical
replacement, malformed correlation and the finite permanent-stall boundary.

[Browser invitation recovery evidence](../evidence/gate-6/browser-sdk-invitation-recovery.json)
records the causal failures, corrected behavior and remaining G6-061 scope.


## Private messages and durable journal recovery

`conversation.send(...)` takes a saved Message ID, a nonempty `{ first, rest }`
list of Participant IDs and text. Save that intent and the mutation ID before
calling it. Targets are explicit; a missing target never means broadcast.
Text is limited to 512 KiB of valid UTF8, with at most 100 distinct targets.
The signed 1 MiB wire-size limit also applies after JSON escaping.

```ts
const events = conversation.events({}, wait);
const baseline = await events.ready();
await saveBaseline(baseline);
const send = await conversation.send({
    messageId: savedMessageId,
    to: { first: target.id, rest: [] },
    text: savedText,
}, { ...wait, operationId: savedSendOperationId });
await saveOriginalReference(send.referenceJson());
const acceptance = (await send.wait(wait)).value;
// acceptance.boundary === "local-mutation": Store acceptance only.
try {
    for await (const event of events) {
        await applyJournalFact(event);
        // Persist after processing; resume may repeat an unprocessed fact.
        await saveCursor(event.cursor);
    }
} finally {
    await events.close();
}
```

`browserOperationKinds.messageSend` restores the original acceptance without
sending again. Public operation references name canonical commands:
`conversation.create` and `message.send`. The SDK validates their existing
Store records under the recorded `create` and `send` names internally. Old
pre-alpha `create` references are refused; no legacy alias is inferred.
A message acceptance does not prove receipt, rendering, reading or provider
delivery. Delivery receipt queries still require the separate application read
service and are not implemented by this browser checkpoint.

Enrollment must grant `send` for messages, `conversation.get` for the baseline,
`conversation.subscribe` for journal reads and `history` for visible message
content. Current Conversation membership, recipient scope and content visibility
remain server decisions. The similarly named `events` grant is not the Store
subscription permission.

`events({ after: browserCursor(savedCursor) }, wait)` resumes after a saved
cursor. Cursors are opaque, Identity/Conversation/membership scoped values;
the client validates syntax without interpreting order. `ready()` establishes
an authorized baseline without consuming an event. With no saved cursor, that
baseline is the current snapshot watermark; snapshot items are not synthesized
as new journal facts. With a saved cursor, readiness validates a finite current
read, and `next()` reads again before delivering a fact. `resync-required` is
explicit: the application must obtain and reconcile current state, then choose
its new baseline. The reader never silently adopts a replacement cursor.

Journal facts preserve event/source correlation, original revision and observed
time, resource scope and audience membership epochs. The camelCase wrapper is
typed; `data` retains the canonical tagged JSON fact. The returned fact and its
nested JSON are immutable. Notifications do not replace this durable journal.

Each physical client permits eight event readers. Each reader owns at most one
finite two-second transport read, asks for one fact with `live: false`, and polls
an idle journal at 25 ms intervals within the caller's finite wait budget. Local
timeout, abort or close settles that wait without consuming a fact or closing
the shared physical connection. Any abandoned read is drained before a fresh
current-authorized read from the last unconsumed position; its old result is
never released as cached permission. Closed readers with pending IO retain
their slot until that finite read settles. Overlapping waits on one reader
refuse with `conflict`.

Explicit reconnect or physical retirement retires its readers. Create a new
reader with the last application-processed cursor on the replacement physical;
old work cannot follow it implicitly. No read resubmits a message or opens media.

[Browser message and journal evidence](../evidence/gate-6/browser-sdk-messages-journal.json)
records the signed boundary cases and actual WT/WSS workflow. Complete receipts,
broader frozen APIs and the G6-061 qualification audit remain open.
