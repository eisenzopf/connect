# Durable transition operations

The [shared control receipts continuation](control-receipts.md) records current
schema-32 operation ownership. The [owned runtime continuation](owned-runtime.md)
records the earlier schema-31 startup boundary. Earlier evidence reports retain the schema/source state they
qualified; they are historical checkpoints.

The transition ledger owns admission, status, leases, reservations, effect intent,
attributed observations, decisions and retention. Admission does not prepare a
provider call or commit a live route. The consultation/handoff/replacement
coordinators and physical readiness checks are separate Gate 6 implementation
items; a stored `ready` stage alone is not evidence that media works.

`Operation::Transition { request }` (JSON `op: transition`) accepts a typed
`TransitionRequest`: stable caller `operation_id`, conversation/session, optional
source connection, expected session topology revision, absolute deadline and a
consult/handoff/replace intent. Consult participant sets are sorted before JCS
hashing. Explicit duplicates, invalid IDs and unsafe integer revisions fail.
Conversation-scope handoff also requires the expected conversation revision.
Replacement selects exactly one provisioned endpoint or candidate connection.
Endpoint ownership and verification, connection/session binding, membership epochs,
original session access and current credential recipient ceilings are checked.
Roles and supplied identifiers do not grant authority.

The authenticated stable identity and tenant own the ID, independent of the current
credential ID or transport envelope. Matching business intent returns the current
authorized record. Changed intent conflicts. The existing control receipt writer,
provisioning and membership writers cannot reuse an ID reserved in the other core
receipt family. Application task request IDs remain scoped to their task facade.
Actor/tenant/worker fields are never body authority. Older provisioning/membership
operation status uses their existing current-source replay policy.

Admission snapshots member epochs, selected connection generations and endpoint
revisions, and reserves conflicting session/connection/conversation resources in
one immediate transaction. Unknown/interrupted external effects keep reservations.
A new operation can replace a terminal reservation only with a higher fence.
Current authorization applies before replay, status and duplicate handling.

`op: durable_operation` wraps an `OperationAction`:

- `get`: the caller's original `operation_id`, requiring `operation.get` and the
  original command's current authority. It returns a bounded current record,
  effect metadata and authorized causal journal IDs. Authorized status catch-up
  records elapsed deadlines/leases without dispatching work.
- `cancel`: its own stable `operation_id`, `target_operation_id` and optional reason,
  requiring `operation.cancel` and current target authority. Unsubmitted work can
  cancel atomically. Submitted/confirmed effects or an uncertain commit produce
  `reconciliation_required`, keep the target reserved and fence the old worker.
- `reconcile`: its own stable ID, target, desired completed/cancelled/failed outcome
  and an existing attributed `evidence_id`. It requires `operation.reconcile` and
  current target authority. A body cannot create evidence. Local completion also
  requires a verified local-state result, current source epochs and matching local
  revisions. A late contradictory terminal outcome conflicts.

A cancel/reconcile response is a completed **decision receipt**. Inspect
`result.disposition` and `result.state_at_decision`; this does not claim that its
possibly submitted target was cancelled. Query the target for its latest state.
A post-commit cancellation never reverts the committed role/route decision.

Rust clients use `admit_transition` and `operation_action`; TypeScript uses
`admitTransition` and `operationAction`; Python uses the snake case methods.
The generic native and loopback HTTP operations share the service/store path.
[OpenAPI](../../contracts/openapi-operations.json) defines these current control
shapes. Authorized snapshots/subscriptions and canonical envelope dispatch remain
G6-036/G6-037; no compatibility alias or canonical wire qualification is implied.

## Protected worker and adapter composition

These are Rust backend capabilities, not public operation verbs or model tools.
They require explicit current scopes in addition to the original command scope:
`operation.execute`, `operation.observe`, and `operation.compensate` as appropriate.
Workers receive an unforgeable handle with a private nonce, generation, runtime
and resource fences; none can be selected by an HTTP/MCP body. Same-process handles
for one file share the runtime. A restored copy or a new process fences old work.

`claim_transition` atomically moves accepted or proven-safe interrupted work to
preparing. Leases last at most 60 seconds; renewal and every effect/commit recheck
current authority. A lost lease cannot retire or reactivate a connection. Safe
unsubmitted preparation can be claimed again. A possibly submitted effect cannot.
A deadline cannot be extended by changing a replay payload.

`prepare_transition_effect` persists a stable effect ID, provider correlation,
idempotency capability and optional compensation binding. The budget allows 32
original effects and at most 64 effects including compensation; filling the original
budget leaves room for cleanup of each creation. `begin_transition_effect` commits
one submission attempt before a provider call **outside** the transaction.
A duplicate never authorizes another call. Provider-key/status-query capability
is retained for attributed reconciliation, not blind resend after uncertainty.
Authenticated adapters call `observe_transition_effect` after verifying provider
origin and binding. Provider/event reuse with changed content conflicts. Callbacks
can settle an original attempt after worker loss; terminal facts cannot regress.

The local coordinator's SQL-only commit callback shares a transaction with final
completion. Revision, epoch, endpoint, credential-time and lease fences apply
before/after it. Local completion cannot silently admit a removed/rejoined member,
name unrelated connections in its completion, or report an uncommitted/ended
session as complete. The store-owned coordinator must restrict its SQL writes to
its admitted decision; the callback is not a sandbox for arbitrary application SQL.
External effects must be confirmed before completion. A verified graph probe can
also authorize the coordinator's protected `reconcile_transition_commit` callback:
its recovered local decision, terminal state and separate reconciliation receipt
commit atomically. A public receipt alone cannot assert completion while the local
role/topology state is still old. Replaying that reconciliation never calls the
commit callback again. Physical adapters still have
to establish their own current ingress/egress readiness before calling this hook.

Cancellation of a confirmed remote creation requires explicit cleanup. A fresh
cleanup-only lease can submit a separately recorded compensation after deadline
or cancellation; it cannot continue or commit the original transition. The original
creation remains confirmed and linked to its compensation. Verified settlement
then permits a separate cancellation/failure decision. Operator attestation alone
cannot manufacture remote termination or local commit.

If the original owner is revoked, a current backend can use
`inspect_transition_for_cleanup` / `settle_abandoned_transition` with explicit
`operation.cleanup`, current session-read access and current recipient ceilings.
It cannot complete the abandoned handoff or rewrite its owner. Verified probes
can establish absence or observed compensation; original creation facts remain.
Observed cleanup has attempt zero: it does not claim a new provider call occurred.
The repair has its own stable ID, current actor attribution and causal event.
These protected methods are not exposed by the HTTP operation enum or model tools.
Named real-provider cleanup still requires the designated G6-093 configuration.

The [operation authority integration](operation-authority.md) describes current
write fences, protected held-proof operation actions and bounded recovery batches.

## Recovery, capacity and retention

Opening state and bounded `recover_transition_work` fence stale runtimes/leases.
Recover/fence/purge batches process an eligible prefix with at most 64 merged
owner/Conversation write scopes. Drain callers continue on any positive count;
a partial batch does not mean the installation has no remaining work.
Possible submissions and commits become unknown or interrupted and remain reserved.
A same-process server restart explicitly fences all old leases too. A second read
handle alone does not interrupt the active worker. They never become a pending
callback to submit again. Stable IDs, digests, source
snapshots and evidence survive backup; physical network state is not restored.

Each active transition reserves room for a terminal decision and journal event,
sharing installation limits with tasks and ordinary receipts. Failure to persist
acceptance, observation, decision or completion rolls back the local transaction.
An atomic role decision cannot outlive a failed completion receipt.

At full journal capacity, fencing a worker token is still durable and independently
attributed. Normal startup fails if it cannot persist the recovery state event;
it never permits that old worker to continue. `Store::open_for_recovery` is an
explicit backend repair path. It fences worker tokens and admits no automatic
continuation; verified cleanup can consume the protected terminal slot. It is not
a schema migration, data reset or live-provider teardown command.

Explicit result-retention maintenance requires a closed conversation for at least
30 days, a terminal operation older than that window and settled external effects.
Full intent/results may then be removed; the original digest, ID and scope tombstone
remain. A matching expired ID returns `gone`; changed intent conflicts. It can
never execute again silently. Fresh state is schema 31; older candidates are refused
unchanged and require a verified backup plus explicit new-state provisioning.

Local evidence distinguishes control/storage fixtures from real media. The crash
harness exits actual child processes at 27 admission/claim/effect/ready/commit,
evidence, cancellation, reconciliation and cleanup boundaries and checks a separate durable fixture-provider counter and SQL role
state. Its ignored child entry is invoked explicitly by the parent harness; it is
not an unexecuted behavioral pass. Gate 6 acceptance and physical/remote profiles
remain unqualified until their full designated checks pass.
