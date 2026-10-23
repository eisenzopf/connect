# Canonical durable operation recovery (G6-037, in progress)

The schema-31 checkpoint below is extended by the current
[shared control receipts](control-receipts.md), which move ordinary mutations into
the ledger on fresh schema 32. Its original evidence remains historical.

The explicitly composed canonical service dispatches operation.get,
operation.cancel and operation.reconcile into the same durable Store ledger used
by current HTTP and public SDK actions. The [verification report](../evidence/gate-6/canonical-operations.json)
continues [Store-backed peer authority](store-peer-authority.md). Schema stays 31;
this continuation adds no table, conversion or compatibility dispatch.

The payload operation_id always names the target. A cancel or reconcile command
also supplies its own top-level operation_id for the separate logical decision.
Fresh signed retries, including on a replacement physical QUIC connection, return
the same authorized decision receipt. Changing the decision intent conflicts.
Responses are signed, correlated operation.updated envelopes with the durable
operation ID, state, revision and current authorized result. Optional Conversation,
Session and Connection assertions must match stored target metadata. A mismatched
get header ID is forbidden. Expected-revision assertions are explicitly refused
as unsupported for this installed recovery family; they are never discarded.

Peer requests now carry a closed command classification instead of independent
optional Conversation/scope fields. Operation admission checks the current exact
credential, Device and signing key, operation permission and original command
permission. It derives the actual operation owner/Conversation write scopes, plus
current member and frozen operation-source Identity readers. Membership receipt
projections include invitation targets, changed participants and shared source
owners. Administrative receipt projections retain their actual Identity source,
including records with no Conversation. Decision provenance is walked before its
recursive authorization check, with a 32-record depth/cycle bound and at most
1,024 distinct merged authority scopes. Stored typed results select these sources;
ignored wire actor/tenant fields cannot select authority.

Admission waits outside SQLite, rederives after the wait, and verifies coverage
in a consistent transaction. The same proof spans signature verification,
uncancellable Store work and signed response submission. The Store checks the
exact action, target assertions, credential and runtime ownership before acting;
source key/credential expiry is checked again before each commit. A wrong action,
read-only data proof, foreign root or changed credential cannot reuse that proof.
The existing metadata writer is reused without entering a fair writer a second
time behind a queued revocation. Get retains writer authority because it can
perform lazy worker recovery. Recovery of a possible submission preserves unknown
state, the original effect and its one attempt; it cannot reclaim or resubmit it.
Reconciliation still requires attributed evidence and existing effect settlement.

Local tests cover SQL-wait expiry with unchanged state, actual queued key
retirement, source projection custody, permission attenuation, proof confusion,
get/cancel/reconcile receipt replay and lazy uncertainty recovery. A malformed
persistent decision graph is explicitly injected only to verify bounded refusal;
the public API separately rejects mutations targeting another decision. The first
fixture incorrectly assumed nested decisions were valid and is recorded as a
failed candidate, not a lifecycle pass.

Actual loopback TLS/QUIC fixtures use the canonical driver, Store policy, real
Device enrollment and signatures. They compare get/cancel/reconcile with HTTP and
the current public SDK, retry on a new physical connection, reject changed intent,
unattributed evidence, wrong resource assertions and foreign owners, and preserve
unknown effect state across reconnect without another claim. Their Session and
connection rows are trusted control fixtures; they do not qualify physical media
or provider submission. Actual HTTP samples are checked against the operations
OpenAPI reference, including documented 403 and retryable 503 responses. Six new
HTTP samples cover the installed operation family. The fixture server
verification key is explicitly trusted; production signing-key distribution and
listener installation remain separate.

Installed policy refusals distinguish current permission denial (403), bounded
capacity (429), temporary authority contention (503 authorization) and Store
unavailability (503 storage). The source-lock records the upstream change as its
61st patch. An actual QUIC/HTTP test holds the Conversation reader, observes a
retryable refusal without a decision mutation, releases it and retries the same
logical decision ID successfully. An isolated missing-table fault separately
proves the signed storage refusal and recovery; no durable row is changed by that
refusal. Unknown signing keys remain forbidden, while SQL lookup failures retain
their storage classification. Other authentication-backend and cryptographic
resolver outage propagation still needs its own audit.

G6-037 remains open: production canonical listener/full command installation,
membership and transition admission routing, subscriptions and other approved
extensions, complete source readers/held task composition, physical per-write
admission across all drivers and HTTP response-body custody still need work.
Current ordinary message/task request receipts are not unified with the durable
transition operation table by this continuation; the general recovery boundary
must be completed rather than adding a compatibility fallback. Voice/SIP and
bootstrap/provisioning I/O custody, native media retirement, remaining expiry/bulk
profiles, G6-020 and dependent G6-038, full transport/SDK parity and all acceptance
remain open. No Linux/remote/browser/live-provider/publication claim is made.
