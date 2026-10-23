# Canonical client read reports and receipt hints (G6-037, in progress)

`message.read` now enters the ordinary `message_receipt` engine through an exact
Store peer proof. Its `operation_id` is the caller's stable logical mutation ID;
its envelope ID is fresh on each exchange. A Conversation and message are required.
`by_participant`, when present, must match the authenticated original recipient's
current appearance. Session, Connection, reply and revision assertions are refused.
A sender, role label, administrator or later history share cannot report for the
original recipient. Leaving and readmission cannot restore report authority or
access to an old report's operation receipt.

The signed invocation proof binds the exact message, logical ID, asserted appearance
and diagnostic `read_at`. The effect is the authenticated recipient's read report.
Like other protocol timestamps, `read_at` is diagnostic; it does not establish
human action, ordering or authority. The shared engine records server observation
time and `client_reported` provenance. Typed receipt hints use that same server
observation time, with explicit evidence metadata. Changing diagnostic time in a
fresh signed exchange does not change the logical effect or append another fact.
A proof from one exchange cannot be reused for a different diagnostic assertion.

Native, HTTP and canonical reports with the same stable ID share one retained
operation result. The canonical reply is `operation.updated` followed by an ACK
with message, actual participant/epoch and `recorded`. Native and HTTP clients
receive the ordinary `ReceiptAcknowledgement`. Current original recipient authority
is checked before replay. SQL failure leaves neither receipt facts nor a completed
ledger entry; the same ID can recover. A source expiring behind a SQL wait cannot
commit. Current readers for the original recipient and source sender, including a
departed sender, remain held through local submission. They are reused without
reader reentry when an authority writer is queued.

Read reporting still defaults off. With delivery enabled and read disabled, a read
report can record `delivered`. With both disabled it records neither. The optional
read fact is never backfilled by a preference change. Current preferences filter
retained status and journal facts before paging. A provider cannot create native
read evidence. An inbound `message.delivered` cannot select a delivery channel or
forge a server delivery observation; it remains an unsupported command.

The explicit [physical subscription composition](canonical-subscriptions.md)
projects eligible journal receipt facts as typed `message.delivered` or
`message.read` hints, followed by their `conversation.event`. Only an original
subscription principal with the data protocol floor receives the typed hints.
Read hints require native `client_reported` evidence. Delivery hints require
attributed native reports or verified provider observations/state; queued,
accepted, unknown and failed facts never become delivered hints. `payload.evidence`
contains the journal event ID, channel, provenance and server observation time.
These remain advisory reports, not proof of human viewing.

A typed hint precedes its cursor-bearing journal fact. Persist the cursor only
after processing the associated hints and fact. On an interrupted page, replay
may repeat an event ID. Canonical requests accept a limit up to 127 as an upper
bound; the effective journal page ceiling is 63 facts, subject to the installation's
lower ceiling. Each fact can produce a hint and event, plus the final ACK, within
128 application envelopes and 2 MiB. Native/HTTP event page ceilings are unchanged.
The fixed live deadline applies to both receipt kinds even when a backend supplies
a longer authority lease. Neither receipt kind can use terminal snapshot grace.

The source patch is `patches/rvoip/gate6-receipt-observations.patch`, recorded as
source-lock patch 63. No database migration is needed: the ordinary receipt model,
server times, operation ledger and retained journal are unchanged. The selected
verification runner is `scripts/check-gate6-canonical-receipts.py`. Cases include
real signed local QUIC with native/HTTP replay and credential rotation, preference
suppression, spoofed recipient/server delivery refusal, 64 receipt facts crossing
the frame budget, source expiry, atomic SQL failure/recovery, history readmission
and actual held-reader release at the writer deadline.

Production canonical signed native SDK/listener installation, remaining command
families, complete per-write HTTP body custody, media/provider retirement and full
Gate 6 acceptance remain open. This continuation qualifies selected local macOS
arm64 fixtures, not remote/browser/live-provider operations or publication.
