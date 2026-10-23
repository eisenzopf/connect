# Canonical conversation subscriptions (G6-037, in progress)

The schema-34 continuation implements `conversation.subscribe` through an explicit
`CanonicalHandler::physical_factory(service)`. Each accepted physical peer owns
its handler, cancellation lifetime and bounded subscription registry. The server's
production native listener still uses its previous handler; this fixture-qualified
composition does not install the canonical listener or a signed native SDK.
There is no database migration in this continuation.

The initial page, snapshot watermark, cursor, audience selection and retained
journal facts come from the same Store APIs as native, HTTP and SDK event reads.
`live: false` returns one finite page and its ACK. Live mode defaults to true and
requires physical composition. The current
[receipt continuation](canonical-receipts.md) caps canonical pages at 63 facts,
each with an optional typed receipt hint, its event and the final ACK. A requested
limit up to 127 remains an upper bound; the installation's lower ceiling applies.
The common writer also enforces 128 envelopes and 2 MiB of encoded application replies. Each peer
has at most eight live Conversations. Renewing the same Conversation replaces
only its previous subscription; a ninth Conversation receives a capacity error.
Leases are 1–30 seconds, capped by the original credential, signing key, task and
physical dispatch lifetime. A continuation never slides its original lease.

A live journal fact becomes `conversation.event`, with its complete typed
`JournalEvent` in `payload.data`. Its envelope retains the original subscribe
request's `in_reply_to`, the exact Conversation and any authorized Session,
Connection and operation correlation. An event's operation ID belongs to its
original authorized mutation, not to the subscribe read. The final ACK carries
`cursor`, `has_more`, `resync_required`, `lease`, `expires_at` and `live`. Resume
from a cursor only after processing the corresponding facts; repeated event IDs
are idempotent. The same opaque cursor resumes through HTTP or a new physical
peer subject to current audience and membership checks.

`PeerSubscriptionSource` is an opaque, non-serializable continuation of the
original installation, credential chain, Device/key, actor membership epoch and
scope ceiling. It retains no authority readers while waiting. Every bounded page
acquires a fresh owned Store proof, checks those original facts and the current
selected sources inside SQL, and retains that proof through actual local writer
submission. Current members and referenced source owners are reader-fenced,
including a departed owner's retained message, attachment or reply reference.
Revoking a sender prevents future actions; it does not retract immutable content
from another recipient's still-current original audience epoch.

A journal append can introduce another source between planning and the read
transaction. The old proof refuses that page with `authority-busy`. Owned reads
and sealed continuations release it and retry the same cursor within the original
bounded authority wait. They never upgrade the held reader, advance a refused
cursor, or change an authorization refusal into a retry. Calls composed with a
caller-held proof return the refusal for their owner to handle.

The per-peer actor registers wake hints before reading and keeps one pending
bounded batch at a time. An empty page can advance only its authorized internal
cursor. It releases all SQL and authority guards before waiting on journal hints,
a one-second fallback or cancellation. A retained initial authority must first
leave the common writer before the live source becomes ready. The writer owns
both initial replies and later deliveries; a backend cannot use a raw sink or
borrow a successor authentication incarnation. A completion callback means local
submission succeeded, not remote receipt or human delivery. The next page is read
only after that callback. Failure, EOF or cancellation releases its proof.

Refresh retires the old physical authentication incarnation. Removal/readmission
cannot revive an old source even when the member can read fresh history. A foreign
physical origin is never submitted. Event submission ends at the fixed live
boundary, including a blocked sink. During at most one additional second, and
only while the original credential and current read authority remain valid, the
source can send a fresh authorized snapshot plus a terminal resync ACK. This grace
cannot carry late journal events. Retention loss likewise returns explicit resync.
If authority has retired, no terminal page is fabricated under another grant.

The original subscription checkpoint used
`scripts/check-gate6-canonical-subscriptions.py`. Current receipt-aware delivery
is qualified by `scripts/check-gate6-canonical-receipts.py`; the original report
remains historical after the receipt continuation. Its report must bind unchanged
sources, prepared patch digests, compiled test artifacts and raw log hashes.
Behavior cases include actual signed local QUIC snapshot/live/catchup/lease end,
refresh, credential revocation without idle guards, member readmission, capacity,
replacement and HTTP cursor parity. Upstream common-writer cases separately cover
foreign origins, initial readiness and blocked local submission with a real held
authority reader. Store cases cover exact query/source binding and source growth
without reentry. These are local macOS arm64 checks, not production native
installation, complete HTTP body custody, media retirement, remote/browser/provider
profiles, independent conformance, publication or Gate 6 acceptance.
