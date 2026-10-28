# Node durable journal readers

`conversation.events({after}, {timeoutMs, signal})` captures its input and returns
an owned async iterator. `after` is an opaque `SdkCursor`; use `sdkCursor(saved)`
when restoring a persisted value. A cursor never grants access. Each `next()` has
the captured finite timeout. Persist `fact.cursor` only after processing that fact.
`ready()` completes the authorized initial snapshot/subscription and returns its
baseline cursor without consuming a fact. This supports opening a reader before
producing an event; it does not expose snapshot data as journal facts.

```ts
const stream = room.events({after: saved ? sdkCursor(saved) : undefined}, {timeoutMs: 4000});
try {
  for await (const fact of stream) {
    await applyIdempotently(fact.eventId, fact.data);
    await persistCursor(fact.cursor);
  }
} finally {
  await stream.close();
}
```

The existing Rust SDK owns bootstrap, subscriptions, renewal, duplicate checks,
current authority and native journal catch-up. The helper retains one SDK pending
fact until Node acknowledges its exact event ID and cursor on a later poll. A
local abandoned wait cannot advance the cursor, acknowledge a fact, or return a
cached reply without another Rust health check. Its original bounded Rust job
survives the helper's short poll intervals, so slow initialization/catch-up is not
repeatedly cancelled. Eight readers, 256 facts and 2 MiB per SDK source remain the
original bounds. Replies retain the existing 2 MiB framing limit; ordinary outgoing
requests keep their existing smaller limit.

Facts preserve their original scoped identities, audience epochs, source,
revision, observation time and recursive immutable data. Diagnostic inspection
redacts cursor/source/data; intentional JSON export retains them. Local close and
for-await cleanup detach the reader while preserving unrelated control. A valid
SDK source protocol/resync error retires only that source; malformed helper replies
still fence the helper. No retry of mutations or replacement helper is performed.
Participant helper profile 4 is required; administrative profiles are unchanged.

Verification is currently 109 synthetic component functions and negative
TypeScript declarations, plus 11 event functions through the packed, independently
installed distribution. Rust ACK units and the original nine signed journal tests
are included in the Linux lane. Their execution and real-server event qualification
remain pending. This does not close G6-062 or qualify remote HTTP, native media
ownership, acceptance or publication.

The next Linux candidate (`node-controls/7`) extends the original installed
native control journey with initial event delivery, original record and independent
SQL event identity, private cursor persistence, explicit resume in a fresh helper
after the existing host restart, another synthetic message with native/HTTP record
equality, and no-data observation after current credential revocation. No budgets,
helper counts, drain assertions, or audio thresholds are relaxed. The candidate
also adds one signed helper-specific slow-ACK/exact-delivery test alongside the
original nine SDK journal regressions. Execution remains pending.

The capacity simulation now matches the original SDK: a second live reader for
one Conversation conflicts; the eight-reader budget uses distinct Conversations.
The earlier synthetic same-Conversation capacity scene is superseded. All109
component functions and11 installed event functions pass with this corrected
fixture. Native source behavior still requires its signed/Linux evidence.
