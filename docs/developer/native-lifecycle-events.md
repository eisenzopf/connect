# Native lifecycle and quality events

G6-046 installs signed native lifecycle observations on the physical peer that
owns the negotiated Connection. `connection.ready` and `stream.opened` require
observed media flow in the negotiated directions; an accepted operation or a
signed offer/answer alone cannot establish readiness. `stream.closed` describes
a previously announced binding that has disappeared or been replaced.

`connection.quality` uses the engine's measured packet and byte counters at the
default five-second cadence. Its stream IDs and wire Conversation, Session and
Connection scopes belong to that peer. Unmeasured loss, jitter, RTT and MOS are
omitted. Quality observations do not establish remote receipt, playback or a
business operation outcome.

Registrations retain original source metadata without retaining the original
command's permission readers. Each event obtains current Store authority for the
original credential, signing key, Device, Endpoint, Identity, participant epoch
and Connection generation. The Service validates its exact physical scope and
current media owner. That guard remains owned through the engine's bounded real
reliable write; it expires with the original source or physical/runtime lifetime.
The engine additionally fences the exact flow and stream bindings. Waiting for
current authority holds no permission reader. One physical peer multiplexes its
own registered Connections independently of Conversation subscriptions.

These signed native frames have no `in_reply_to` or logical `operation_id`.
`CanonicalClient::request` preserves unrelated frames in its bounded event inbox;
callers use `receive` to consume them. Low-level `send`/`receive` users must
correlate command replies and handle intervening events. A cursor-bearing journal
fact remains the authority for durable state; these events are transport hints.

The native envelope reference is
[uctp-envelope-2.schema.json](../../contracts/schemas/uctp-envelope-2.schema.json).
The [HTTP command reference](../../contracts/openapi-canonical-http.json) retains
its explicit physical-peer requirement: these native observations are not HTTP
command replies or a substitute for the Conversation event journal.

The [local verification report](../evidence/gate-6/auxiliary-native-lifecycle.json)
records three actual-listener cases,34 affected runtime regressions, two physical
provider-write regressions and166 library checks (205 distinct tests). EdDSA and ES256 verify readiness after actual duplex flow, exact stream
and Connection scopes and measured quality. Revoking the actual target Endpoint
through the public administration SDK stops its fresh events without parking an
idle reader or suppressing another peer's quality. A real PCMU-to-PCMA change
closes the original binding and opens only the observed replacement; FFmpeg checks
its decoded media. Owned public audio also replays its original selection after
leaving lifecycle traffic pending through a full quality cadence.

The public audio selector collects unrelated frames from the wire into its
bounded inbox. It does not consume that inbox recursively or mistake another
scope for its own parameter request or receipt. Control-only test fixtures
explicitly consume lifecycle hints; the dedicated lifecycle tests inspect them
through raw signed SDK reception. Eighteen actual received frames validate against
the native wire schema, including both signing configurations and codec closure.

This qualifies the local default delivery component. Configurable cadence,
capability advertisements, DTMF, step-up, continuous signaling consumption by
owned provider audio, Linux, browser, remote hosting, acceptance and the complete
G6-046 task remain open. The component does not qualify captured media or general
availability under load.

Codec replacement now publishes its `stream.closed` / `stream.opened` pair
adjacently on one guarded writer claim after fresh replacement flow. The old
`closed_at` remains the first observed retirement time; the two envelope
`ts` values share their publication time. A negotiation ACK, old handle or old
flow cannot cause either half to be published early. The announcement state
advances only after both writes finish. A partial second write stays uncertain
and destroys its physical sink before current authority is released.
