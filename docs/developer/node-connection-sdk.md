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
Typed [consultation controls](node-consultation-sdk.md) are also available.
Handoff, device movement, incoming invitations, events, trusted administration
and the complete Node contract remain unfinished.

The recorded `1ac57f40` Ubuntu24.04 x86_64 / Node26.12.1 cohort passes the
installed control function for EdDSA and ES256 in 53.47 seconds. All ten scenario
phases pass, all six original helpers and both servers join, and physical
peers/media readers drain. Native/HTTP original-end records agree after helper
restart; both end waits and current Session refresh pass, followed by current
revocation. All 78 selected native functions pass with 8068 source inputs
unchanged, including 3976 tracked files independently checked against the
commit. Installed distribution and consumer bytes match local preparation.

The source checker runs 53 synthetic helper/HTTP functions and negative type
checks. Downloaded Node22.23.4 and Node26.12.1 artifacts pass those checks with
source maps identical to local preparation. An actual native Node22 cohort has
not run. The package preparation script packs with lifecycle scripts ignored,
installs offline into a fresh private external project and compares every
installed distribution file to the current build:

```sh
python3 sdk/tests/check_canonical_helper.py --output .local/node-check
python3 sdk/tests/prepare_native_consumer.py \
  --output .local/node-install --package-output dist/node-install
```

The functional fixture profile is `node-controls/2`: separate Session setup,
native control and Session cleanup scenarios each retain an absolute 15-second
deadline, followed by recovery and revocation scenarios. Every SDK request is
capped at four seconds, and the whole two-algorithm function is capped at 90
seconds. Both algorithms, separate ordinary callers, HTTP comparisons, current
authority, original helper joins and native-owner drains remain required. This
is functional control evidence; it does not qualify decoded media, performance,
named remote/provider/model use, full installed packaging, the full SDK or
acceptance.

Three earlier combined Session/native initial phases failed at their original
15-second bound; `bb981bbc` identified `native-offer` as the active block. The
first phased run (`cbdf1959`) reached recovery, then failed a four-second
read-only request. The next probe separated native end HTTP/status/wait and
Session end wait/refresh with original-record pins and redacted phase/error
metadata. It passed with all deadlines unchanged. The production CLI is
byte-identical across that failure and the later pass, so the recovery failure's
cause and a production fix remain unproven. Those failures and earlier startup
deadline failures remain retained. Local Mac native compilation also failed at
the linker with ENOSPC; no runtime pass is inferred from that attempt.

See [the evidence record](../evidence/gate-6/node-connection-sdk.json) for exact
source, package, artifact and result hashes and unresolved scopes. G6-062 remains
in progress.
