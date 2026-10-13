# Connect development SDKs

`connect-client` is the native Rust SDK. It uses existing rvoip UCTP/QUIC with
mandatory certificate/hostname verification. The WS parity peer is a test
harness, not this SDK or a TypeScript native package.

The configuration, native exchange lifecycle, typed errors and application
facade have separate modules. Consumers import protocol types from
`connect_client`; they do not need the server or storage crate.

| Method | Result / behavior |
|---|---|
| `open(ClientConfig)` | Verified TLS, required profile and credential-subject binding |
| `resolve(address)` | Scoped durable identity and declared capabilities |
| `create(request_id, participants)` | Saved conversation, caller included |
| `send(request_id, conversation_id, body)` | Durable `SendReceipt`; not proof of peer processing |
| `history(conversation_id, after, limit)` | Ordered `HistoryPage`; follow next until empty |
| `inbox(cursor, limit, wait_ms)` | Resumable `InboxPage`; at-least-once events |
| `request_status(request_id)` | Accepted saved outcome, Rejected, or NotFound |
| `close_conversation(request_id, conversation_id)` | Blocks new sends; history/receipts remain |
| `health()` | Network installation/profile/schema facts |
| `request(Operation)` | Full typed application operation |
| `request_cancelled(operation, token)` | Explicit cancellation; closes physical peer |
| `close()` | Bounded physical-peer shutdown |
| `session_invite/accept/reject/cancel/end/get` | Durable bounded legs under an existing conversation |
| `sip_dial/status/end` | Durable outbound SIP leg inside an accepted audio Session |
| `open_native_audio` | Negotiated PCMU/8 kHz UCTP/QUIC datagram stream |
| `subscribe_native_audio` | Authorized publisher subscription on the same Session |
| `availability_set/get` | Expiring availability and preferred-route hint |
| `context_view` | Authorized 32 KiB conversation projection with provenance and provider-delivery outcomes |
| `artifact_begin/append/finish/get/delete` | Offset-checked, digest-verified reliable transfer operations |

Client requests are serialized per client. Timeout/cancellation closes that
physical peer; reopen it to continue. A transaction might already have committed,
so retain the original logical ID and inputs. The SDK does not silently retry or
mint replacement IDs. `NotFound` is not proof a concurrent uncertain request
cannot still finish. Exact replay and request lookup use current authority.

Configuration timeout is bounded to 1–60,000 ms per exchange (10,000 default).
An inbox wait is at most 5,000 ms. The caller may choose shorter limits; an inbox
wait longer than its deadline predictably returns a timeout. Trust material is
required. This local profile accepts loopback endpoints only. The config's stable
identity metadata does not override the server's authenticated sender; examples
also resolve their own alias and verify its ID before opening their work state.

Persist mutations **before** sending. Persist each business effect, handled event
ID, any pending reply command and cursor atomically in the app's own store.
A restored server snapshot may be older than the app cursor; that is an explicit
error requiring reconciliation, not an instruction to reset app history.

A compiling native example is in the crate's Rust documentation; the runnable
process journey is [examples/review-loop/README.md](../examples/review-loop/README.md).
Verify them with:

```sh
cargo test --locked -p connect-client --doc
cargo test --locked -p connect-server --test sdk_facade
python3 scripts/check-examples.py
python3 scripts/check-example-recovery.py
```

The deterministic examples prove the transport/recovery interface without
provider keys. Gate 3's native two-peer and native-to-Vapi audio paths are implemented in Rust.
Python and TypeScript expose the control, context, availability, artifact and
bounded PCMU media operations through the native helper. MCP and the installable
skill remain Gate 4.

## Gate 2 interfaces

The Rust client also exposes `provider_status(conversation_id)`. The TypeScript
SDK is in `sdk/typescript`; Python is in `sdk/python/connect_sdk`. Both provide
`NativeClient` through a persistent Rust helper, and a separate loopback
`HttpClient`. They share identities, logical request IDs, outcomes and cursors.
Neither uses WebSocket as a substitute for native QUIC. These are development
sources, not published npm/PyPI packages.

The messaging sample and provider setup instructions are in
[examples/agent-presence](../examples/agent-presence/README.md). Gate 2 requires real Vapi chat and telephone voice in both directions. Reviewed SMS
enrollment, conversation and keyword enforcement have
[entry qualification evidence](connect-pre-gate-4-entry.html). Historical
[task/SMS controls](evidence/gate-4/sms-0737-rehearsal.json) and
[voice-to-chat/SMS continuity](evidence/gate-4/live-voice-continuity.json)
retain their actual tested sources. The frozen dc73996 package and full local
qualification are recorded in the [source reconciliation](evidence/gate-4/final-source-composition.json).
Owner-review repairs are qualified at 709cf82 in the [numbered plan](connect-gate-4-implementation.html).
MCP tools and the installable skill are delivered Gate 4 interfaces;
HTTP/JSON alone is not MCP.

The default [agent installation](AGENT_ACCESS.md) uses one regular revocable
communication credential across its authorized conversations, with optional
owned task tracking. A separate [protected local developer exchange](TASK_ACCESS.md)
supports optional Native/HostSession task grants. The CLI retains the original issuance ID and
exports the bearer into a private file for MCP host setup. It cannot issue
provider-origin grants, and model grants cannot refresh themselves. Native SDK
grant authentication is implemented as described below. It is an optional
narrow mode, not a prerequisite for ordinary communication. The
[actual normal-agent journey](evidence/gate-4/real-model-agent-dc73996.json)
qualifies explicit exact-session continuation at dc73996. Current repair-cycle qualification passes at 709cf82 and is tracked in the [Gate 4 plan](connect-gate-4-implementation.html#G4-021).

Tracked reply consumption is an explicit owner operation, separate from reads,
waits and follow-up sends. The native `consume_reply` task command and the ordinary
MCP `connect_task_consume_reply` tool use the selected task, stable request ID,
received conversation journal event ID and expected task revision. The normal
MCP result wraps the task record as `kind: task` → `kind: record`; optional narrow
grant catalogs do not expose that owner tool. See
[agent access](AGENT_ACCESS.md) for the reply-reference and replay contract.
The newly exposed MCP parity path is qualified at 709cf82 in
[G4-017 repair qualification](connect-gate-4-implementation.html#G4-017).

## Gate 2 telephone call control

Implemented methods are `call_start(request_id, conversation_id, route_id, destination)`,
`call_status(conversation_id, call_id)` and
`call_end(request_id, conversation_id, call_id)`. TypeScript uses camelCase
method names; Python and Rust use snake_case.

They control Vapi telephone calls; they do not carry client audio. New voice
routes restrict destinations and bind a Vapi assistant/phone ID to a conversation.
Voice credentials require an explicit grant; messaging credentials do not gain
voice access. Call receipt replay must not redial, and uncertain creation must
not be automatically repeated. Private provider control URLs remain server-side.

[Voice samples](../examples/agent-presence/voice.md) include persistent request
receipts and inbound setup. The supported Vapi native Telnyx integration provides
the Gate 2 telephone route. The separately tested BYO SIP credential path still
returns Vapi HTTP 400. Gate 3's separate SIP APIs originate and terminate a real
rvoip SIP connection inside an accepted Connect Session; they do not reuse Vapi
call control.

Carrier origination is an operator concern. `[sip.outbound]` supplies a proxy,
asserted identity and authentication username while referring to a password
environment variable by name. Provider credentials never enter an SDK request,
receipt, journal event, or committed configuration example. A successful local
SIP peer test proves Connect's media path; Telnyx/PSTN is claimed only after the
same path passes live qualification. A wildcard signaling bind requires a
concrete `sip.media_bind_ip` for local RTP allocation; `sip.advertised` and
`sip.media_public` describe public mappings and are never assumed to exist on
the host.
An optional `sip.outbound.registrar_uri` registers the credential trunk before
startup reports ready. rvoip handles refresh and unregister on graceful shutdown;
the Contact identifies the trunk username, separately from caller identity.

Schema 6 preserves call direction, inbound bindings, discovery watermarks and
the credential that requested a call end. End intent survives concurrent polls
and restart; an uncertain end is reconciled by reads rather than blindly repeated.
Dispatch rechecks the requesting credential's current voice authority.

Inbound records appear through the existing inbox after authenticated polling.
Discovery scans bounded updated-time windows with pagination, provider-ID
deduplication and a durable watermark. See the sample for scan limits and delay.
Caller ID is a routing hint, never an authorization grant. Current verification
and live audio results are linked from the Gate 2 implementation checklist.

## Gate 3 native media boundary

`open_native_audio` binds an already accepted durable Session to the stock UCTP
session/connection/stream lifecycle. Audio is PCMU, 8 kHz, mono and 20 ms per
frame. The client owns a peer-global media router and processes unsolicited
`stream.opened` announcements independently from correlated application
requests, so history or artifact control cannot consume media-control events.

When an accepted Session targets an identity with one enabled Vapi phone or connector route,
the initiator's native connection can attach the saved assistant through the
pinned `rvoip-vapi` duplex WebSocket adapter. Connect injects the authorized
bounded context as explicitly untrusted history, records pending/delivered/failed
delivery state, and keeps provider credentials and WebSocket URLs server-side.
Final provider transcripts and bounded end reports are appended as
source-attributed `voice_transcript` and `voice_outcome` context items in the
same durable conversation. Exact replays are idempotent; changed content for a
stable provider reference becomes an explicit source-linked correction. The
bridge drains queued observations on teardown within a two-second deadline.
Missing end reports and detected transcript-event loss produce explicit
`voice_context_status` items attributed to Connect; these are not provider
transcripts or inferred successful outcomes. The
loopback provider test proves PCMU in both directions, restart, and delivery of
the earlier spoken decision to a subsequent assistant Session. The separately
recorded real-account native proof carried three spoken turns over UCTP/QUIC and
Vapi; independent returned-audio qualification and owner browser confirmation are recorded in the [Gate 3 report](evidence/gate-3/acceptance.html).
The [native microphone example](../examples/conversation-client/README.md#native-microphone-and-speaker)
uses optional local audio-device support. Python helper diagnostics inherit
stderr by default, or can be directed to a caller-owned file via the `stderr`
argument, so an unread internal diagnostics pipe cannot stall media.

`sip_dial` stores intent before signaling. A committed rvoip SIP connection is
registered before it becomes peer-visible, then bound to the durable Session.
Busy, local cancellation, no-answer, restart, and authority loss have stable
terminal outcomes. SIP, WebRTC and native QUIC connections that name the same
accepted Session can be paired through the rvoip media graph. Configured inbound
routes may set `session_id`; when omitted, Connect creates a bounded inbound
Session. Caller ID and SIP headers never select a conversation.

The browser entry point uses authenticated WebRTC signaling and does not load
the Node helper or claim browser-native QUIC. The configured WebRTC route accepts
only its selected credential; another valid credential, including a different
grant for the same identity, cannot assume that route's authority. Its HTTP
control facade permits
credentialed CORS only from loopback HTTP origins; webhook-only listeners do
not receive that layer. Microphone acquisition, ICE and signaling are bounded,
and a stream that resolves after its deadline is stopped. The local
interoperability suite proves WebRTC↔native QUIC and WebRTC↔SIP duplex audio.
HTTP/JSON does not carry real-time audio. Artifact operations use bounded base64
chunks on reliable UCTP control and raw authenticated HTTP bytes; limits are 16
MiB per file, 256 MiB per installation and 32 KiB decoded chunks. Blob storage,
current authorization, resume, deletion, expiry and backup/restore are
implemented.

`BrowserVoiceSession.ended` settles after either side ends the leg and microphone
capture stops. Applications should use it to reset their call controls and
detach playback. A server hangup retains the parent conversation; joining again
creates a new bounded Session in that conversation.

Vapi text chat receives the same authorized projection, including final voice
transcripts, through `assistantOverrides.variableValues.connect_context`.
Connect-created assistants include `{{connect_context}}` in their system prompt.
An existing assistant must include that placeholder; the connector checks its
configuration and rejects a context-bearing chat when it is absent. Connect
does not edit an existing assistant's prompt automatically. It submits the
current message separately, with a 10,000-character provider limit. Full context
delivery is journaled before dispatch; a successful response records delivery,
while an interrupted request remains uncertain. Actual model use requires a
separate observed reply. See [Vapi's dynamic-variable guide](https://docs.vapi.ai/chat/quickstart).

For a definite rejected Vapi job, a local administrator may use
`connect discard-failed-chat --config OPERATOR_CONFIG --server-config SERVER_CONFIG --job-id JOB_ID`.
This records an acknowledgement and retains the failed context selection. It
never retries that job, and cannot discard an uncertain remote outcome. New
work needs a new explicit message and mutation ID.

## Qualified scope and context limits

Gate 3 qualifies local macOS arm64, native PCMU/QUIC, recorded Telnyx SIP/RTP
telephone routes and Chrome host/host UDP ICE. The public browser facade owns
accepted input tracks during negotiation and closes them on failure or hangup.
Native provider hangup reports `media-ended`; it is a terminal media state.
Physical QUIC closure, including credential expiry or Session revocation, reports
a transport error promptly instead of waiting for the audio receive deadline.
Reconnect using a currently authorized credential to continue the Conversation.

Provider context selects up to 1000 newest candidates within a 32 KiB item
budget. Python/TypeScript context-read conveniences default to 100 items;
ordinary MCP `connect_context_view` defaults to 30 when its limit is omitted.
Explicit MCP limits remain 1–100. Omitted sources remain in
authorized history; automatic summaries, pinned-fact retrieval and perfect
ASR/model spelling are not claimed. A voice transcript can render a written
name phonetically; both sources retain their attribution. See the
[continuity evidence](evidence/gate-3/autonomous-continuity.json).

## Native task grants

For optional narrow delegation, use a signed Native invocation grant through
the same UCTP/QUIC task facade.
`ClientConfig::with_task_grant` replaces the parent token; the developer-only
`connect task-access export-native` produces a private language-helper/CLI config.
See [protected installation](TASK_ACCESS.md) for the actual command sequence.

Rust applications can sample metadata through
`ConnectClient::task_diagnostics(task_id, cursors, limit)`. The independent
`connect task-diagnostics` CLI uses the same public client. See
[task diagnostics](TASK_DIAGNOSTICS.md) for cursor lanes, optional denied sections,
read-only behavior and the distinction between journal observations and current
effect receipts. This convenience does not add a wire command or permission.

Negotiate one explicit task with Rust/Python `negotiate_task(task_id)` or
TypeScript `negotiateTask(taskId)`. Unlike directory capability discovery, this
requires no resolve permission. A matching task record is required before later
task commands. The JSON CLI uses `--task-id`; it retains `--task-address` for
capability discovery under a permitted backing credential.

A Native model grant admits task commands under its current stored binding;
communication uses `TaskCommand::Invoke`. Primitive control, administrator,
raw artifact HTTP and raw media signaling do not inherit parent authority.
Host/provider grants require their own admission. Revocation, expiry, cancellation
and backing-credential changes are checked even on already authenticated peers
and accepted-operation replay. Native media attachment and installed-host/provider
tool behavior have separate evidence and limits in the
[Gate 4 implementation](connect-gate-4-implementation.html).

The Python/TypeScript control helper returns correlated typed errors for both
startup and later requests. Invalid configuration returns `configuration`;
authentication rejection, including grant expiry on an existing peer, returns
`unauthorized`. The Rust client retains the verified UCTP `auth.session` expiry
and rejects expired sessions before dispatch. Expiry after dispatch does not
reclassify an unknown result as a rejection. A transport failure or deadline still returns
`transport-uncertain`: preserve the logical mutation ID and reconcile acceptance.
After fixing configuration or explicitly renewing a grant, close the helper and
create a new client. It never reloads credentials or falls back to an owner token.

Current owner-review qualification: Runtime 709cf82 passes 42 offline phases, 17 installed checks and 32 actual MCP calls with no errors. Qualification controls d64f1b9 combine the reviewed fixtures at 7bdd534 with all 3 hosted jobs. Local macOS arm64 owner review only; Gate 5, community publication and remote deployment remain separate.

[offline-709cf82](evidence/gate-4/offline-709cf82.json), [package-709cf82](evidence/gate-4/package-709cf82.json), [real-model-agent-709cf82](evidence/gate-4/real-model-agent-709cf82.json), [review-remediation-709cf82](evidence/gate-4/review-remediation-709cf82.json), [qualification-controls-d64f1b9](evidence/gate-4/qualification-controls-d64f1b9.json)
