# Typed browser control SDK (G6-061 checkpoint)

Import `BrowserSdkClient` from `@rudeless/connect/browser`. It owns the same
transport selection, current enrollment and explicit reconnection rules as
[the transport owner](browser-transport-recovery.md). The browser entry point
loads no Node helper. Identity and revision brands are shared with the Node
SDK through a type-only leaf.

This checkpoint implements Client, Conversation, Participant, Session,
Connection, Stream and Operation handles. Supported mutations are Conversation
creation, private text messages, durable journal reads, Participant
admission/role updates/removal, Session
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
Stream revision. `connection.refresh(wait)` exposes the server-recorded committed
`streamRevision` for a ready physical generation, including audience changes
such as handoff. The field is omitted without an attributed ready observation.
A locally owned peer can retain a newer confirmed result while the Store catches
up. An observed revision grants no local capture or rendering ownership. `resourceRevision` describes a Store resource and cannot be used
as a negotiated revision. A control receipt records its original revision;
restoring an older receipt never rolls the current local revision backward.
Concurrent changes can still conflict: observe the original result and refresh
current state before making a separate explicit decision.

Native gate admission distinguishes a busy negotiation from a stale revision.
The runtime-plan check captures the requested/current revisions in its failed
receipt. Earlier Store admission may instead return the standard canonical
conflict against its last committed revision. Preserve the original input and
last completed control receipt. Neither preflight refusal starts an engine effect. A later explicit
attempt uses a new ID and a fresh revision; restoring the failed original reads
its unchanged failure. [The native control reference](native-runtime-controls.md)
describes the boundary between these refusals and uncertainty after an attempt.

```ts
const current = await connection.refresh(wait);
if (current.streamRevision === undefined) throw new Error("No observed media revision");
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
delivery. Delivery receipt queries use the explicit application read service described
below; they remain separate from message acceptance.

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


## Participant controls and membership epochs

`conversation.participants.add(...)` admits an existing Identity with its actual
`human`, `ai`, `system` or `external` kind and an explicit role. Identity
provisioning remains a separate trusted administrative capability. The browser
client does not create an Identity or infer its kind from contact metadata.

```ts
const current = await conversation.refresh(wait);
// Persist the complete input and its separate mutation ID before IO.
const admitted = await conversation.participants.add({
    identityId: savedRecipientIdentity,
    kind: savedRecipientKind,
    role: "reviewer",
    expectedRevision: current.revision,
}, { ...wait, operationId: savedAddOperationId });
const participant = (await admitted.wait(wait)).value;
const changed = await participant.update({
    role: "observer",
    expectedConversationRevision: (await conversation.refresh(wait)).revision,
}, { ...wait, operationId: savedRoleOperationId });
await changed.wait(wait);
const removed = await participant.remove({
    expectedConversationRevision: (await conversation.refresh(wait)).revision,
}, { ...wait, operationId: savedRemoveOperationId });
await removed.wait(wait);
```

Every control returns an original observable Operation. Use
`browserOperationKinds.participantAdd`, `participantUpdate` or
`participantRemove` with a saved reference for read-only recovery. No status
read repeats admission/removal, refreshes a stale mutation's asserted revision
or assigns a replacement operation ID. A revision conflict is an application
planning decision; uncertainty requires observing the original operation first.

The `receiptSnapshot` and `receiptConversationRevision` on a returned handle
are immutable views from the authorized receipt read. `refresh()` performs a
separate current authorized snapshot. Cached role, active state, epoch and
receipt metadata carry no authority. Recovery may refuse after membership
changes; an old receipt cannot borrow a successor membership epoch.

Enrollment must explicitly grant `conversation.participant.add`,
`conversation.participant.update` or `conversation.participant.remove` and its
required read operations and recipients. Roles contain 1–64 UTF8 bytes with no
control characters. The expected revision is a Conversation revision; Session,
topology and Stream revisions are distinct types. Unsupported fields and invalid
kinds, roles or revisions refuse before IO. A malformed correlated result or
Participant-change notice retains the original key with uncertain submission.

Removal stops current membership access. A separately authorized explicit
readmission preserves the stable Participant ID and increments its membership
epoch. Old epoch cursors and membership receipts remain subject to current
server authorization; possession of either cannot reopen old history. This
browser control API does not introduce history sharing.

[Browser Participant evidence](../evidence/gate-6/browser-sdk-participants.json)
records the signed control boundaries and actual two-transport membership
lifecycle. Full G6-061 and remote media qualification remain open.


## Separate application receipt reads

`conversation.receipts(messageId, wait)` returns immutable per-recipient/channel
observations. It requires an explicitly configured application HTTP service and
its own current enrollment resolver. Missing configuration returns `unsupported`;
there is no endpoint inference or reuse of the active WT/WSS control credential.

```ts
const client = await BrowserSdkClient.connect({
    tenantId: "tnt_default",
    identityId: enrolledIdentityId,
    deviceId: enrolledDeviceId,
    endpoints: trustedEndpoints,
    resolveEnrollment: vault.resolveProfiles(trustedControlProfiles),
    resolveApplicationRead: vault.resolveApplicationReads({
        name: savedApplicationReadEnrollmentName,
        url: trustedApplicationApiOrigin,
    }),
    timeoutMs: 10000,
});
const facts = await client.conversations.byId(savedConversationId)
    .receipts(savedMessageId, { timeoutMs: 5000, signal: controller.signal });
```

Prepare and approve the separate enrollment through the existing
[browser enrollment flow](browser-enrollment.md), saving its original key and
intent before IO. Use the same Identity and Device, an explicit browser Origin,
an approved browser signaling profile, `history` authority and the required
recipient ceiling. The owned fixture uses a separate `wss-webrtc` enrollment
with `control` capability and only `history` operation scope. The server already
permits finite HTTP reads independently of signaling transport while enforcing
the enrollment's exact Origin and current issuer/key/Device/credential state.
No new enrollment transport is introduced. `http-json` in the resolved read
options identifies the local HTTP client service.

The existing `history` authority also covers receipt reporting/preferences in
the server API; it is not a separately defined read-only credential scope. This
SDK method executes only receipt queries. Trusted backend policy chooses the
grant. The resolver reclaims the original current enrollment for each logical
read and materializes its finite credential in memory. The vault retains the
original key/intent, never the issued bearer. `applicationReadOptions(...)` also
supports explicit in-memory configuration from an already claimed matching
enrollment; the configured HTTPS origin must match its enrollment service.

Every fact preserves Participant ID, membership epoch, channel, delivery ID,
state, provenance and observation time. `queued` with `journal` provenance is
different from `delivered` with `client_reported` provenance. A `read` fact retains
its attribution and does not establish human action. The query does not report
delivery, mark anything read, change privacy preferences or send another message.
Current original audience, membership epoch, recipient ceiling and preferences
are enforced on each server page. A later page refusal discards the accumulated
result; no partial facts are returned.

One finite caller deadline covers resolver, original enrollment claim, HTTP,
body streaming and all pages. The client permits eight simultaneous reads, at
most 64 pages of 100 facts, 4096 distinct recipient/epoch/channel/delivery groups
and 2 MiB of response bytes in total. All requests omit cookies and referrers,
refuse redirects and bypass caches. Receipt source identifiers, enum values,
integer bounds and opaque cursor progress are validated before delivery. HTTP
401/403 are authorization refusals; an `unauthorized` body with HTTP 200 or 500
cannot masquerade as one. Errors omit remote messages.

Local cancellation, timeout, close or physical replacement aborts only that
independently disposable HTTP read. An abandoned result cannot request another
page or follow a replacement physical. A later call resolves current authority
again; no cached partial observations are released. Malformed/refused HTTP reads
do not close the shared canonical control connection or replay a mutation.

[Browser receipt evidence](../evidence/gate-6/browser-sdk-receipts.json) records
the signed control-boundary/HTTP fixtures and actual HTTPS/Chrome workflow. Its
synthetic delivery reports come from a separate fixture recipient through the
public application client; they are not evidence of human reading or a provider
outcome. Full G6-061 qualification remains open.


## Durable operation decisions

`original.cancel(...)` and `original.reconcile(...)` return separate observable
Operations. Each decision needs its own saved logical mutation ID. Persist the
decision intent, original target reference and decision ID before calling it.
Stopping a local `wait()` with an AbortSignal sends no durable cancellation;
`Session.cancelInvitation(...)` remains a separate Session decision.

```ts
await saveDecisionIntent({
    operationId: savedCancelDecisionId,
    target: original.referenceJson(),
    action: "operation.cancel",
});
const cancellation = await original.cancel({
    ...wait,
    operationId: savedCancelDecisionId,
});
await saveOriginalReference(cancellation.referenceJson());
const receipt = (await cancellation.wait(wait)).value;
// receipt.decision identifies this decision; receipt.target identifies original.
// receipt.observedState may still be "unknown" even though the decision completed.
const currentTarget = await original.status(wait);
```

The receipt preserves the target state observed at the decision and the host's
bounded disposition. It does not turn a cancellation request into proof of a
cancelled target, undo a completed message, establish media readiness or grant
capture/render custody. An already-terminal target can produce an
`already_terminal` decision while its original result remains unchanged.

Reconciliation takes one terminal outcome and an attributed evidence reference:

```ts
const reconciliation = await original.reconcile({
    outcome: savedTerminalOutcome, // "completed", "cancelled" or "failed"
    evidenceId: savedAttributedEvidenceId,
}, { ...wait, operationId: savedReconcileDecisionId });
const observed = (await reconciliation.wait(wait)).value;
```

The host independently verifies current authority, target state, worker revision
and the evidence it recognizes. Caller text or an outcome assertion supplies no
authority. The real Chrome fixture explicitly refuses an invented evidence ID;
signed boundary fixtures cover accepted reconciliation receipts. This checkpoint
does not add a browser evidence-issuance API or claim a new live provider/native
reconciliation qualification.

The request asserts the original target's Conversation/Session/Connection scope.
The decision's saved reference and status queries preserve Conversation/Session
scope and omit Connection ID, matching the durable decision ledger. A root
operation produces a root reference. Use `browserOperationKinds.operationCancel`
or `operationReconcile` with the frozen saved-reference JSON contract to restore
the decision itself. Recovery reads its original record without resubmission.
If submission is uncertain before a reference is returned, reconstruct that
contract from the saved decision ID/command and its Conversation/Session scope,
or retain the key on the SDK error. Never mint another decision ID to hide an
uncertain result.

Malformed target/evidence/state/result/scope or correlated acknowledgements
retain the separate decision key with uncertain submission. Command-only
operation scope may be absent from a correlated observation; if supplied it must
match. A 5xx problem after mutation submission also remains uncertain, since a
decision may already have committed. Explicit 4xx refusals remain refusals.
An already-aborted submission is `not-submitted` with its decision key. Same-ID
self-targeting, unsupported input/option fields and invalid evidence/outcomes
refuse before IO. Current revocation is checked again on every status/recovery
read; cached receipts do not supply permission.

The actual fixture exercises root and Conversation decisions on both WT and
bounded WSS fallback, original decision recovery after reconnect, unchanged
completed source operations, unattributed evidence refusal and current enrollment
retirement. Independent SQL verifies four cancel records and four decision-link
rows, with no accepted reconciliation record for the two refusals.

The full media/UI regression fixture now explicitly signals completion after its
last trusted keyboard step. Its UI coordinator stops at that boundary while API
and media recovery checks continue under their existing budgets. The final phase
and completion flag are asserted; the 60-second UI watchdog, 70-second evaluation
cap, per-call deadlines and media thresholds are unchanged.

[Browser decision evidence](../evidence/gate-6/browser-sdk-decisions.json) records
the exact scope, causal error-classification checks and remaining G6-061 work.

## Local runtime capabilities

```ts
const support = await client.capabilities.refresh({ timeoutMs: 1000 });
console.log(support.runtimeRole, support.selectedTransport, support.fallbackReason);
await client.capabilities.require('original-id-operations', { timeoutMs: 1000 });
if (support.features.has('message-receipts')) {
    // This client has an explicit application-read resolver configured.
    // Each actual receipt query resolves and checks its current authority.
}
```

`refresh` returns an immutable `connect/2` browser snapshot for the current
connected physical owner. Its transport and fallback reason reflect actual
selection, including explicit reconnect. Older snapshots remain immutable
observations. Closed/disconnected owners and expired signed authentication
cannot return a live profile. Both methods honor the caller's bounded local
budget and abort signal without enrolling, using a canonical control slot,
resolving HTTP authority or prompting for devices.

The exposed feature names are `original-id-operations`, `durable-events`,
`incoming-invitations` and `raw-canonical`, plus `message-receipts` when an application-read resolver
was configured. `browserSdkFeatures` exports these names. The readonly set has
no mutators, including through callbacks or native `Set` methods. Unknown or
unexposed names return `unsupported` with read-only certainty; malformed names
return validation errors. Native packet-media parity
is not advertised by this typed facade at this checkpoint.

Support describes local interfaces. It supplies no current server permission,
delivery fact, codec agreement, capture permission, render custody or decoded
media progress. A revoked credential can still have local interface support
until the owner learns of retirement; the next real action must pass current
server authorization. `browserCaptureSupport` and `browserCodecSupport` expose
separate local API/codec probes, while actual negotiation and media observations
remain on the Connection workflow.

[Capability evidence](../evidence/gate-6/browser-sdk-capabilities.json) includes
signed boundary cases, fully occupied control slots, expiry and retirement,
and actual Chrome WT/fallback selection with zero capability-query IO.

## Consultation and returning to the retained engagement

```ts
const original = await session.consult({
    via: primaryConnectionId,
    retain: remoteConnectionId,
    with: { first: organizerParticipantId, rest: [expertParticipantId] },
    hold: 'silence', // or 'configured-media', subject to host configuration
    expectedTopologyRevision,
}, { timeoutMs: 5000, operationId: savedConsultationId });
await saveOriginalReference(original.referenceJson());
const observed = (await original.wait({ timeoutMs: 5000 })).value;
// observed.retained/admitted/retired and topologyRevision are direct observations.

const returning = await original.returnToRetained({ expectedTopologyRevision: nextTopologyRevision }, {
    timeoutMs: 5000, operationId: savedReturnId,
});
await saveOriginalReference(returning.referenceJson());
const returned = (await returning.wait({ timeoutMs: 5000 })).value;
const receipt = returned.receiptConsultationReturn;
if (receipt?.remoteContinuing === false) {
    // Present the observed remote end; do not display a continuing call.
}
```

Save the full original intent and both distinct logical IDs before their
respective submissions. Consultation requires two to one hundred distinct
Participants, sorted canonically for submission, distinct primary/retained
Connections and an explicit topology revision below the maximum safe integer.
Unsupported input/option fields refuse before IO. Hold treatment remains an
explicit host policy; the SDK does not synthesize configured media.

The original saved reference uses `connect-operation-reference/1` with the same
Conversation/Session/primary-Connection scope and a `consultation` continuation
containing `retained_connection_id`. The correlated host observation adds
`record_id`, which identifies the host execution record, separately from the
logical operation ID. Save the opaque reference through `referenceJson()`; a
subsequent original status observation can add a previously unknown host record.
Restoration checks the command, scope, continuation shape, stable record and
monotonic operation revision. Unrelated operation kinds reject this context.

`returnToRetained` uses an already observed original host record without needing
parent-read permission. If the record remains unknown after a lost reply, one
current-authorized original status read obtains it under the same caller budget
as the new return. Failed/aborted/expired parent lookup leaves the new return
`not-submitted` with its own ID. A lost return reply remains uncertain and is
recovered by its original return reference; neither mutation is replayed.
Caller text or a guessed record ID cannot supply host return authority.

Accepted, ready and unknown observations remain separate from completion.
Bounded completed receipts validate distinct retained/admitted/retired IDs,
positive safe topology revision, exact original retained Connection, return
record and explicit continuation boolean. The original consultation admits
private Connections without retiring the retained engagement; return admits no
new Connections. Remote EOF may retire the original remote Connection and must
retain `remoteContinuing: false`. Receipts grant no capture/render custody or
proof of actual private audio. Refresh current resources and negotiate local
media through their separate owners.

[Consultation evidence](../evidence/gate-6/browser-sdk-consultations.json) records
signed controlled receipt/recovery cases and real Chrome current-enrollment
refusal. Successful private consultation/return over live browser media remains
a separate integration and qualification requirement.

## Handoff and device movement

An incoming target can offer one duplex Opus audio Stream with
`purpose: 'handoff-preparation'`. It must independently hold `connection.offer`
and `session.handoff`, accept the Session, start its own capture, and wait for
its original offer. The host isolates this source until an attributed handoff
commits. The target supplies that completed offer's `snapshot.recordId` as
`policyRef`; the caller saves it with the complete handoff intent before IO.
The host revalidates the target's current physical grant, membership and readiness.
The native peer retains revision 2 only after the original completed receipt
confirms the muted preparation; malformed confirmation closes that peer.
[Preparation revision evidence](../evidence/gate-6/browser-sdk-preparation-revision.json)
covers the exact receipt checks.
A copied source offer record is refused before a handoff effect starts.

Independent preparation supports WebTransport and WSS/WebRTC. Both paths
keep the target isolated and reject public/prepared subscriptions before a media
attempt starts; public duplex audio continues. See the
[prepared-source privacy evidence](../evidence/gate-6/browser-sdk-webrtc-handoff.json).

The same owned local workflow also completes handoff over both paths, retires
the outgoing capture, preserves the remote Connection and decodes duplex audio.
A previous WSS failure came from treating `handoff-pending` metadata as physical
retirement. The idle custodian now retains the original RTC owner during
attribution; fresh packet policy continues to deny IO until attribution is
available. Observation and decoded audio then resume on the same resources.
The evidence retains the failing runs, direct pending/resumed diagnostics and
passing normal cohort. Current stream revision refresh after audience changes,
consultation/return, device movement and full task qualification remain open.

```ts
const handingOff = await session.handoff({
    via: primaryConnectionId,
    from: outgoingParticipantId,
    to: incomingParticipantId,
    retain: remoteConnectionId,
    candidate: preparedCandidateId, // optional; otherwise the host selects admission
    policyRef: targetCompletedOffer.snapshot.recordId,
    outgoing: 'leave', // or 'observe'
    role: { scope: 'conversation', expectedConversationRevision },
    expectedTopologyRevision,
    consultation: originalConsultation, // optional original Operation
}, { timeoutMs: 5000, operationId: savedHandoffId });
await saveOriginalReference(handingOff.referenceJson());
const handoff = (await handingOff.wait({ timeoutMs: 5000 })).value;
// handoff.retained/admitted/retired and optional roleChange are captured observations.

const replacing = await primaryConnection.replaceDevice({
    retain: remoteConnectionId,
    target: { endpoint: selectedEndpointId }, // or { candidate: preparedCandidateId }
    expectedTopologyRevision,
}, { timeoutMs: 5000, operationId: savedMoveId });
await saveOriginalReference(replacing.referenceJson());
const moved = (await replacing.wait({ timeoutMs: 5000 })).value;
// moved.connection is a handle; moved.binding describes the observed committed binding.
```

Save full mutation intent and each original logical ID before IO. Session role
handoff uses `role: { scope: 'session' }`; Conversation role handoff also requires
its distinct Conversation revision. Topology and Conversation assertions must be
below the maximum safe integer. Source and target Participants differ, primary
and retained Connections differ, and a prepared candidate differs from both.
Device movement chooses exactly one nominal Endpoint or Connection target.
Closed input/option shapes refuse injected authority or payload extensions.
The host checks current participant, membership, endpoint, device and worker
state before any effect; a selected target ID supplies no authority.

An optional consultation is captured from its original saved reference and must
belong to the same Conversation and Session with a distinct logical ID. Its
already observed host record avoids a parent read. When that record remains
unknown, handoff obtains it through the current client under the same bounded
budget as submission, even if the original handle belonged to a retired client.
It never borrows the old physical client. Parent read refusal, abort or deadline
leaves the new handoff not submitted with its own ID. Neither flow enrolls a
replacement identity or retries an uncertain mutation.

Strict `handoff` and `replacement` continuations preserve target/role/outgoing
intent in `connect-operation-reference/1`. Restore with
`browserOperationKinds.sessionHandoff` or `connectionReplace`, then query or
wait on the original operation. Status enforces current authority and monotonic
record observation. Lost ACKs and server 5xx retain uncertainty; explicit
reconnect and original-ID recovery do not resubmit either mutation. An unknown
state and a local wait abort do not dispatch another handoff, move or durable
cancellation.

Completed receipts are bounded to 32 KiB and one hundred distinct topology IDs.
Both admit exactly one Connection and retain the specified engagement. Handoff
checks any exact candidate, source retirement versus leave/observe, Conversation
revision advancement and any attributed role-change fields. Native role data
can be absent; the SDK does not invent it. Replacement checks the source it
replaces, its retirement, exact selected target, admitted Connection and positive
safe generation/membership epoch with nominal Participant/Endpoint/Device IDs.
The immutable `binding` and optional `roleChange` are observations, rather than
current grants. Server readiness metadata does not allocate a local media owner;
returned handles cannot borrow capture/render custody from another Connection.

[Movement evidence](../evidence/gate-6/browser-sdk-movements.json) separates
controlled signed successful receipts/recovery from actual Chrome refusal of
ungranted actions. [Live WebTransport handoff evidence](../evidence/gate-6/browser-sdk-live-handoff.json)
adds independently prepared synthetic audio, isolation before handoff, copied-policy
refusal preserving public audio, decoded duplex audio after handoff, and recovery
without resubmission. Only the submitting Operation's validated completion retires
its captured outgoing local owner; it checks the same physical peer, epoch and
local entry. Restored references carry no retirement callback. Device movement,
consultation/return and successful WebRTC handoff remain outstanding. [Fallback audio evidence](../evidence/gate-6/browser-sdk-fallback-audio.json)
resolves the earlier zero-output probe: the SDK owns a muted audio element that
starts Chrome RTC decoding while its Web Audio graph controls audible volume.
Actual duplex PCM and playback after renderer pause/resume now pass. Startup
and resume share bounded graph/playback budgets; rejection, deadline, abort,
retirement and close clear the exact owned sink and join graph cleanup.

## Advanced finite raw canonical exchanges

```ts
const original = client.raw.validate({
    v: 2,
    id: 'local-validation-metadata',
    ts: '2026-10-29T00:00:00Z',
    type: 'operation.get',
    cid: savedConversationId,
    payload: { operation_id: savedLogicalId },
});
const frames = await client.raw.request(original, { timeoutMs: 5000 });
for (const frame of frames) {
    // Complete immutable original envelopes, including signed optional fields.
    inspectPrivately(frame.value);
}
```

`validate` captures an immutable, unforgeable envelope witness using the fixed
current schema, 66-kind registry, required operation/revision fields, identifier
and calendar rules, and raw message decoded-body limits. It performs no IO,
configuration, enrollment or signature authentication, including after the client
closes. Local schema validation supplies no authorization or media custody.
Caller signature metadata may be schema-valid while cryptographically unverified.

`request` uses the current owned signed physical client and its bounded control
queue. Fresh attempt ID, timestamp and signature replace only physical metadata;
original payload extensions, logical operation ID, scope and expected revision
are preserved. Save full intent and a stable logical ID before a mutation. Raw
successful results contain every correlated validated frame through the final
ACK; interpret command-specific outcomes explicitly or restore their typed
original Operation. Raw envelopes do not create local capture/render owners.

The finite request set is generated from the existing Rust raw policy. It covers
Conversation/Participant/Session/movement/Connection/stream control, messages,
capabilities, DTMF and original-operation reads/decisions. Unknown top-level
intent fields, response correlation supplied as a request, authentication/step-up,
native offer/answer, binary-record transfer and live event leases refuse before
IO. Finite `conversation.subscribe` requires explicit `live: false`; use
`Conversation.events` for the journal workflow. Validating a known envelope does
not imply that this finite raw interface can execute it.

Responses pass the fixed schema before canonical refusals are interpreted, in
addition to the owned peer's signature, replay and correlation checks. Exact
resource scope and the final ACK are required. Immutable results retain signed
extensions; malformed later frames discard the batch and retire the captured
physical. A retired response cannot follow a replacement owner. Current
revocation is checked on each real request; earlier validated envelopes and
observed results provide no cached permission.

The caller's single bounded budget covers request and complete response
validation. Before-IO abort/validation preserves a known not-submitted mutation;
submitted abort, lost reply or server 5xx retains the original uncertain key.
Explicit reconnect and status-first recovery never replay raw mutations. Explicit
canonical 4xx refusals remain refusals. Raw request options are closed to
`timeoutMs` and `signal`.

Wire JSON follows the frozen Rust parser's one-MiB envelope and 127-container
boundary, with a 1,100,000-node ceiling sufficient for the byte-bounded syntax.
General application JSON keeps its existing 32-depth/100,000-node defaults.
The owned reply queue additionally bounds a batch to 128 frames and one MiB
including framing. Raw `message.send` obeys the frozen 65,536-byte decoded-body
bound, including strict canonical standard base64 padding and pad bits. These
are wire/interface bounds, not media or performance qualification.

Regenerate fixed browser data after an approved source-contract change:

```sh
bash scripts/prepare-dependencies.sh
python3 scripts/generate-browser-canonical.py
# Or verify checked-in data:
python3 scripts/generate-browser-canonical.py --check
```

Generation checks the prepared Rust schema/registry and finite-policy source.
[Raw canonical evidence](../evidence/gate-6/browser-sdk-raw.json) includes 339
shared vectors checked independently by the frozen Rust parser, identifier regression
coverage and causal calendar/signature-clock negatives, signed component
exchanges, and actual Chrome WT/fallback mutation, reconnect/original recovery
and current enrollment retirement.


## Frozen TypeScript shape audit

Consultation, handoff and device-move results implement `BrowserTopologyChange`
directly: `retained`, `admitted`, `retired`, `topologyRevision` and optional
`conversationRevision` are result fields. Consultation additionally has `session`;
move additionally has `connection` and `binding`. `BrowserConsultationOperation`
is the named original-operation type. Returning takes the explicit
`{ expectedTopologyRevision }` input. The earlier nested result and positional
return forms have been removed to match the approved TypeScript contract.

[The contract audit](../evidence/gate-6/browser-sdk-contract-audit.json) records
all 25 mapped method surfaces, actual compiler witnesses and remaining gaps.
The shared cross-language return samples include owner and assistant;
[their four compiler checks](../evidence/gate-6/browser-sdk-return-outlines.json) pass.
The [bounded shutdown evidence](../evidence/gate-6/browser-sdk-shutdown.json)
resolves the earlier close signature and cleanup finding. The historical native
Screen-resume branch cannot be recovered from the old trace.
[Explicit gate refusal and recovery evidence](../evidence/gate-6/browser-sdk-gate-refusals.json)
now distinguishes busy negotiation from captured revision conflicts and verifies
original-receipt recovery followed by a new decoded Screen resume. G6-061 stays in
progress; the verified WT/WSS handoff covers this bounded local movement workflow.
Full runtime parity, browser consultation/return, device movement and current stream
revision refresh after audience changes require additional implementation and
evidence.

## Bounded owned shutdown

```typescript
await client.close({ timeoutMs: 5000, signal: shutdownSignal });
```

`close(options)` requires a 1–60000 ms deadline and returns `Promise<void>`.
Valid shutdown retires control, local capture/playback, event readers and
application receipt reads immediately. It joins control and datagram reader
cancellation, writer abort and admitted writes, native reconciliation, RTC
signaling/track replacement, audio graph cleanup and owned HTTP fetch/body
cleanup. Retired and opening physical peers remain owned until their cleanup
settles. At most 16 physical peers may await cleanup; another reconnect refuses
before allocating a new physical peer when that budget is full.

Cancellation (including an already aborted signal) and deadline stop the wait;
retirement continues. A later `close` call can join the same work with a new wait
budget. Cleanup failure is reported after other owned cleanup settles. Invalid
options refuse before retirement. Closing sends no durable cancellation,
Session-end or replacement intent: retain operation references to observe
remote outcomes through a separately authorized client.

Application enrollment/receipt resolvers and device permission prompts can
outlive cancellation in the host. Shutdown aborts their owned wrappers and
fences late results; late tracks are stopped and late profiles cannot open
another physical or submit work. Owned HTTP requests and browser resource
cleanup remain joined, and a stalled host resource can therefore exhaust the
shutdown wait. Logs and browser profiles for local verification stay ignored.


## Stream observations across audience changes

A Connection snapshot’s nullable `streamRevision` is the current server-observed
control version for its physical generation. A Stream snapshot’s
`resourceRevision` versions that logical Stream’s observed state. Use the
Connection control version for a new control assertion; retain the original
operation ID and intent when acceptance or completion is uncertain. Neither
version grants capture or renderer ownership.

An attributed handoff, consultation or return carries existing Stream
descriptors only when the same live Connection generation advances by exactly
one control version. A route-only change preserves Stream IDs and resource
versions. If the actual atomic Core commit changes a gate, the matching audio
Stream receives that captured state and a new resource version. A missing or
wrong Stream incarnation makes the descriptor snapshot unavailable; it does
not create a substitute Stream. Retired, replaced, unavailable or newer
snapshots cannot be revived by a historical commit.

The browser handoff fixture checks these observations over WT and WSS alongside
decoded media and original-operation recovery. Replacement descriptor carry
and successful live browser consultation/return and device movement remain
open. See [audience descriptor evidence](../evidence/gate-6/browser-sdk-audience-descriptors.json)
for the bounded verification scope.
