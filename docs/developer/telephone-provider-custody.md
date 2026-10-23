# Telephone provider dispatch custody

One telephone worker owns recovery and dispatch in each installation runtime.
An overlapping worker is refused before manifest registration or recovery. The
lease spans discovery/control tasks and admitted blocking mutations; a replacement
worker starts only after the previous owner drains.

The protected telephone worker uses `Store::begin_voice_dispatch` to obtain a
nonserializable `VoiceDispatch`. A claim is scheduling metadata. The dispatch
permit owns the current transport interval and the exact claimed call, route,
remote binding and end-intent generation. It cannot be cloned or constructed
from provider headers, a public principal or a restored database row.

Durable once-only admission commits before the worker acquires its transport
read. The read repeats current original credential, audience, conversation,
route, task and invocation-grant checks. Withdrawal between those two phases
settles a known unsent request without issuing a permit. Infrastructure failure
preserves the durable admission; it cannot authorize redial on recovery.

Start and end requests retain their original current Conversation/member
interval while the bounded driver future is pending. The worker clips its
35-second request budget to the original credential, task and narrow grant
expiry and checks runtime retirement. End uses its exact current end-intent
issuer and generation; it does not borrow the original dialer's credential.
Disabling an outbound start route does not erase authority to end an already
owned remote effect. Polling observes that exact existing remote binding under
host custody; it cannot initiate another call under an invalidated task.

`VoiceDispatch::capture` consumes the transport permit and releases its authority
before a result writer can wait on revocation. The resulting
`CapturedVoiceResult` binds the original installation manager, runtime, logical
mutation and physical response. Capturing grants no further network IO. Invalid
or mismatched decoded outcomes after possible submission remain uncertain.
A captured authenticated outcome may settle after principal withdrawal. Its
projection, call state and journal commit in one transaction; retries preserve
the same event and mutation IDs and cannot replace an existing remote binding.

Captured-result recording uses immediate authority and SQL admission. Contention
returns without retaining a writer queue or SQL mutex. The worker retries only
the same captured metadata for a bounded four-second window, never the transport
request. An unconfirmed recording stops control explicitly for reconciliation.
This in-memory custody does not claim crash-recoverable storage of an uncommitted
response. Crashed or interrupted starts remain uncertain and are never redialed.
Runtime ownership remains retained through the physical request and captured
settlement; retiring an owner prevents further transport use and a replacement
owner cannot take over while original custody is retained.

These are protected adapter APIs. They do not restore the retired public
`call_start`/`call_end` dialect. G6-044 still requires canonical generic
Session/operation/Endpoint integration for telephone and messaging and migration
of the five historical public telephone behavior cases. Backend worker tests
are not substitute evidence for those public workflows or named live-provider
rehearsals. Named provider proofs require G6-093's designated endpoints and
concrete bounded configuration.

Local component verification and explicit remaining scope are recorded in
[`provider-voice-custody.json`](../evidence/gate-6/provider-voice-custody.json).
