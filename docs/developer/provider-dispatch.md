# Owned provider message dispatch (G6-037, in progress)

Current status custody continues in [provider reconciliation](provider-reconciliation.md).

Fresh state uses schema 31. The [local verification report](../evidence/gate-6/provider-dispatch.json)
extends the [owned runtime checkpoint](owned-runtime.md). Earlier candidate
databases are preserved and refused. Retain a verified backup and matching binary,
then explicitly initialize a fresh path. This change adds no conversion or downgrade.

Exact, current grant issuance replay is read-only and returns the original secret
and expiry after checking owner/task/origin/intent. Changed or revoked grants deny
without fallback; new issuance still requires the write fence. This lets a
provider MCP session authenticate while delivery retains its read guards.

A protected worker owns a unique runtime role before registration or recovery.
A duplicate worker cannot mutate recovery state. Retired roles refuse new I/O;
a pending transport keeps the role and installation kernel ownership until the
actual socket is dropped. Worker setup and store admission run in spawn_blocking.

Shared route chat history admits one pending turn at a time. Independent task
turns keep their separate context and can complete in either order.

Every queued message stores an immutable source file identity. A claim mints one
runtime-bound private token and freezes the prior chat ID. Last-mile admission
rechecks the exact stored route, body, original sender and delegate credentials,
audiences, task effect, connector operation and context. Conversation context is
re-evaluated at its original watermark: later appends do not change that selection,
while changed selected resources deny admission. These checks happen again in the
immediate transaction after bounded resource-lock acquisition outside SQLite.

The returned DispatchPermit retains Conversation/all current member Identity read
guards, the worker role, and the earlier current credential/task/SMS expiry or
35-second attempt deadline. It contains no wire authority. Clones share one
atomic effect admission. The connector API requires the actual permit at each
message forwarding/leaf boundary; fixtures have an explicitly named conformance
permit, which production workers never mint.

Telnyx/Vapi message creation preserves SDK payload and reply types, using a
Connect-owned dedicated HTTP/1 connection with WebPKI TLS and a guarded actual
socket. There are no retries, redirects or pooled connections. Each socket write,
vectored write and flush checks nonblocking current role/expiry/cancellation before
polling. Caller cancellation signals the connection driver; the driver owns its
socket and guards through actual cleanup. Plain HTTP is restricted to loopback
configuration and a loopback peer. Bodies are bounded to 1 MiB. A read-only Vapi
assistant lookup can precede the one chat creation under the same permit.

Recovery drains deterministic prefixes of at most 1,000 jobs and 64 Conversation
write scopes. It rederives after waiting and checks the exact original eligible IDs
inside the transaction before changes. Same-file unadmitted claims are cancelled;
admitted attempts are uncertain. Restored pending snapshots are also uncertain:
the original may have submitted after the backup. They are never requeued for a
new effect. A 1,030-Conversation restore fixture verifies bounded draining and
held-reader refusal without partial writes.

The actual loopback worker test holds a provider response after receiving the
request body. An HTTP identity revoke returns 503 without changing state, duplicate
worker setup refuses without recovery writes, shutdown closes the real pending
connection, and the same logical revoke succeeds after cleanup. Restart retains
uncertainty and does not send again. TCP and pinned-TLS tests separately cover
per-write retirement, deadline expiry, cancellation custody and single-use clones.
Adapter contribution conformance rejects retired permits and a second creation
using a clone. These are isolated fixtures, with no live provider accounts.

G6-037 remains in progress. This checkpoint does not qualify provider status or
reconciliation I/O, voice/SIP transport ownership, every natural-expiry path,
post-retirement attributed settlement, all current reader/task/transition scopes,
signed-key Device policy, actual HTTP/signed response-body delivery, canonical
listener/full commands or transport/SDK parity. G6-020 and G6-038 remain open.
Linux, remote/browser/live-provider checks, publication and all acceptance remain
unqualified.
