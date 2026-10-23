# Atomic native media subscriptions

Public `stream.subscribe` and `stream.unsubscribe` operate on the receiver's
expected stream revision and logical operation ID. A subscription to a Participant
means all current and future streams of the requested kinds. An omitted kind list
means all media. That requirement remains part of G6-020.

The current implementation resolves exact stream IDs against the admitted managed
scene. It considers actual publishing bindings, rejects an unknown or ambiguous
ID, and binds direct video/screen/data delivery to that exact publisher and a
receiver logical ID equal to the selected source ID. It never chooses an arbitrary
generic video output. Audio uses the managed MCU output. `by_participant` asserts
the authenticated receiver's owner; it does not grant authority.

`CanonicalNativeAdapter.plan_native_subscriptions` checks the real authenticated
physical route, credential, configured wire scopes, confirmed media revision and
current managed scene. It combines the selector result with the actual retained
peer capability catalog and server profile. Existing parameters are preserved;
new direct receivers must support the source's exact encoding. The host's current
held policy independently approves the entire operation and source audience.

When receiver resources change, the actual signed subscribe invocation drives
one native parameter offer through the existing physical actor. No synthetic
`AuthorizedEnvelope` is constructed. The endpoint must return a signed, exactly
scoped confirmation of the actual provisional handles and complete parameter
batch. Core commits receiver resources and both audio/direct routing programs
under one revision. A failed confirmation or local preparation preserves the old
bindings and selections; failed handles remain burned.

A route-only change uses the same negotiation slot and a single routing
transaction. `MediaControlIntent::Subscriptions` identifies a combined routing
change. `ChangeSubscriptions` includes the complete media request and both
program changes when resources are prepared. A policy must approve the exact
intent, receiver, domain, actor and revision. Existing single-program APIs share
the transaction while retaining their explicit policy intents.

The engine validates the complete request before preparation, stages route
tickets without publishing them, drains affected sends, then revalidates current
source membership, owner, audience, codec and direction. The scene, receiver state
and both program locks cover the one commit. Quiescence guards remain held until
all swaps finish. Cancellation or failure before commit preserves both old
programs. Replaced selection generations reject previously queued media.
Repeating an unchanged selection or removal is idempotent. Removing a selection
retains confirmed receiver resources for reuse and does not refund opening budgets.

Current held source/audience checks borrow the actual invocation guard, including
during endpoint confirmation and commit. They do not reacquire storage behind a
queued writer. Delivery independently checks current policy on every admission
and write; retained selection metadata never supplies a future grant.

Participant selectors retain the original Identity, principal owner and membership
epoch. This third program commits atomically with audio/direct routes, including
when no matching source port exists yet. It contains no authorization lease.
An unsubscribe creates a bounded logical-stream exclusion, so source changes or
background reconciliation cannot silently resubscribe it. A new explicit selection
can clear the corresponding exclusions. A readmitted membership requires a new
explicit selection.

Hosts enable non-audio future selection with `enable_native_subscriptions` after
actual signed negotiation. A scene wake hint causes a fresh catalog read, shared
negotiation-slot reservation and `NativeSubscriptionPolicy::reconcile` call. The
host supplies current authority and the logical operation ID from its journal.
The sealed server invocation carries that real authority through the actual signed
parameter exchange and checked Core commit. It never constructs an authorized
client command from stored envelope fields. A route without this policy refuses
non-audio Participant selection before acknowledging a future promise.

`NativeSubscriptionPolicy::observe` records the exact local or uncertain result
using the same held authority. A failed observer after local commit retries the
original committed plan and operation ID under fresh authority, without repeating
media IO. Only metadata remains while idle. Policy changes are wake hints; a missed hint cannot make a transient denial
permanent. Pending authorization retries on a bounded interval without retaining
readers. Current authorization is checked again before effects and delivery.

An inactive hold pauses both directions in one drained gate transaction. Resume
restores negotiated directions while preserving an independent source mute;
directional hold explicitly replaces a prior full pause. These controls preserve
transport IDs, handles and cumulative resource budgets. An inactive stream offer
with no usable selected media remains an explicit unsupported-profile refusal.

Selector handlers distinguish unknown sources (404), incompatible capabilities
(488), revision conflict (409), invalid selectors (400) and denied scope (403).
An uninstalled future-selection policy is a wiring gap (501), so it cannot silently
acknowledge only a subset of the requested media.

Only a passing source-bound `atomic-subscriptions.json` regression report
qualifies this scope before plan status changes. Production Connect Store/Session/journal installation and the full
application workflow retain their own task scopes. Complete virtual connection
termination/presence is not inferred from ending a publishing stream. Local graph
acknowledgments are not durable operation receipts or Participant presence.

Behavior checks cover mixed audio/video/screen/data engine delivery, whole-request
refusal, one-revision resource/routing commit, idempotence, queued-media fences,
cancelled drains, current audience denial/recovery and held authority under a
queued writer. Native wire checks use actual managed admission and signed TLS
peers. The current wire fixture's endpoints belong to one Participant, whose audio
echo the MCU intentionally suppresses; its video behavior does not establish
cross-Participant native audio playback. That audio behavior needs its own wire
fixture in addition to the existing distinct-owner engine checks.

Full Gate 6 retains the approved Linux x86_64 and remote/native/browser lanes.
Named external rehearsals require bounded G6-093 configuration and designated
endpoints. Publication remains separate.
