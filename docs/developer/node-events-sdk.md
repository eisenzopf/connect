# Node durable journal readers

G6-062 implementation is complete in its [recorded scope](../evidence/gate-6/node-implementation.json). Full installed package/platform parity and remaining runtime failures are tracked in G6-069/G6-070/G6-096/G6-098. Earlier evidence below retains its original scope.

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
cached reply without fresh server authorization for the same journal permission
and cursor. Local capability support and an asynchronously updated health flag
are insufficient. A finite `conversation.subscribe` read (`live:false`, limit 1)
checks current authority without acknowledging, advancing or renewing the original
source. The public Rust iterator also checks before delivery. The
[retained-fact regression](../evidence/gate-6/language-decoded-media.json) failed
before this correction and passes under EdDSA/ES256 afterward. Its original bounded Rust job
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

The exactca3 Linux run receives the original Message fact on both Node22/26,
then fails the causal-link assertion because compact canonical
`operation.updated` omits `causal_event_ids`. The attempted full-record read
through `HttpClient.operationAction()` was also wrong: `DurableOperation` is
explicitly excluded from the application API. Exact38 reaches that call and
fails on both matrices. Its runtime error code was not captured.

The current consumer uses canonical `operation.get` for public record identity,
state and command, plus the exact original operation/Conversation/Message fact.
The independent Rust host reads the same authenticated owner's full Store record
and requires the exact logical ID, record ID, command, Conversation, completed
state and observed event digest in `causal_event_ids`. Native and HTTPS SQL
witnesses independently require one exact original sender/Conversation/Message/
logical-operation/completed-record link and compare its event digest. The
application client is used only for its allowed receipt extension. Restart,
cursor, current-authority, one-effect and cleanup checks retain their original
deadlines. No legacy route or new canonical field is exposed. Corrected
production execution remains pending; [canonical witness evidence](../evidence/gate-6/node-canonical-journal-witness.json)
preserves the source audit and failed attempts separately.
