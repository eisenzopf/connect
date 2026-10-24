# Native Participant selectors and server reconciliation

Public signed QUIC `stream.subscribe` accepts exact streams and Participant
selectors for negotiated audio, camera, screen and reliable data. A Participant
selector pins the current member identity and epoch. Core retains that receive
intent and resolves later admitted sources without relabeling it onto a
readmitted member. The receive intent is not publication or packet authority.

Omitting `kinds` selects all compatible receive kinds in the installed native
profile and the peer's last confirmed or explicitly declared capabilities. For
example, an audio/video receiver can select a Participant before that
Participant publishes a camera. Omission does not require screen or data support
from that receiver. An explicit `kinds` list remains exact and refuses an
unsupported kind. Current accepted Session capabilities and source authority
still gate the selector and each later publication; saved intent cannot grant a
new media capability.

An audio-only source can satisfy an unfiltered audio/video receiver through its
shared audio kind. Unsupported source kinds remain future intent; they do not
become authorized media. Explicit filters must be supported exactly. Core skips
only a typed unsupported-kind result for a wildcard. It never skips stale
membership, private audience or source/recipient authorization failure, and at
least one compatible authorized kind is required.

A successful routing update terminates with correlated `ack`. An unknown current
Stream or Participant terminates with correlated `error` code **404**. Disjoint
codec sets or incompatible explicit media kinds terminate with **488**. Both
refusals first report `operation.updated` with `state: failed`, remain queryable
through `operation.get`, and create no native effect. A mixed valid/unknown
selector list cannot partially change the routing table. Exact failed replay
returns the same durable result under current authorization, even after a later
successful routing revision.

`stream.unsubscribe` removes the listed logical Streams toward that Connection.
An explicit removal also excludes that Stream from a retained Participant
selector. Replaying the command returns its original outcome. A new command
removing an already removed Stream is a no-op and preserves the current stream
revision. Explicitly subscribing to that Stream again clears the exclusion.
The allocated receive binding can remain available, so this need not cause
another native parameter exchange. Other selected media continue to flow.

The production service installs `enable_native_subscriptions` after each real
native offer completes. The idle worker holds native origin metadata and a
policy, with no signed command, Store reader or old operator grant. Catalog
notifications are hints; each materialization derives a fresh original source,
current `stream.subscribe` and history/recipient authority, original Device/key,
verified endpoint, membership, capability and native generation.

The policy resolves the exact current Core plan before native IO. Its bounded
held grant owns target and source readers and validates the actual native
request scope, domain, operation, revision, selected media and routing delta.
Temporary contention looking up that invocation's native source remains
`authority-busy` and retries inside the original command deadline before the
engine call. A missing or poisoned source still refuses; this metadata retry
cannot repeat a routing effect or extend the command's lifetime.
Core catalog planning uses immediate probes of the scene member set and target
controller state, as well as each source state. Busy metadata returns temporary
backpressure rather than blocking a runtime worker behind a mixer tick. The
native reconciliation worker yields and retries; accepted host commands keep
their original deadline. Poisoned state and actual revision conflicts remain
refusals. No busy probe changes the route, selected intent or stream revision.
The [catalog probe evidence](../evidence/gate-6/auxiliary-native-catalog-probes.json)
separates controlled Core recovery from pending exact host/runtime verification.

Native target and source reads are captured together under one bounded Store
connection interval. The original attempt is validated before and after capture;
each source retains the same audience, scope and expiry checks as an individual
read. A refused source stays absent. Other capture errors return no partial
snapshot. This releases the SQL connection before Core work, acquires no new
authority interval and leaves each actual media callback's current checks intact.
Core callbacks and signed-reply verification reuse those readers without SQL
or grant reentry. A server reoffer must match the complete selected kind,
direction, codec and prepared handle set. The current original signing key must
confirm that exact setup and reversed peer directions. A changed setup, key,
credential or scope cannot borrow the reconciliation grant.

Core can choose any live Connection as a Participant's identity witness. That
witness's currently selected audio ports do not bound the Participant's receive
intent. Current accepted Session capability bounds the requested kind. Actual
future publication and every forwarded stream still require their own current
endpoint, profile, negotiated port and source/recipient grants. In particular,
receiving a screen intent does not grant a camera endpoint screen publication.

Schema 44 adds an internal `stream.reconcile` operation kind to the existing
native stream control ledger. This is not an accepted wire command. Only a
Store-issued fresh `NativeReconciliationRead` and a completed explicit
subscription control can begin it. The original parent native offer, credential,
key, Device, member epoch, resource ceiling and source expiry remain fixed.
Its exact plan descriptor and subscription record ID are durable before the
single native attempt. A per-parent open-control constraint prevents competing
attempts. An operation ID or stored intent cannot manufacture another lease.

The operation is visible through the existing current-authorized
`operation.get` and journal APIs. Its command type is `stream.reconcile`; a
completed result identifies `subscription_record_id`, `server_reconciliation`,
the Connection, stream revision and whether native resources changed. Status
requires current `stream.subscribe` and original membership/source visibility.
HTTP can inspect or cancel this work but cannot submit a reconciliation or
perform native negotiation. Native operation IDs remain unchanged across
uncertainty. Gate 6 schemas are pre-alpha and old schemas are refused.

Only an actual native return creates a captured committed receipt. A runtime
owned recording job retains that receipt and kernel attribution while storage
recovers. It retains no command, physical peer, credential grant or readers and
never invokes Core again. The upstream observer receives an observation-only
token during retry; this token is refused by request, reply and media control
authorization. Cancellation or expiry after a possible commit cannot invent
failure or cause resubmission. Uncertain errors and predecessor-runtime work
remain unknown. Historical completion never revives a retired Connection.

A later camera publication causes a signed server reoffer when new target
parameters are needed. Source EOF retires the original source and reconciles
routes. An already negotiated unused receive port may remain allocated, so a
source loss need not generate another reoffer or reset cumulative handle history.
Other authorized media continue during confirmation refusal and captured
recording failure.

The production local macOS arm64 signed QUIC cases cover Participant camera,
screen and JSON routing; later camera publication on a separate physical peer;
altered confirmation refusal; real VP8 delivery; source EOF with audio preserved;
and recording recovery while a public role writer proceeds. Controlled Store
cases separately cover current permission/generation refusal, foreign kernel
and peer-command rejection, single attempts, storage rollback, cancellation and
historical captured completion. Controlled readiness rows are not physical
evidence. The [general Session runtime evidence](../evidence/gate-6/general-session-runtime.json)
records the local G6-038 scope. Other media/profile cases and Gate 6 acceptance remain open;
these checks do not qualify Linux, browser, remote or provider lanes.

The server currently emits no optional `stream.active-speaker` hints. The wire
contract remains Session scoped (`cid` and `sid`); `connid` is optional. Its
required `active_participant` is a Participant identifier or explicit `null`
for silence/no dominant speaker. Missing this field is invalid. `strm_id` and
`changed_at` remain required. Clients must not infer role, mute state, routing,
authorization or Session completion from a hint or from its absence. Detection
and cadence are implementation policy; no detector or emission guarantee is
installed by this profile.

Subscription authorization now waits for the native connection's leaf binding
metadata lock. Its production holders perform only relationship comparisons,
clones or assignment and release it before calling Core, Store or IO. A busy
metadata read no longer becomes withdrawn authority and an uncertain operation.
The same identity, endpoint, Device, membership, generation and current control
checks still run before commit; poisoned metadata continues to refuse.

The actual signed initial-SIP subscription fixture holds this exact native
binding lock while admitting a command. The operation stays preparing until the
lock is released, then completes one effect and passes the existing marked
duplex audio thresholds. Both signing algorithms also run four ordinary setup
and replay rehearsals each: replay retains the original result, connection
revision and single effect. The pre-fix fixture reproduces an authority error
and `unknown` result from binding contention. This establishes that failure
class; the earlier spontaneous unknown outcomes lack enough tracing to attribute
every instance to it. Full selector, multiparty and hosting qualification remain
separate evidence obligations.
