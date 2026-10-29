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

Exact `0b2f1769` Linux evidence now verifies all three ACK units and all ten
signed SDK/managed-helper event regressions on Node22/26, including deliberately
held initialization across several polls, exact retained replay/ACK, independent
control and close. The baseline HTTPS selector then stopped before installed Node
stages. Its corrected selection preserves both mandatory opt-in HTTPS cases;
installed event delivery/resume/current-authority qualification remains pending.

The installed message witness now follows the current producer contract. An
ordinary atomic message creates an authoritative `JournalData::Message` fact;
its completed Operation record lists that event in `causal_event_ids`. The
consumer checks the exact original operation, Conversation and Message IDs,
then compares the completed native/HTTP record and its causal event ID. The
independent SQL witness requires exactly one matching sender/Conversation/
Message/logical-operation row and compares its event-ID digest. It retains the
persisted cursor, original server restart, new message, current revocation and
all existing deadlines and joined cleanup.

Exact13d Node22 timed out at `events-original` because the previous fixture
waited for an `operation_updated` fact that ordinary message acceptance does not
emit. This expectation is corrected; no new Store/journal event or authority
behavior is invented. Eight isolated SQL selection vectors, Rust formatting and
both consumer syntax checks pass. Actual native/HTTPS correction remains pending
in the source CI lane; see [the witness evidence](../evidence/gate-6/node-message-journal-witness.json).

The exactca3 Linux run now receives the original Message fact on both Node22/26.
It then fails the new causal-link assertion: canonical `operation.updated` is a
compact projection and deliberately omits `causal_event_ids`. The consumer now
reads the full record through the existing public
`HttpClient.operationAction({action: 'get', operation_id})`, after observing the
fact. It requires the same original logical ID, record ID, command, Conversation,
completed state and exact causal event ID. Both native and HTTPS journeys keep
their canonical reads, journal checks, current-authority checks and deadlines.
No wire/schema extension or mutation behavior changed. Corrected production
execution remains pending; [record-witness evidence](../evidence/gate-6/node-operation-record-witness.json)
preserves the failed source-bound result separately.
