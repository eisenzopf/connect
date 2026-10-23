# Authority-changing transactions (G6-037, in progress)

The current schema remains 31. The [writer inventory](../../contracts/gate6-authority-writers.json)
records the exact entry paths in this slice and the outstanding audit. The
[local evidence report](../evidence/gate-6/authority-writers.json) covers these
paths alongside retained credential/fence and selected native regressions. This
is partial integration; the production canonical listener and current peer
policy remain uninstalled. The earlier
[fence foundation](authority-fences.md) checkpoint is historical source evidence.

Ordinary Conversation close, Session accept/reject/cancel/end, call/SIP end,
artifact deletion, receipt privacy and contact pairing changes now acquire
write scopes before entering SQLite. The actual operation engine verifies a
sufficient Store-owned proof again inside the transaction. A caller that reaches
the shared engine without that proof receives a refusal before business writes
or receipts. Existing principal, scope, recipient, epoch, replay and resource
checks remain authoritative.

Owner task execution and authenticated narrow invocation execution derive their
current Task Conversation and acquire its write set before the outer transaction.
Nested ordinary mutations carry that proof into the shared engine. Lazy task
expiry in Get, effect settlement and claim inspection also need write admission;
read-only inbox/wait/recall/replies do not acquire a new writer. Task defaults are
still derived inside the original transaction, after current admission, so
waiting never freezes a credential clock or grants a precomputed recipient scope.
Grant registration, issuance and revocation use the same Task write set.

Protected graph commit and local reconciliation callbacks wait for current
Conversation/member readers before their transaction. Current lease, immutable
source/reservation, evidence and completion checks still run inside and after the
callback; a fence does not bless arbitrary topology or remote observations.
Trusted routed Session admission/end and binding/unbinding also use their exact
current Conversation write sets. Unbind covers every Conversation referenced by
the connection even though the existing API returns one Session.

`execute_authority_mutation`, `execute_membership_with_authority` and
`execute_provisioning_with_authority` allow protected composition to retain an
already-owned sufficient write proof through the operation and delivery. They
check its namespace and full current scope set without acquiring again. A read
proof, missing Conversation/member scope, foreign Store proof or unknown current
credential is refused; a sufficient proof grants no principal or new operation.
These are trusted backend APIs, not payload-selected authority fields.
Task/transition dispatcher composition still needs its corresponding held-proof
integration before production policy installation.

The current HTTP control route maps authority-busy/unavailable to 503. An actual
receipt-privacy request held behind a delivery reader returns a bounded 503
without changing the logical snapshot, then succeeds with the same mutation ID
after release. Both exchanges are checked against
[the attachment/receipt HTTP reference](../../contracts/openapi-attachments.json).
The other current control references also document this response. No receipt or
new grant is fabricated for the busy refusal.

The cancellation fixture now observes actual writer admission, rather than a
scheduling delay, and releases its blocking worker on assertion failure. A caller
abort still leaves the authority owned by the uncancellable blocking operation
until it settles; revocation follows that settlement. The administration child
also replays the exact original expiry-bearing request, avoiding a fixture clock
boundary that could falsely turn an idempotent retry into a changed intent.

The [runtime continuation](runtime-authority.md) covers selected provider/media
callbacks, immutable route enable/disable and SMS policy writers. Remaining
work includes owned dispatch permits through actual network admission, runtime
ownership and recovery, operation-worker cancellation/generations, retention and
natural expiry, per-command reader scopes, response-body delivery, binding
retirement watchers and exact signed-key Device validation. Passing these local
fixtures does not qualify all writers, native media retirement, canonical
transport parity, another platform, a remote/provider/browser rehearsal or any
Gate 6 acceptance criterion.

The schema-34 [canonical subscription continuation](canonical-subscriptions.md)
adds exact snapshot/subscription query proofs and selected-source readers, including
departed source owners. Sealed live sources release their idle readers and retain
fresh per-page proofs through the common authenticated writer. A source-set race
refuses without advancing a cursor or upgrading an existing guard; owned admission
retries within its original wait and lease. The production canonical listener,
full HTTP response custody and physical media retirement remain open.

The schema-34 [canonical read report continuation](canonical-receipts.md) retains
original recipient and source-sender readers, including departed senders, through
local writer submission. It uses the ordinary receipt mutation and operation ledger.
Typed receipt hints retain provenance and precede cursor facts within the fixed
live deadline; preferences and recipient epochs are rechecked before projection.
