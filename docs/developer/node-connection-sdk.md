# Typed Node Connection and Stream control

G6-062 remains in progress. `session.connections` now exposes scoped `byId` and
`offer` handles. Connection handles provide `refresh`, `update`, `end` and
`streams.byId/subscribe/unsubscribe`; Stream handles provide current `refresh`.
The helper delegates to the original public Rust SDK Client, which owns native
bindings across requests. Constructing or restoring metadata handles does not
create media ownership.

Offer inputs use nonempty `{first, rest}` streams and codec preferences. Updates
support mute, unmute, hold, resume, quality interval and codec renegotiation.
Every mutation requires a caller-preserved operation ID. The returned operation
completes through its original journal and supports scope-bound saved recovery.
Connection mutations preserve Conversation, Session and Connection IDs together.

`receiptStreamRevision` is an optional historical revision captured in the
original offer receipt. It is not a current observation. Current Connection
snapshots leave `streamRevision` absent when the Rust journal cannot supply it;
the SDK does not infer it from a resource revision. Updates and receive selection
require an explicit `SdkRevision<'stream'>`. A Stream snapshot's
`SdkRevision<'stream-resource'>` is a separate type and is rejected there.
Selection bindings retain `null` for unavailable and `[]` for known empty.
Clock units use `bigint`, preserving the full unsigned 64-bit value.

Stream refresh currently reports the Rust SDK's observation ownership. These
controls do not expose Node media frames; programmatic media remains G6-064.
Consultation, handoff, incoming invitations, events, trusted administration and
the complete Node contract remain unfinished.

The source checker runs 48 synthetic helper/HTTP functions and negative type
checks. The public package preparation script packs with lifecycle scripts
ignored, installs offline into a fresh private external project and compares
every installed distribution file to the current build:

```sh
python3 sdk/tests/check_canonical_helper.py --output .local/node-check
python3 sdk/tests/prepare_native_consumer.py \
  --output .local/node-install --package-output dist/node-install
```

The isolated Ubuntu workflow now runs the installed package against an actual
Rust/native server with separate ordinary caller and peer configurations, both
signing algorithms, HTTP record comparison, helper restart/current revocation
and original helper/server/native-owner drain assertions. Its new Connection
journey is pending execution. Local Rust compilation reached the Mac linker,
which failed with ENOSPC; no runtime pass is inferred from that attempt.
See [the evidence record](../evidence/gate-6/node-connection-sdk.json).

The original combined Session/native initial phase failed in three Ubuntu
cohorts. The source-bound `bb981bbc` cohort reports `native-offer` as its last
active checkpoint when the original 15-second phase expired. This identifies
the active block; it does not establish which sub-call was active or a product
cause. All 77 preceding native fixture functions passed in each cohort. Those
whole-phase failures remain failed evidence.

The next functional fixture profile is `node-controls/2`: separate Session
setup, native-control and Session-cleanup scenarios each retain an absolute
15-second deadline, followed by the existing recovery and revocation scenarios.
Every SDK request remains capped at four seconds, and the whole two-algorithm
function remains capped at 90 seconds. The explicit boundaries prevent prior
Session work from consuming the native scenario's observation window. All
original assertions, signing algorithms, callers, HTTP comparisons, current
revocation, helper joins and native-owner drain checks remain required. This
profile change is functional verification, not a performance qualification or
a claim to resolve the earlier startup deadline failures. Its actual run is
pending; no prior failed cohort is promoted to a pass.

The first `node-controls/2` run reached recovery after native control and first
helper shutdown, then failed a four-second read-only request in the recovery
block. The full function remains failed. The next source-bound probe separates
HTTP recovery, native end status/wait, Session end wait and Session refresh,
pins the original native end record and retains fixed command/state error
metadata plus completed-phase durations. This adds observations while keeping
the four-second request, 15-second absolute phase and 90-second whole-function
bounds unchanged. No product cause or runtime qualification is yet established.
