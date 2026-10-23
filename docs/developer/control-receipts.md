# Shared control mutation receipts (G6-037, in progress)

This schema-32 checkpoint is historical. The [schema-33 conversation continuation](conversation-control.md) adds current conversation projections and create/list/close. Its report below is retained unchanged.

Fresh schema 32 removes the ordinary `requests` table. Ordinary control mutations
now write the same `operations` ownership key used by canonical operation recovery,
provisioning, membership and transition decisions. `operation_control_receipts`
retains only typed origin and authorization metadata; the operation row owns the
single intent, digest, state and result. No receipt is copied or imported on read.
Opening a schema-31 candidate still requires an explicit backup and fresh state.
The [verification report](../evidence/gate-6/control-receipts.json) continues the
[canonical operation checkpoint](canonical-operations.md).

The ordinary writer normalizes the typed command before hashing, excludes its
logical request ID from business intent, and uses a 64-character digest in the
shared ledger. Recipient sets and creation participant sets are normalized.
Artifact upload links are excluded from intent; a completed pairing proof is
retained as a digest instead of another plaintext challenge. The local identity
creation helper now uses the existing provisioning writer and its receipt.

The business mutation, journal, result and typed receipt origin commit in one
transaction. Infrastructure failure rolls everything back and reserves no ID.
An authorized business rejection retains a failed operation without a completed
result; replay cannot turn it into a new successful intent. Matching replay
returns the original result and changed intent conflicts. Receipt and intent
metadata are immutable. Full control receipts remain retained within the global
receipt capacity; exhaustion refuses admission rather than evicting old IDs.

A message acceptance carries a `receipt`: operation ID, Conversation, completed
local-mutation state and an exact `operation.get` status reference. This proves
local journal acceptance. Recipient receipts and provider delivery remain their
own observations; it does not prove remote delivery or successful provider I/O.
The Rust, Python and TypeScript send helpers require a matching reference and
reject missing, mismatched or fabricated provider-completion receipts without
retrying. Task effect results deliberately carry no core status reference.

Current credentials, original command permission, frozen recipient ceilings and
the actor's original membership epoch apply before status or replay. Leaving and
rejoining cannot restore the old operation result. Optional signed CID/SID/Conn
assertions must match the stored record. Canonical operation admission includes
the actual metadata writers and frozen recipient/source Identity readers, then
retains its original proof through SQL and signed response submission. The Store
uses the same current authority checks for HTTP, public SDK and request status.
Owner journal events carry their original operation ID; recovery finds their
causal IDs without rewriting immutable event facts.

Task invocation receipts persist their task/grant origin alongside the private
namespaced wire ID. Generic operation.get and request_status refuse those rows,
even for an account operator. Task effect inspection and replay continue through
the current task/grant checks. Their separate public task request IDs remain in
the task namespace.

Actual signed TLS/QUIC evidence sends a message without consuming its ack,
observes durable commitment via HTTP, closes the physical connection and rotates
the credential. A new signed peer, native SDK and HTTP recover the same record;
replay preserves one message and one journal fanout, and changed intent or extra
resource assertions are refused. A separate current-facade scenario exercises
native SDK, HTTP, CLI, MCP, Python and TypeScript with one binary message, including
querying its receipt reference. Local failure tests cover receipt-write rollback,
retained rejection (including actual canonical status), permission attenuation,
credential retirement, restored receipts and task privacy. The actual HTTP
operation facade also classifies durable-storage outages as retryable 503,
matching the signed policy refusal.

G6-037 remains in progress. Production canonical listener installation, remaining
command/extension/subscription routing, complete reader and held-task composition,
HTTP response-body and physical per-write custody still need work. Ordinary
control receipts record local mutations; complete provider/voice/SIP lifecycle
projection into generic operation effects is not qualified here. Task-owned core
receipts remain private; this is not complete task-extension transport parity.
G6-020 media/stream negotiation, dependent G6-038, full SDK/transport parity and all
Gate 6 acceptance remain open. This report makes no remote, Linux, physical browser,
live-provider or publication claim.
